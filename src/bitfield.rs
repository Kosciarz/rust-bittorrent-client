#[derive(Debug, Clone)]
pub struct BitField {
    pub bytes: Vec<u8>,
    pub num_pieces: usize,
}

impl BitField {
    pub fn new(bytes: Vec<u8>, num_pieces: usize) -> Self {
        Self { bytes, num_pieces }
    }

    pub fn empty(num_pieces: usize) -> Self {
        Self {
            bytes: vec![0u8; (num_pieces + 7) / 8],
            num_pieces,
        }
    }

    pub fn has_piece(&self, index: usize) -> bool {
        if index >= self.num_pieces {
            return false;
        }

        let byte = index / 8;
        let bit = 7 - (index % 8);
        self.bytes[byte] & (1 << bit) != 0
    }

    pub fn set_piece(&mut self, index: usize) {
        if index >= self.num_pieces {
            return;
        }
        let byte = index / 8;
        let bit = 7 - (index % 8);
        self.bytes[byte] |= 1 << bit;
    }
}
