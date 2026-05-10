use tokio::sync::{mpsc, oneshot};

use crate::{bitfield::BitField};

#[derive(Debug, Clone, PartialEq)]
pub enum PieceState {
    Missing,
    InProgress,
    Completed,
}

#[derive(Debug, Clone)]
pub enum PieceEvent {
    Completed { piece_index: usize },
    HashMismatch { piece_index: usize },
    DownloadFailed { piece_index: usize },
}

#[derive(Debug)]
pub enum PiecePickerCommand {
    RequestPiece {
        bitfield: BitField,
        response_tx: oneshot::Sender<Option<usize>>,
    },
}

#[derive(Debug)]
pub struct PiecePicker {
    states: Vec<PieceState>,

    piece_event_rx: mpsc::Receiver<PieceEvent>,
    piece_picker_event_rx: mpsc::Receiver<PiecePickerCommand>,
}

impl PiecePicker {
    pub fn new(
        num_pieces: usize,
        piece_event_rx: mpsc::Receiver<PieceEvent>,
        piece_picker_event_rx: mpsc::Receiver<PiecePickerCommand>,
    ) -> Self {
        Self {
            states: vec![PieceState::Missing; num_pieces],
            piece_event_rx,
            piece_picker_event_rx,
        }
    }

    pub async fn run(&mut self) {
        loop {
            tokio::select!(
                Some(event) = self.piece_event_rx.recv() =>  {
                    match event {
                        PieceEvent::HashMismatch { piece_index }
                        | PieceEvent::DownloadFailed { piece_index } => {
                            self.mark_as_failed(piece_index);
                        }
                        PieceEvent::Completed { piece_index } => {
                            self.mark_as_completed(piece_index);
                        },
                    }
                }
                Some(event) = self.piece_picker_event_rx.recv() => {
                    match event {
                        PiecePickerCommand::RequestPiece { bitfield, response_tx } =>{
                            let idx = self.claim_piece(&bitfield);
                            let _ = response_tx.send(idx);
                        },
                    }
                }
            )
        }
    }

    pub fn claim_piece(&mut self, bitfield: &BitField) -> Option<usize> {
        let idx = self.states.iter().enumerate().find_map(|(i, state)| {
            (bitfield.has_piece(i) && *state == PieceState::Missing).then_some(i)
        })?;

        self.states[idx] = PieceState::InProgress;
        Some(idx)
    }

    pub fn mark_as_completed(&mut self, index: usize) {
        self.states[index] = PieceState::Completed;
    }

    pub fn mark_as_failed(&mut self, index: usize) {
        self.states[index] = PieceState::Missing;
    }

    pub fn is_finished(&self) -> bool {
        self.states
            .iter()
            .all(|state| *state == PieceState::Completed)
    }
}
