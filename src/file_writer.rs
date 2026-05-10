use tokio::{
    fs::File,
    io::{self, AsyncSeekExt, AsyncWriteExt},
    sync::mpsc,
};

use anyhow::Result;

use crate::{piece::CompletedPiece, piece_picker::PieceEvent};

#[derive(Debug)]
pub struct FileWriter {
    piece_length: u32,

    file: File,
    completed_piece_rx: mpsc::Receiver<CompletedPiece>,
    piece_event_tx: mpsc::Sender<PieceEvent>,
}

impl FileWriter {
    pub async fn new(
        torrent_length: u64,
        name: String,
        piece_length: u32,
        completed_piece_rx: mpsc::Receiver<CompletedPiece>,
        piece_event_tx: mpsc::Sender<PieceEvent>,
    ) -> Result<Self> {
        let file = File::options()
            .create(true)
            .write(true)
            .read(true)
            .open(name)
            .await?;

        file.set_len(torrent_length).await?;

        Ok(Self {
            piece_length,
            file,
            completed_piece_rx,
            piece_event_tx,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        while let Some(completed) = self.completed_piece_rx.recv().await {
            let offset = (completed.index as u64) * (self.piece_length as u64);
            self.file.seek(io::SeekFrom::Start(offset)).await?;

            self.file.write_all(&completed.data).await?;
            self.file.flush().await?;

            let _ = self.piece_event_tx.send(PieceEvent::Completed { piece_index: completed.index }).await;
        }

        Ok(())
    }
}
