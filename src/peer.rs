use std::{net::SocketAddr, time::Duration};

use anyhow::{Result, anyhow, bail};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};

use crate::{
    bitfield::BitField,
    piece::{ActivePiece, BLOCK_SIZE},
};

const HANDSHAKE_SIZE: usize = 68;

#[derive(Debug, Clone)]
pub struct Peer {
    addr: SocketAddr,
    peer_id: [u8; 20],
    bitfield: BitField,
    am_choking: bool,
    am_interested: bool,
    peer_choking: bool,
    peer_interested: bool,
}

impl Peer {
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            peer_id: [0u8; 20],
            bitfield: BitField::empty(0),
            am_choking: true,
            am_interested: false,
            peer_choking: true,
            peer_interested: false,
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn is_chocked(&self) -> bool {
        self.peer_choking
    }

    pub fn is_interested(&self) -> bool {
        self.peer_interested
    }

    pub fn bitfield(&self) -> &BitField {
        &self.bitfield
    }

    fn bitfield_mut(&mut self) -> &mut BitField {
        &mut self.bitfield
    }

    pub fn set_bitfield(&mut self, bitfield: BitField) {
        self.bitfield = bitfield;
    }
}

#[derive(Debug)]
pub struct PeerConnection {
    peer: Peer,
    stream: TcpStream,
    num_pieces: usize,
}

impl PeerConnection {
    pub fn peer(&self) -> &Peer {
        &self.peer
    }

    pub async fn connect(
        mut peer: Peer,
        info_hash: &[u8; 20],
        peer_id: &[u8; 20],
        num_pieces: usize,
    ) -> Result<Self> {
        println!("\nTrying peer {}", peer.addr());

        let mut stream =
            match timeout(Duration::from_secs(5), TcpStream::connect(&peer.addr())).await {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => bail!("connection failed: {e}"),
                Err(_) => bail!("connection timed out"),
            };

        let handshake = &Self::build_handshake(info_hash, peer_id);
        if let Err(e) = stream.write_all(handshake).await {
            bail!("failed to send handshake: {e}");
        }

        let mut buf = [0u8; HANDSHAKE_SIZE];
        if let Err(e) = stream.read_exact(&mut buf).await {
            bail!("failed to read handshake: {e}");
        }

        if buf[0] != 19 {
            bail!("invalid pstrlen");
        }

        if &buf[1..20] != b"BitTorrent protocol" {
            bail!("invalid pstr");
        }

        if &buf[28..48] != info_hash {
            bail!("info hash does not match");
        }

        peer.peer_id = buf[48..68].try_into().unwrap();
        peer.set_bitfield(BitField::empty(num_pieces));

        println!("Connected to peer: {}", peer.addr());

        Ok(PeerConnection {
            peer,
            stream,
            num_pieces,
        })
    }

    fn build_handshake(info_hash: &[u8], client_id: &[u8]) -> Vec<u8> {
        let mut handshake = Vec::with_capacity(HANDSHAKE_SIZE);

        handshake.push(19);
        handshake.extend_from_slice(b"BitTorrent protocol");
        handshake.extend_from_slice(&[0u8; 8]);
        handshake.extend_from_slice(info_hash);
        handshake.extend_from_slice(client_id);

        handshake
    }

    pub async fn send_message(&mut self, message: Message) -> Result<()> {
        Ok(self.stream.write_all(&message.encode()).await?)
    }

    pub async fn read_message(&mut self) -> Result<Message> {
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf);

        if len == 0 {
            return Ok(Message::KeepAlive);
        }

