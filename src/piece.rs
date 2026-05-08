use sha1::{Digest, Sha1};

pub const BLOCK_SIZE: u32 = 16_384;

#[derive(Debug, Clone, PartialEq)]
pub enum PieceState {
    Missing,
    InProgress,
    Done,
}

#[derive(Debug, Clone)]
pub struct PieceInfo {
    pub index: usize,
    pub length: u32,
    pub hash: [u8; 20],
}

#[derive(Debug, Clone)]
pub struct ActivePiece {
    pub index: usize,
    pub length: u32,
    pub data: Vec<u8>,
}

impl ActivePiece {
    pub fn verify(&self, piece_hash: &[u8; 20]) -> bool {
        let hash: [u8; 20] = Sha1::digest(&self.data).into();
        hash == *piece_hash
    }
}

#[derive(Debug, Clone)]
pub struct CompletedPiece {
    pub index: usize,
    pub data: Vec<u8>,
}
