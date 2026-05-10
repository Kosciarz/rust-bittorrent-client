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
        mpsc::{self, Sender},
        oneshot,
    },
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

use crate::{
    client::Client,
    file_writer::FileWriter,
    peer::{Peer, PeerConnection},
    piece::{ActivePiece, CompletedPiece},
    piece_assembler::PieceValidator,
    piece_picker::{PieceEvent, PiecePicker, PiecePickerCommand},
    torrent_info::TorrentInfo,
    tracker::{AnnounceStats, Tracker},
};

#[derive(Debug)]
pub struct Stats {
    pub downloaded: AtomicU64,
    pub left: AtomicU64,
    pub uploaded: AtomicU64,
}

#[derive(Debug, Clone)]
pub struct TorrentSession {
    pub info: Arc<TorrentInfo>,
    stats: Arc<Stats>,

    tracker: Arc<Tracker>,
    tracker_list: Vec<Vec<Arc<Tracker>>>,

    piece_event_tx: mpsc::Sender<PieceEvent>,
    piece_picker_event_tx: mpsc::Sender<PiecePickerCommand>,
    piece_tx: mpsc::Sender<ActivePiece>,
}

impl TorrentSession {
    pub async fn new(info: Arc<TorrentInfo>) -> Result<Self> {
        let (piece_event_tx, piece_event_rx) = mpsc::channel(256);

        let (piece_picker_event_tx, piece_picker_event_rx) = mpsc::channel(32);
        let mut piece_picker =
        PiecePicker::new(info.pieces.len(), piece_event_rx, piece_picker_event_rx);
        tokio::spawn(async move { piece_picker.run().await });

        let (completed_piece_tx, completed_piece_rx) = mpsc::channel::<CompletedPiece>(32);
        let mut file_writer =
        FileWriter::new(info.length, info.name.clone(), info.piece_length, completed_piece_rx, piece_event_tx.clone()).await?;
        tokio::spawn(async move { file_writer.run().await });

        let (piece_tx, piece_rx) = mpsc::channel::<ActivePiece>(32);
        let mut piece_assembler = PieceValidator::new(
            info.piece_hashes(),
            piece_rx,
            completed_piece_tx.clone(),
            piece_event_tx.clone(),
        );
        tokio::spawn(async move { piece_assembler.run().await });

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

        Ok(Self {
            info,
            stats,
            tracker,
            tracker_list,
            piece_event_tx,
            piece_picker_event_tx,
            piece_tx,
        })
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

        let mut interval = tokio::time::interval(self.tracker.interval());

        loop {
            tokio::select! {
                _ = interval.tick() => {
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
                            peers.push(Peer { addr });
                        }
                    }

                    if !peers.is_empty() {
                        peer_tx.send(peers).await?;
                    }
                },
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
                let addr = peer.addr;
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
        loop {
            let (tx, rx) = oneshot::channel();

            self.piece_picker_event_tx
                .send(PiecePickerCommand::RequestPiece {
                    bitfield: conn.bitfield().clone(),
                    response_tx: tx,
                })
                .await?;

            let Some(piece_idx) = rx.await? else {
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
                    let _ = self
                        .piece_event_tx
                        .send(PieceEvent::DownloadFailed {
                            piece_index: piece_idx,
                        })
                        .await;
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

            println!("Downloaded piece {}", piece.index);
        }

        Ok(())
    }
}
