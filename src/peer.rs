use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

use anyhow::{Result, anyhow};

const HANDSHAKE_SIZE: usize = 68;

#[derive(Debug)]
pub struct Peer {
    addr: SocketAddr,
    chocked: bool,
    interested: bool,
}

impl Peer {
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            chocked: true,
            interested: false,
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn chocked(&self) -> bool {
        self.chocked
    }

    pub fn interested(&self) -> bool {
        self.interested
    }

    pub fn connect(&mut self, info_hash: &[u8], client_id: &[u8]) -> Result<()> {
        let mut connection = PeerConnection::new(&self.addr)?;
        println!("Connected to peer: {}", self.addr);

        connection.send_handshake(info_hash, client_id)?;
        connection.read_handshake(info_hash)?;
        connection.read_loop()?;

        Ok(())
    }
}

#[derive(Debug)]
struct PeerConnection {
    stream: TcpStream,
    handshake_sent: bool,
}

impl PeerConnection {
    fn new(socket: &SocketAddr) -> io::Result<Self> {
        Ok(Self {
            stream: TcpStream::connect_timeout(socket, Duration::from_secs(5))?,
            handshake_sent: false,
        })
    }

    fn send_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        Ok(self.stream.write_all(bytes)?)
    }

    fn encode_handshake(info_hash: &[u8], client_id: &[u8]) -> Vec<u8> {
        let mut handshake = Vec::with_capacity(HANDSHAKE_SIZE);

        handshake.push(19);
        handshake.extend_from_slice(b"BitTorrent protocol");
        handshake.extend_from_slice(&[0u8; 8]);
        handshake.extend_from_slice(info_hash);
        handshake.extend_from_slice(client_id);

        handshake
    }

    fn send_handshake(&mut self, info_hash: &[u8], client_id: &[u8]) -> io::Result<()> {
        if self.handshake_sent {
            return Ok(());
        }

        self.send_bytes(&Self::encode_handshake(info_hash, client_id))?;
        self.handshake_sent = true;

        Ok(())
    }

    fn read_handshake(&mut self, info_hash: &[u8]) -> Result<()> {
        let mut buffer = [0u8; HANDSHAKE_SIZE];

        self.stream.read_exact(&mut buffer)?;

        if buffer[0] != 19 {
            return Err(anyhow!("Invalid pstrlen"));
        }

        if &buffer[1..20] != b"BitTorrent protocol" {
            return Err(anyhow!("Invalid pstr"));
        }

        let mut reserved = Vec::new();
        reserved.extend_from_slice(&buffer[20..28]);

        if &buffer[28..48] != info_hash {
            return Err(anyhow!("Info hash does not match"));
        }

        let mut peer_id = Vec::new();
        peer_id.extend_from_slice(&buffer[48..68]);

        Ok(())
    }

    fn read_loop(&mut self) -> Result<()> {
        loop {
            let mut len_buf = [0u8; 4];
            self.stream.read_exact(&mut len_buf)?;
            let len = u32::from_be_bytes(len_buf);

            if len == 0 {
                println!("Message: Keep alive");
                continue;
            }

            let mut buf = vec![0u8; len.try_into().unwrap()];
            self.stream.read_exact(&mut buf)?;
            let message = Message::decode(&buf)?;

            println!("Message: {:?}", message);
        }
    }
}

#[derive(Debug)]
enum Message {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    Bitfield(Vec<u8>),
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
            Message::Bitfield(bitfield) => Self::encode_bitfield(bitfield),
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
        let id = buf.first().ok_or_else(|| anyhow!("Id missing"))?;
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
            _ => return Err(anyhow!("Invalid id")),
        })
    }

    fn decode_have(buf: &[u8]) -> Message {
        let piece_index = u32::from_be_bytes(buf.try_into().unwrap());

        Message::Have(piece_index)
    }

    fn decode_bitfield(buf: &[u8]) -> Message {
        Message::Bitfield(buf.to_vec())
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
