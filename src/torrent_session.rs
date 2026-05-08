use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, bail};
use tokio::{
    sync::{
        Mutex,
        broadcast::{self, error::TryRecvError},
        mpsc::{self, Sender},
    },
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

use crate::{
    client::Client,
    file_writer::FileWriter,
    peer::{BitField, Peer, PeerConnection},
    piece::{ActivePiece, CompletedPiece, PieceState},
    piece_assembler::PieceAssembler,
    torrent_info::TorrentInfo,
    tracker::{AnnounceStats, Tracker},
};

#[derive(Debug, Clone)]
pub enum TorrentEvent {
    PieceCompleted { piece_index: usize },
}

#[derive(Debug)]
pub struct Stats {
    pub downloaded: AtomicU64,
    pub left: AtomicU64,
    pub uploaded: AtomicU64,
}

#[derive(Debug, Clone)]
pub struct TorrentSession {
    pub info: Arc<TorrentInfo>,

    tracker: Arc<Tracker>,
    tracker_list: Vec<Vec<Arc<Tracker>>>,

    stats: Arc<Stats>,

    piece_states: Arc<Mutex<Vec<PieceState>>>,
    event_tx: broadcast::Sender<TorrentEvent>,
    piece_tx: mpsc::Sender<ActivePiece>,
}

impl TorrentSession {
    pub async fn spawn(info: Arc<TorrentInfo>) -> Result<Self> {
        let (file_tx, file_rx) = mpsc::channel::<CompletedPiece>(32);
        let mut file_writer =
            FileWriter::new(info.length, info.name.clone(), info.piece_length, file_rx).await?;
        tokio::spawn(async move { file_writer.run().await });

        let (piece_tx, piece_rx) = mpsc::channel::<ActivePiece>(32);
        let mut piece_assembler =
            PieceAssembler::new(info.piece_hashes(), piece_rx, file_tx.clone());
        tokio::spawn(async move { piece_assembler.run().await });

        let (event_tx, _) = broadcast::channel(256);

        let tracker = Arc::new(Tracker::new(info.announce.clone()));

        let mut tracker_list = Vec::new();
        for tier in &info.announce_list {
            let mut trackers = Vec::new();

            for tracker in tier {
                trackers.push(Arc::new(Tracker::new(tracker.clone())));
            }

            tracker_list.push(trackers);
        }

        let stats = Arc::new(Stats {
            downloaded: 0.into(),
            left: info.length.into(),
            uploaded: 0.into(),
        });
        let piece_states = Arc::new(Mutex::new(vec![PieceState::Missing; info.pieces.len()]));

        Ok(Self {
            info,
            tracker,
            tracker_list,
            stats,
            piece_states,
            event_tx,
            piece_tx,
        })
    }

    pub async fn is_completed(&self) -> bool {
        self.piece_states
            .lock()
            .await
            .iter()
            .all(|state| *state == PieceState::Done)
    }

    pub async fn run(&self, client: &Client) -> Result<()> {
        let (peer_tx, peer_rx) = mpsc::channel::<Vec<Peer>>(1);

        let cancel = CancellationToken::new();

        let announce_task = tokio::spawn({
            let torrent = self.clone();
            let client = client.clone();
            let cancel = cancel.clone();

            async move { torrent.run_announce_loop(peer_tx, &client, cancel).await }
        });

        let download_task = tokio::spawn({
            let torrent = self.clone();
            let client = client.clone();
            let cancel = cancel.clone();

            async move { torrent.run_download_loop(peer_rx, &client, cancel).await }
        });

        let (announce_result, download_result) = tokio::join!(announce_task, download_task);

        announce_result??;
        download_result??;

        Ok(())
    }

    async fn run_announce_loop(
        &self,
        peer_tx: Sender<Vec<Peer>>,
        client: &Client,
        cancel: CancellationToken,
    ) -> Result<()> {
        let mut addr_set = HashSet::new();

        loop {
            if self.tracker.is_due() {
                let addrs = self
                    .tracker
                    .announce(
                        &self.info.info_hash,
                        &client.peer_id,
                        client.port,
                        &AnnounceStats {
                            uploaded: self.stats.uploaded.load(Ordering::Relaxed),
                            downloaded: self.stats.downloaded.load(Ordering::Relaxed),
                            left: self.stats.left.load(Ordering::Relaxed),
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

            tokio::select! {
                _ = tokio::time::sleep(self.tracker.interval()) => {},
                _ = cancel.cancelled() => return Ok(())
            }
        }
    }

    async fn run_download_loop(
        &self,
        mut peer_rx: mpsc::Receiver<Vec<Peer>>,
        client: &Client,
        cancel: CancellationToken,
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
                        cancel.cancel();
                        return Ok(());
                    }
                }
                else => {
                    cancel.cancel();
                    bail!("ran out of peers before download completed");
                },
            }
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
                    &torrent.info.info_hash,
                    &client.peer_id,
                    torrent.info.pieces.len(),
                )
                .await
                .context(format!("peer {addr} failed"))?;

                conn.send_interested()
                    .await
                    .context("failed to send interested")?;

                conn.wait_until_ready()
                    .await
                    .context("failed to receive initial messages")?;

                torrent
                    .download_from_peer(&mut conn)
                    .await
                    .context("download failed")
            });
        }
    }

    async fn download_from_peer(&self, conn: &mut PeerConnection) -> Result<()> {
        let mut event_rx = self.event_tx.subscribe();

        loop {
            loop {
                match event_rx.try_recv() {
                    Ok(TorrentEvent::PieceCompleted { piece_index }) => {
                        conn.send_have(piece_index).await?;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Lagged(n)) => {
                        println!("have broadcast lagged by {n}")
                    }
                    Err(TryRecvError::Closed) => return Ok(()),
                }
            }

            let Some(piece_idx) = self.pick_piece(conn.peer().bitfield()).await else {
                break;
            };

            let piece_len = self.info.pieces[piece_idx].length;

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
                    self.piece_states.lock().await[piece_idx] = PieceState::Missing;
                    return Err(e);
                }
            };

            let _ = self.piece_tx.send(piece.clone()).await;

            self.stats
                .downloaded
                .fetch_add(piece.length as u64, Ordering::Relaxed);
            self.stats
                .left
                .fetch_sub(piece.length as u64, Ordering::Relaxed);

            self.piece_states.lock().await[piece.index] = PieceState::Done;

            let _ = self.event_tx.send(TorrentEvent::PieceCompleted {
                piece_index: piece.index,
            });

            println!("Downloaded piece {}", piece.index);
        }

        Ok(())
    }

    async fn pick_piece(&self, bitfield: &BitField) -> Option<usize> {
        let mut piece_states = self.piece_states.lock().await;
        let idx = piece_states.iter().enumerate().find_map(|(i, state)| {
            (bitfield.has_piece(i) && *state == PieceState::Missing).then_some(i)
        })?;
        piece_states[idx] = PieceState::InProgress;
        Some(idx)
    }
}
