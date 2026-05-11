use std::{collections::HashSet, net::SocketAddr, sync::Arc};

use anyhow::{Context, Result};
use tokio::{sync::mpsc, task::JoinSet};
use tokio_util::sync::CancellationToken;

use crate::{
    client::Client,
    peer::{Peer, PeerConnection},
    piece::ActivePiece,
    piece_picker::{PieceEvent, PiecePickerCommand},
    torrent_info::TorrentInfo,
};

#[derive(Debug)]
pub struct PeerManager {
    info: Arc<TorrentInfo>,
    client: Client,
    cancellation_token: CancellationToken,

    join_set: JoinSet<Result<()>>,
    address_set: HashSet<SocketAddr>,

    peer_rx: mpsc::Receiver<Vec<Peer>>,
    active_piece_tx: mpsc::Sender<ActivePiece>,
    piece_picker_event_tx: mpsc::Sender<PiecePickerCommand>,
    piece_event_tx: mpsc::Sender<PieceEvent>,
}

impl PeerManager {
    pub fn new(
        info: Arc<TorrentInfo>,
        client: Client,
        cancellation_token: CancellationToken,
        peer_rx: mpsc::Receiver<Vec<Peer>>,
        active_piece_tx: mpsc::Sender<ActivePiece>,
        piece_picker_event_tx: mpsc::Sender<PiecePickerCommand>,
        piece_event_tx: mpsc::Sender<PieceEvent>,
    ) -> Self {
        Self {
            info,
            client,
            cancellation_token,
            join_set: JoinSet::new(),
            address_set: HashSet::new(),
            peer_rx,
            active_piece_tx,
            piece_picker_event_tx,
            piece_event_tx,
        }
    }

    pub async fn run(&mut self) {
        loop {
            tokio::select! {
                Some(peers) = self.peer_rx.recv() => {
                    self.process_peers(peers);
                }
                Some(res) = self.join_set.join_next() => {
                    match res {
                        Ok(Ok(())) => {},
                        Ok(Err(e)) => eprintln!("peer connection failed: {e}"),
                        Err(e) => eprintln!("peer task panicked: {e}"),
                    }
                }
                _ = self.cancellation_token.cancelled() => {
                    self.join_set.abort_all();
                    break;
                }
            }
        }
    }

    fn process_peers(&mut self, peers: Vec<Peer>) {
        for peer in peers {
            if !self.address_set.insert(peer.addr) {
                continue;
            }

            let info = Arc::clone(&self.info);
            let client = self.client.clone();
            let active_piece_tx = self.active_piece_tx.clone();
            let piece_picker_event_tx = self.piece_picker_event_tx.clone();
            let piece_event_tx = self.piece_event_tx.clone();

            self.join_set.spawn(async move {
                let addr = peer.addr;
                let mut conn = PeerConnection::connect(
                    info,
                    peer,
                    &client.peer_id,
                    active_piece_tx,
                    piece_picker_event_tx,
                    piece_event_tx,
                )
                .await
                .context(format!("peer {addr} failed"))?;

                conn.send_interested()
                    .await
                    .context("failed to send interested")?;

                conn.wait_until_ready()
                    .await
                    .context("failed to receive initial messages")?;

                conn.run().await.context("download failed")
            });
        }
    }
}