        let mut buf = vec![0u8; len.try_into().unwrap()];
        self.stream.read_exact(&mut buf).await?;
        Message::decode(&buf)
    }

    pub async fn send_interested(&mut self) -> Result<()> {
        self.send_message(Message::Interested).await
    }

    pub async fn wait_until_ready(&mut self) -> Result<()> {
        loop {
            match self.read_message().await? {
                Message::BitField(b) => {
                    self.peer.set_bitfield(BitField::new(b, self.num_pieces));
                }
                Message::Have(index) => {
                    self.peer.bitfield_mut().set_piece(index as usize);
                }
                Message::Choke => {
                    self.peer.peer_choking = true;
                }
                Message::Unchoke => {
                    self.peer.peer_choking = false;
                }
                Message::Interested => {
                    self.peer.peer_interested = true;
                }
                Message::NotInterested => {
                    self.peer.peer_interested = false;
                }
                Message::KeepAlive => continue,
                msg => {
                    println!("ignoring during init: {:?}", msg);
                }
            }

            if !self.peer.peer_choking {
                return Ok(());
            }
        }
    }

    pub async fn download_piece(
        &mut self,
        piece_index: usize,
        piece_length: u32,
    ) -> Result<ActivePiece> {
        let num_blocks = (piece_length + BLOCK_SIZE - 1) / BLOCK_SIZE;
        let mut piece_buf = vec![0u8; piece_length as usize];
        let mut blocks_received = 0;

        for block in 0..num_blocks {
            self.request_block(block as usize, piece_index, piece_length)
                .await?;
        }

        while blocks_received < num_blocks {
            let msg = match tokio::time::timeout(Duration::from_secs(60), self.read_message()).await
            {
                Ok(res) => res?,
                Err(_) => bail!("peer timed out (no messages for 60s)"),
            };

            match msg {
                Message::Piece {
                    index,
                    begin,
                    block,
                } => {
                    if index as usize != piece_index {
                        bail!("received block for wrong piece");
                    }

                    piece_buf[(begin as usize)..(begin as usize + block.len())]
                        .copy_from_slice(&block);
                    blocks_received += 1;
                }
                Message::BitField(b) => {
                    self.peer.set_bitfield(BitField::new(b, self.num_pieces));
                }
                Message::Have(index) => {
                    self.peer.bitfield_mut().set_piece(index as usize);
                }
                Message::Choke => {
                    self.peer.peer_choking = true;
                }
                Message::Unchoke => {
                    self.peer.peer_choking = false;
                }
                Message::Interested => {
                    self.peer.peer_interested = true;
                }
                Message::NotInterested => {
                    self.peer.peer_interested = false;
                }
                Message::KeepAlive => continue,
                msg => println!("unexpected message: {:?}", msg),
            }
        }

        return Ok(ActivePiece {
            index: piece_index,
            length: piece_length,
            data: piece_buf,
        });
    }

    async fn request_block(
        &mut self,
        block_num: usize,
        piece_index: usize,
        piece_length: u32,
    ) -> Result<()> {
        let begin = (block_num as u32) * BLOCK_SIZE;
        let length = BLOCK_SIZE.min(piece_length as u32 - begin);

        self.send_message(Message::Request {
            index: piece_index as u32,
            begin,
            length,
        })
        .await
    }

    pub async fn send_have(&mut self, piece_index: usize) -> Result<()> {
        self.send_message(Message::Have(piece_index as u32)).await
    }
}

#[derive(Debug)]
pub enum Message {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    BitField(Vec<u8>),
    Request {
        index: u32,
        begin: u32,
        length: u32,
    },
    Piece {
        index: u32,
        begin: u32,
        block: Vec<u8>,
    },
    Cancel {
        index: u32,
        begin: u32,
        length: u32,
    },
}

impl Message {
    fn encode(&self) -> Vec<u8> {
        match self {
            Message::KeepAlive => Self::encode_keep_alive(),
            Message::Choke => Self::encode_state(0),
            Message::Unchoke => Self::encode_state(1),
            Message::Interested => Self::encode_state(2),
            Message::NotInterested => Self::encode_state(3),
            Message::Have(piece_index) => Self::encode_have(*piece_index),
            Message::BitField(bitfield) => Self::encode_bitfield(bitfield),
            Message::Request {
                index,
                begin,
                length,
            } => Self::encode_request(*index, *begin, *length),
            Message::Piece {
                index,
                begin,
                block,
            } => Self::encode_piece(*index, *begin, block),
            Message::Cancel {
                index,
                begin,
                length,
            } => Self::encode_cancel(*index, *begin, *length),
        }
    }

