use std::{
    collections::BTreeMap,
    fs, io,
    net::SocketAddr,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result, anyhow};
use sha1::{Digest, Sha1};
use tokio::{fs::File, io::{AsyncSeekExt, AsyncWriteExt}, sync::Mutex, task::JoinSet};
use url::Url;

use crate::{
    bencode::{
        self, Object, ObjectType, decode_object,
        object::{extract_byte_array, extract_dict, extract_list, extract_num, extract_str},
    },
    client::Client,
    peer::{BitField, Peer, PeerConnection},
    tracker::{AnnounceStats, Tracker},
};

pub const BLOCK_SIZE: u32 = 16_384;

#[derive(Debug, Clone, PartialEq)]
enum PieceState {
    Missing,
    InProgress,
    Done,
}

#[derive(Debug, Clone)]
struct Piece {
    index: usize,
    length: usize,
    state: PieceState,
}

#[derive(Debug, Clone)]
pub struct Torrent {
    // core download fields
    info_hash: [u8; 20],
    piece_hashes: Vec<[u8; 20]>,
    piece_length: u64,
    length: u64,

    // metadata (only for serialization/display)
    name: String,
    tracker: Tracker,
    announce_list: Vec<Vec<Tracker>>,
    comment: String,
    created_by: String,
    creation_date: u64,

    // runtime state
    downloaded: Arc<AtomicU64>,
    left: Arc<AtomicU64>,
    uploaded: Arc<AtomicU64>,
    pieces: Arc<Mutex<Vec<Piece>>>,
    peers: Arc<Mutex<Vec<Peer>>>,
    file: Arc<Mutex<File>>,
}

impl Torrent {
    pub fn announce(&self) -> &Tracker {
        &self.tracker
    }

    pub fn announce_list(&self) -> &Vec<Vec<Tracker>> {
        &self.announce_list
    }

    pub fn comment(&self) -> &str {
        &self.comment
    }

    pub fn created_by(&self) -> &str {
        &self.created_by
    }

