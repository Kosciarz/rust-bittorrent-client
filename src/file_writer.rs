use tokio::{
    fs::File,
    io::{self, AsyncSeekExt, AsyncWriteExt},
    sync::mpsc,
};

use anyhow::Result;

use crate::piece::CompletedPiece;

#[derive(Debug)]
pub struct FileWriter {
    piece_length: u32,

    rx: mpsc::Receiver<CompletedPiece>,
    file: File,
}

impl FileWriter {
    pub async fn new(
        torrent_length: u64,
        name: String,
        piece_length: u32,
        file_rx: mpsc::Receiver<CompletedPiece>,
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
            rx: file_rx,
            file,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        while let Some(completed) = self.rx.recv().await {
            let offset = (completed.index as u64) * (self.piece_length as u64);
            self.file.seek(io::SeekFrom::Start(offset)).await?;

            self.file.write_all(&completed.data).await?;
            self.file.flush().await?;
        }

        Ok(())
    }
}
