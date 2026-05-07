use std::{
    collections::{BTreeMap, HashSet},
    fs, io,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use sha1::{Digest, Sha1};
use tokio::{
    fs::File,
    io::{AsyncSeekExt, AsyncWriteExt},
    sync::{
        Mutex,
        mpsc::{self, Sender},
    },
    task::JoinSet,
};
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
pub enum PieceState {
    Missing,
    InProgress,
    Downloaded { data: Vec<u8> },
    Done,
}

#[derive(Debug, Clone)]
pub struct Piece {
    pub index: usize,
    pub length: usize,
    pub state: PieceState,
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
    tracker: Arc<Tracker>,
    announce_list: Vec<Vec<Arc<Tracker>>>,
    comment: String,
    created_by: String,
    creation_date: u64,

    // runtime state
    downloaded: Arc<AtomicU64>,
    left: Arc<AtomicU64>,
    uploaded: Arc<AtomicU64>,
    pieces: Arc<Mutex<Vec<Piece>>>,
    peers: Arc<Mutex<Vec<Peer>>>,
    file_tx: mpsc::Sender<Piece>,
}

impl Torrent {
    pub fn announce(&self) -> &Tracker {
        &self.tracker
    }

    pub fn announce_list(&self) -> &Vec<Vec<Arc<Tracker>>> {
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

    pub async fn is_completed(&self) -> bool {
        self.pieces
            .lock()
            .await
            .iter()
            .all(|p| p.state == PieceState::Done)
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

    pub async fn download(&self, client: &Client) -> Result<()> {
        let (peer_tx, peer_rx) = mpsc::channel::<Vec<Peer>>(1);

        let announce_task = tokio::spawn({
            let torrent = self.clone();
            let client = client.clone();

            async move { torrent.run_announce_loop(peer_tx, &client).await }
        });

        let download_task = tokio::spawn({
            let torrent = self.clone();
            let client = client.clone();

            async move { torrent.run_download_loop(peer_rx, &client).await }
        });

        let (announce_result, download_result) = tokio::join!(announce_task, download_task);

        announce_result??;
        download_result??;

        Ok(())
    }

    async fn run_announce_loop(&self, peer_tx: Sender<Vec<Peer>>, client: &Client) -> Result<()> {
        let mut addr_set = HashSet::new();

        loop {
            if self.tracker.is_due() {
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

                let mut peers = Vec::new();
                for addr in addrs {
                    if addr_set.insert(addr) {
                        peers.push(Peer::new(addr));
                    }
                }

                if !peers.is_empty() {
                    peer_tx.send(peers).await?;
                }
            }

            tokio::time::sleep(self.tracker.interval()).await;
        }
    }

    fn process_peers(&self, peers: Vec<Peer>, join_set: &mut JoinSet<Result<()>>, client: &Client) {
        for peer in peers {
            let torrent = self.clone();
            let client = client.clone();

            join_set.spawn(async move {
                let addr = peer.addr();
                let mut conn = PeerConnection::connect(
                    peer,
                    &torrent.info_hash,
                    &client.peer_id,
                    torrent.piece_hashes.len(),
                )
                .await
                .context(format!("peer {addr} failed"))?;

                conn.wait_until_ready()
                    .await
                    .context("failed to receive initial messages")?;
                conn.send_interested()
                    .await
                    .context("failed to send interested")?;

                torrent
                    .download_from_peer(&mut conn)
                    .await
                    .context("download failed")
            });
        }
    }

    async fn run_download_loop(
        &self,
        mut peer_rx: mpsc::Receiver<Vec<Peer>>,
        client: &Client,
    ) -> Result<()> {
        let mut join_set = JoinSet::new();

        loop {
            tokio::select! {
                Some(peers) = peer_rx.recv() => {
                    self.process_peers(peers, &mut join_set, client);
                }
                Some(res) = join_set.join_next() => {
                    match res {
                        Ok(Ok(())) => {},
                        Ok(Err(e)) => eprintln!("peer connection failed: {e}"),
                        Err(e) => eprintln!("peer task panicked: {e}"),
                    }

                    if self.is_completed().await {
                        join_set.abort_all();
                        break Ok(());
                    }
                }
                else => return Err(anyhow!("ran out of peers before download completed")),
            }
        }
    }

    async fn download_from_peer(&self, conn: &mut PeerConnection) -> Result<()> {
        loop {
            let Some(piece_idx) = self.pick_piece(conn.peer().bitfield()).await else {
                break;
            };

            let piece_len = {
                let pieces = self.pieces.lock().await;
                pieces[piece_idx].length as u64
            };

            let res = tokio::time::timeout(
                Duration::from_mins(3),
                conn.download_piece(piece_idx, piece_len),
            )
            .await
            .context("download timed out")
            .flatten();

            let piece = match res {
                Ok(p) => p,
                Err(e) => {
                    self.pieces.lock().await[piece_idx].state = PieceState::Missing;
                    return Err(e);
                }
            };

            self.verify_and_store_piece(piece).await?;
        }

        Ok(())
    }

    async fn pick_piece(&self, bitfield: &BitField) -> Option<usize> {
        let mut pieces = self.pieces.lock().await;
        let idx = pieces.iter().enumerate().find_map(|(i, piece)| {
            (bitfield.has_piece(i) && piece.state == PieceState::Missing).then_some(i)
        })?;
        pieces[idx].state = PieceState::InProgress;
        Some(idx)
    }

    async fn verify_and_store_piece(&self, piece: Piece) -> Result<()> {
        let data = match &piece.state {
            PieceState::Downloaded { data } => data,
            _ => unreachable!("piece must be in Downloaded state here"),
        };

        let piece_hash: [u8; 20] = Sha1::digest(&data).into();
        if piece_hash != self.piece_hashes[piece.index] {
            self.pieces.lock().await[piece.index].state = PieceState::Missing;
            bail!("piece {} hash mismatch", piece.index);
        }

        self.downloaded
            .fetch_add(piece.length as u64, Ordering::Relaxed);
        self.left.fetch_sub(piece.length as u64, Ordering::Relaxed);

        self.file_tx.send(piece.clone()).await?;

        self.pieces.lock().await[piece.index].state = PieceState::Done;

        println!("downloaded piece {}", piece.index);

        Ok(())
    }

    async fn from_object(object: Object) -> Result<Self> {
        let dict = match object.object_type() {
            ObjectType::Dictionary(d) => d,
            _ => bail!("top level object is not a dictionary"),
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

        let info_hash = compute_info_hash(&dict)?;

        let (file_tx, file_rx) = mpsc::channel::<Piece>(32);
        tokio::spawn(Self::file_writer_task(total_length, file_rx, name.clone()));

        Ok(Torrent {
            info_hash,
            piece_hashes,
            piece_length,
            length: total_length,
            name,
            tracker: Arc::new(announce),
            announce_list,
            comment,
            created_by,
            creation_date,
            downloaded: Arc::new(0.into()),
            left: Arc::new(total_length.into()),
            uploaded: Arc::new(0.into()),
            pieces: Arc::new(Mutex::new(pieces)),
            peers: Arc::new(Mutex::new(Vec::new())),
            file_tx,
        })
    }

    async fn file_writer_task(
        total_length: u64,
        mut rx: mpsc::Receiver<Piece>,
        name: String,
    ) -> Result<()> {
        let mut file = File::options()
            .create(true)
            .write(true)
            .read(true)
            .open(name)
            .await?;

        file.set_len(total_length).await?;

        while let Some(piece) = rx.recv().await {
            file.seek(io::SeekFrom::Start(
                piece.index as u64 * piece.length as u64,
            ))
            .await?;

            let data = match piece.state {
                PieceState::Downloaded { data } => data,
                _ => unreachable!("piece must be in Downloaded state here"),
            };

            file.write_all(&data).await?;
            file.flush().await?;
        }

        Ok(())
    }
}

fn extract_announce_list(dict: &BTreeMap<Vec<u8>, Object>) -> Result<Vec<Vec<Arc<Tracker>>>> {
    let tiers = extract_list(dict, b"announce-list")?;

    let mut announce_list = Vec::new();

    for tier in tiers {
        let mut trackers = Vec::new();

        let list = match tier.object_type() {
            ObjectType::List(l) => l,
            _ => bail!("expected key announce-list to be of type list"),
        };

        for obj in list {
            let bytes = match obj.object_type() {
                ObjectType::ByteArray(b) => b,
                _ => bail!("Expected key announce-list to be byte string",),
            };

            let url = String::from_utf8(bytes.to_vec())?;
            trackers.push(Arc::new(Tracker::new(Url::parse(&url)?)));
        }

        announce_list.push(trackers);
    }

    Ok(announce_list)
}

fn compute_info_hash(dict: &BTreeMap<Vec<u8>, Object>) -> Result<[u8; 20]> {
    let info = dict
        .get(b"info".as_slice())
        .ok_or(anyhow!("missing key info"))?;
    Ok(Sha1::digest(&info.bytes()).into())
}

fn extract_pieces(info_dict: &BTreeMap<Vec<u8>, Object>) -> Result<Vec<[u8; 20]>> {
    let arr = extract_byte_array(info_dict, b"pieces")?;
    chunk_array::<20>(&arr)
}

fn chunk_array<const N: usize>(data: &[u8]) -> Result<Vec<[u8; N]>> {
    if data.len() % N != 0 {
        return Err(anyhow!("length {} is not a mupliple of {N}", data.len()));
    }

    let mut result = Vec::with_capacity(data.len() / N);

    for chunk in data.chunks(N) {
        let mut arr = [0u8; N];
        arr.copy_from_slice(chunk);
        result.push(arr);
    }

    Ok(result)
}
