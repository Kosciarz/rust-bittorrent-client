use tokio::sync::Mutex;

use crate::bitfield::BitField;

#[derive(Debug, Clone, PartialEq)]
pub enum PieceState {
    Missing,
    InProgress,
    Completed,
}

#[derive(Debug)]
pub struct PiecePicker {
    states: Mutex<Vec<PieceState>>,
}

impl PiecePicker {
    pub fn new(num_pieces: usize) -> Self {
        Self {
            states: Mutex::new(vec![PieceState::Missing; num_pieces]),
        }
    }

    pub async fn claim_piece(&self, bitfield: &BitField) -> Option<usize> {
        let mut states = self.states.lock().await;

        let idx = states.iter().enumerate().find_map(|(i, state)| {
            (bitfield.has_piece(i) && *state == PieceState::Missing).then_some(i)
        })?;

        states[idx] = PieceState::InProgress;
        Some(idx)
    }

    pub async fn mark_completed(&self, index: usize) {
        let mut states = self.states.lock().await;
        states[index] = PieceState::Completed;
    }

    pub async fn mark_failed(&self, index: usize) {
        let mut states = self.states.lock().await;
        states[index] = PieceState::Missing;
    }

    pub async fn is_finished(&self) -> bool {
        self.states
            .lock()
            .await
            .iter()
            .all(|state| *state == PieceState::Completed)
    }
}