    pub fn creation_date(&self) -> u64 {
        self.creation_date
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn length(&self) -> u64 {
        self.length
    }

    pub fn piece_length(&self) -> u64 {
        self.piece_length
    }

    pub fn info_hash(&self) -> &[u8; 20] {
        &self.info_hash
    }

    pub fn piece_hashes(&self) -> &Vec<[u8; 20]> {
        &self.piece_hashes
    }

    pub async fn load_from_file(path: &Path) -> Result<Torrent> {
        let bytes = fs::read(path)?;
        let obj = decode_object(&bytes);
        Torrent::from_object(obj).await
    }

    pub async fn save_to_file(&self, path: &Path) -> io::Result<()> {
        let obj = Object::from_torrent(self);
        let bytes = bencode::encode_object(&obj);
        tokio::fs::write(
            format!(
                "{}/{}.torrent",
                path.to_string_lossy().to_string(),
                self.name
            ),
            bytes,
        )
        .await?;
        Ok(())
    }

    pub async fn update_trackers(&mut self, client: &Client) -> Result<()> {
        let addrs = self
            .tracker
            .announce(
                &self.info_hash,
                &client.peer_id,
                client.port,
                &AnnounceStats {
                    uploaded: self.uploaded.load(Ordering::Relaxed),
                    downloaded: self.downloaded.load(Ordering::Relaxed),
                    left: self.left.load(Ordering::Relaxed),
                },
            )
            .await?;

        self.add_peers(addrs).await;

        Ok(())
    }

    async fn add_peers(&self, addrs: Vec<SocketAddr>) {
        let mut peers = self.peers.lock().await;
        for addr in addrs {
            if !peers.iter().any(|p| p.addr() == addr) {
                peers.push(Peer::new(addr));
            }
        }
    }

    pub async fn download(&self, client: &Client) -> Result<()> {
        let peers: Vec<Peer> = self.peers.lock().await.drain(..).collect();

        let info_hash = self.info_hash;
        let peer_id = client.peer_id;
        let num_pieces = self.piece_hashes.len();

        let mut set = JoinSet::new();

        for peer in peers {
            println!("\nTrying peer {}", peer.addr());
            let mut torrent = self.clone();

            set.spawn({
                async move {
                    let mut conn =
                        match PeerConnection::connect(peer, &info_hash, &peer_id, num_pieces).await
                        {
                            Ok(conn) => conn,
                            Err((peer, e)) => {
                                eprintln!("Peer {} failed: {e}", peer.addr());
                                return;
                            }
                        };

                    if let Err(e) = conn.receive_initial_messages().await {
                        eprintln!("Failed to receive initial messages: {e}");
                        return;
                    }

                    if let Err(e) = conn.send_interested().await {
                        eprintln!("Failed to send interested: {e}");
                        return;
                    }

                    match torrent.download_from(&mut conn).await {
                        Ok(_) => return,
                        Err(e) => {
                            eprintln!("Download failed: {e}");
                            return;
                        }
                    }
                }
            });
        }

        while let Some(_) = set.join_next().await {}

        Ok(())
    }

    async fn download_from(&mut self, conn: &mut PeerConnection) -> Result<()> {
        loop {
            let Some(piece_idx) = self.pick_piece(conn.peer().bitfield()).await else {
                break;
            };

            let data = conn.download_piece(piece_idx, self.piece_length).await?;
            self.verify_and_store(piece_idx, &data).await?;
        }

        Ok(())
    }

    async fn pick_piece(&self, bitfield: &BitField) -> Option<usize> {
        let mut pieces = self.pieces.lock().await;
        let idx = (0..pieces.len()).find(|&piece| {
            bitfield.has_piece(piece) && pieces[piece].state == PieceState::Missing
        })?;
        pieces[idx].state = PieceState::InProgress;
        Some(idx)
    }

    async fn verify_and_store(&self, piece_idx: usize, data: &[u8]) -> Result<()> {
        let piece_hash: [u8; 20] = Sha1::digest(data).into();
        {
            let mut pieces = self.pieces.lock().await;
            if piece_hash != self.piece_hashes[piece_idx] {
                pieces[piece_idx].state = PieceState::Missing;
                return Err(anyhow!("Piece {} hash mismatch", piece_idx));
            }
        }

        self.downloaded
            .fetch_add(data.len() as u64, Ordering::Relaxed);
        self.left.fetch_sub(data.len() as u64, Ordering::Relaxed);

        {
            let mut file = self.file.lock().await;
            file.seek(io::SeekFrom::Current(piece_idx as i64 * self.piece_length as i64)).await?;
            file.write_all(data).await?;
            file.flush().await?;
        }

        {
            let mut pieces = self.pieces.lock().await;
            pieces[piece_idx].state = PieceState::Done;
        }

        println!("Verified piece {}", piece_idx);
        Ok(())
    }
}

impl Torrent {
    async fn from_object(object: Object) -> Result<Self> {
        let dict = match object.object_type() {
            ObjectType::Dictionary(d) => d,
            _ => return Err(anyhow!("Top level object is not a dictionary")),
        };

        let announce = Tracker::new(
            Url::parse(&extract_str(&dict, b"announce")?).context("invalid announce URL")?,
        );
        let announce_list = extract_announce_list(&dict)?;
        let comment = extract_str(&dict, b"comment")?;
        let created_by = extract_str(&dict, b"created by")?;
        let creation_date = u64::try_from(extract_num(&dict, b"creation date")?)
            .map_err(|_| anyhow!("creation date is negative or too large"))?;

        let info_obj = extract_dict(&dict, b"info")?;
        let name = extract_str(&info_obj, b"name")?;
        let total_length = u64::try_from(extract_num(&info_obj, b"length")?)
            .map_err(|_| anyhow!("length is negative or too large"))?;
        let piece_length = u64::try_from(extract_num(&info_obj, b"piece length")?)
            .map_err(|_| anyhow!("piece length is negative or too large"))?;
        let piece_hashes = extract_pieces(&info_obj)?;

        let mut pieces = Vec::with_capacity(piece_hashes.len());
        for i in 0..piece_hashes.len() {
            let length = if i == piece_hashes.len() - 1 {
                let last_piece_length = total_length - (piece_length * (i as u64));
                assert!(
                    last_piece_length > 0 && last_piece_length <= piece_length,
                    "last piece length {last_piece_length} is out of range"
                );
                last_piece_length
            } else {
                piece_length
            };

            pieces.push(Piece {
                index: i,
                length: length as usize,
                state: PieceState::Missing,
            });
        }

        let file = File::options().create(true).write(true).open(&name).await?;
        file.set_len(total_length).await?;

        let info_hash = compute_info_hash(&dict)?;

        Ok(Torrent {
            info_hash,
            piece_hashes,
            piece_length,
            length: total_length,
            name,
            tracker: announce,
            announce_list,
            comment,
            created_by,
            creation_date,
            downloaded: Arc::new(0.into()),
            left: Arc::new(total_length.into()),
            uploaded: Arc::new(0.into()),
            pieces: Arc::new(Mutex::new(pieces)),
            peers: Arc::new(Mutex::new(Vec::new())),
            file: Arc::new(Mutex::new(file)),
        })
    }
}

fn extract_announce_list(dict: &BTreeMap<Vec<u8>, Object>) -> Result<Vec<Vec<Tracker>>> {
    let tiers = extract_list(dict, b"announce-list")?;

    let mut announce_list = Vec::new();

    for tier in tiers {
        let mut trackers = Vec::new();

        let list = match tier.object_type() {
            ObjectType::List(l) => l,
            _ => {
                return Err(anyhow!(
                    "Expected key {} to be of type {}",
                    "announce-list",
                    "list"
                ));
            }
        };

        for obj in list {
            let bytes = match obj.object_type() {
                ObjectType::ByteArray(b) => b,
                _ => {
                    return Err(anyhow!(
                        "Expected key {} to be {}",
                        "announce-list",
                        "byte string",
                    )
                    .into());
                }
            };

            let url = String::from_utf8(bytes.to_vec())?;

            trackers.push(Tracker::new(Url::parse(&url)?));
        }

        announce_list.push(trackers);
    }

    Ok(announce_list)
}

fn compute_info_hash(dict: &BTreeMap<Vec<u8>, Object>) -> Result<[u8; 20]> {
    let info = dict
        .get(b"info".as_slice())
        .ok_or(anyhow!("Missing key info"))?;
    Ok(Sha1::digest(&info.bytes()).into())
}

fn extract_pieces(info_dict: &BTreeMap<Vec<u8>, Object>) -> Result<Vec<[u8; 20]>> {
    let arr = extract_byte_array(info_dict, b"pieces")?;
    chunk_array::<20>(&arr)
}

fn chunk_array<const N: usize>(data: &[u8]) -> Result<Vec<[u8; N]>> {
    if data.len() % N != 0 {
        return Err(anyhow!("Length {} is not a mupliple of {}", data.len(), N));
    }

    let mut result = Vec::with_capacity(data.len() / N);

    for chunk in data.chunks(N) {
        let mut arr = [0u8; N];
        arr.copy_from_slice(chunk);
        result.push(arr);
    }

    Ok(result)
}