    fn encode_keep_alive() -> Vec<u8> {
        0_u32.to_be_bytes().to_vec()
    }

    fn encode_state(id: u8) -> Vec<u8> {
        let mut buf = Vec::with_capacity(5);

        buf.extend_from_slice(&1u32.to_be_bytes());
        buf.push(id);

        buf
    }

    fn encode_have(piece_index: u32) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4 + 5);

        buf.extend_from_slice(&5u32.to_be_bytes());
        buf.push(4);
        buf.extend_from_slice(&piece_index.to_be_bytes());

        buf
    }

    fn encode_bitfield(bitfield: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4 + 1 + bitfield.len());
        let length = 1 + bitfield.len();

        buf.extend_from_slice(&(length as u32).to_be_bytes());
        buf.push(5);
        buf.extend_from_slice(bitfield);

        buf
    }

    fn encode_request(index: u32, begin: u32, length: u32) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4 + 13);

        buf.extend_from_slice(&13u32.to_be_bytes());
        buf.push(6);
        buf.extend_from_slice(&index.to_be_bytes());
        buf.extend_from_slice(&begin.to_be_bytes());
        buf.extend_from_slice(&length.to_be_bytes());

        buf
    }

    fn encode_piece(index: u32, begin: u32, block: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4 + 9 + block.len());
        let length = 9 + block.len();

        buf.extend_from_slice(&(length as u32).to_be_bytes());
        buf.push(7);
        buf.extend_from_slice(&index.to_be_bytes());
        buf.extend_from_slice(&begin.to_be_bytes());
        buf.extend_from_slice(block);

        buf
    }

    fn encode_cancel(index: u32, begin: u32, length: u32) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4 + 13);

        buf.extend_from_slice(&13u32.to_be_bytes());
        buf.push(8);
        buf.extend_from_slice(&index.to_be_bytes());
        buf.extend_from_slice(&begin.to_be_bytes());
        buf.extend_from_slice(&length.to_be_bytes());

        buf
    }

    fn decode(buf: &[u8]) -> Result<Message> {
        let id = buf.first().ok_or_else(|| anyhow!("id missing"))?;
        let buf = &buf[1..];

        Ok(match id {
            0 => Message::Choke,
            1 => Message::Unchoke,
            2 => Message::Interested,
            3 => Message::NotInterested,
            4 => Self::decode_have(buf),
            5 => Self::decode_bitfield(buf),
            6 => Self::decode_request(buf),
            7 => Self::decode_piece(buf),
            8 => Self::decode_cancel(buf),
            _ => return Err(anyhow!("invalid id")),
        })
    }

    fn decode_have(buf: &[u8]) -> Message {
        let piece_index = u32::from_be_bytes(buf.try_into().unwrap());

        Message::Have(piece_index)
    }

    fn decode_bitfield(buf: &[u8]) -> Message {
        Message::BitField(buf.to_vec())
    }

    fn decode_request(buf: &[u8]) -> Message {
        let (chunks, _) = buf.as_chunks::<4>();

        let index = u32::from_be_bytes(chunks[0]);
        let begin = u32::from_be_bytes(chunks[1]);
        let length = u32::from_be_bytes(chunks[2]);

        Message::Request {
            index,
            begin,
            length,
        }
    }

    fn decode_piece(buf: &[u8]) -> Message {
        let (head, block) = buf.split_at(8);
        let (chunks, _) = head.as_chunks::<4>();

        let index = u32::from_be_bytes(chunks[0]);
        let begin = u32::from_be_bytes(chunks[1]);
        let block = block.to_vec();

        Message::Piece {
            index,
            begin,
            block,
        }
    }

    fn decode_cancel(buf: &[u8]) -> Message {
        let (chunks, _) = buf.as_chunks::<4>();

        let index = u32::from_be_bytes(chunks[0]);
        let begin = u32::from_be_bytes(chunks[1]);
        let length = u32::from_be_bytes(chunks[2]);

        Message::Cancel {
            index,
            begin,
            length,
        }
    }
}
