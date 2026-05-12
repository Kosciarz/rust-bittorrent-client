use std::sync::Arc;

use anyhow::{Result, bail};
use tokio::sync::mpsc::{self};
use tokio_util::sync::CancellationToken;

use crate::{
    client::Client,
    file_writer::FileWriter,
    peer::Peer,
    peer_manager::PeerManager,
    piece::{ActivePiece, CompletedPiece},
    piece_picker::PiecePicker,
    piece_validator::PieceValidator,
    stats_manager::StatsManager,
    torrent_info::TorrentInfo,
    tracker_manager::TrackerManager,
};

#[derive(Debug)]
pub enum TorrentEvent {
    Completed,
}

#[derive(Debug)]
pub struct TorrentSession {
    info: Arc<TorrentInfo>,
    cancellation_token: CancellationToken,

    torrent_event_rx: mpsc::Receiver<TorrentEvent>,
}

impl TorrentSession {
    pub async fn new(info: Arc<TorrentInfo>, client: &Client) -> Result<Self> {
        let (stats_manager_command_tx, stats_manager_command_rx) = mpsc::channel(32);
        let mut stats_manager = StatsManager::new(info.length, stats_manager_command_rx);
        tokio::spawn(async move { stats_manager.run().await });

        let (torrent_event_tx, torrent_event_rx) = mpsc::channel(10);

        let (piece_event_tx, piece_event_rx) = mpsc::channel(256);

        let (piece_picker_event_tx, piece_picker_event_rx) = mpsc::channel(32);
        let mut piece_picker = PiecePicker::new(
            info.pieces.len(),
            piece_event_rx,
            piece_picker_event_rx,
            torrent_event_tx,
        );
        tokio::spawn(async move { piece_picker.run().await });

        let (completed_piece_tx, completed_piece_rx) = mpsc::channel::<CompletedPiece>(32);
        let mut file_writer = FileWriter::new(
            Arc::clone(&info),
            completed_piece_rx,
            piece_event_tx.clone(),
            stats_manager_command_tx.clone(),
        )
        .await?;
        tokio::spawn(async move { file_writer.run().await });

        let (active_piece_tx, active_piece_rx) = mpsc::channel::<ActivePiece>(32);
        let mut piece_assembler = PieceValidator::new(
            info.piece_hashes(),
            active_piece_rx,
            completed_piece_tx.clone(),
            piece_event_tx.clone(),
        );
        tokio::spawn(async move { piece_assembler.run().await });

        let cancellation_token = CancellationToken::new();

        let (peer_tx, peer_rx) = mpsc::channel::<Vec<Peer>>(10);

        let mut tracker_manager = TrackerManager::new(
            Arc::clone(&info),
            client.clone(),
            cancellation_token.clone(),
            stats_manager_command_tx.clone(),
            peer_tx,
        );
        tokio::spawn(async move { tracker_manager.run().await });

        let mut peer_manager = PeerManager::new(
            Arc::clone(&info),
            client.clone(),
            cancellation_token.clone(),
            peer_rx,
            active_piece_tx.clone(),
            piece_picker_event_tx.clone(),
            piece_event_tx.clone(),
        );
        tokio::spawn(async move { peer_manager.run().await });

        Ok(Self {
            info,
            cancellation_token,
            torrent_event_rx,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        match self.torrent_event_rx.recv().await {
            Some(TorrentEvent::Completed) => {
                self.cancellation_token.cancel();
                return Ok(());
            }
            None => {
                self.cancellation_token.cancel();
                bail!("ran out of peers before download completed")
            }
        }
    }
}
