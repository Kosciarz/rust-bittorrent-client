use tokio::sync::mpsc;

use crate::{
    piece::{ActivePiece, CompletedPiece},
    piece_picker::PieceEvent,
};

#[derive(Debug)]
pub struct PieceValidator {
    piece_hashes: Vec<[u8; 20]>,

    active_piece_rx: mpsc::Receiver<ActivePiece>,
    completed_piece_tx: mpsc::Sender<CompletedPiece>,
    piece_event_tx: mpsc::Sender<PieceEvent>,
}

impl PieceValidator {
    pub fn new(
        piece_hashes: Vec<[u8; 20]>,
        active_piece_rx: mpsc::Receiver<ActivePiece>,
        completed_piece_tx: mpsc::Sender<CompletedPiece>,
        piece_event_tx: mpsc::Sender<PieceEvent>,
    ) -> Self {
        Self {
            piece_hashes,
            active_piece_rx,
            completed_piece_tx,
            piece_event_tx,
        }
    }

    pub async fn run(&mut self) {
        while let Some(mut piece) = self.active_piece_rx.recv().await {
            if piece.verify(&self.piece_hashes[piece.index]) {
                let _ = self
                    .completed_piece_tx
                    .send(CompletedPiece {
                        index: piece.index,
                        data: std::mem::take(&mut piece.data),
                    })
                    .await;
            } else {
                let _ = self
                    .piece_event_tx
                    .send(PieceEvent::HashMismatch {
                        piece_index: piece.index,
                    })
                    .await;
            }
        }
    }
}
