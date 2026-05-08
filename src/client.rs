use std::{env, path::Path, sync::Arc};

use crate::{torrent_info::TorrentInfo, torrent_session::TorrentSession};
use anyhow::{Result};
use rand::RngExt;

#[derive(Debug, Clone)]
pub struct Client {
    pub peer_id: [u8; 20],
    pub port: u16,
}

impl Client {
    pub fn new() -> Self {
        Self {
            peer_id: generate_client_id(),
            port: 12345,
        }
    }

    pub async fn run(&self) -> Result<()> {
        let args: Vec<_> = env::args().collect();
        if args.len() < 2 {
            panic!("invalid argument count");
        }

        let path = args[1].to_string();
        let path = Path::new(&path);

        let torrent = TorrentInfo::from_file(path).await?;
        let torrent = Arc::new(torrent);

        let session = TorrentSession::new(torrent).await?;
        session.run(self).await?;

        println!("Download completed");

        Ok(())
    }
}

fn generate_client_id() -> [u8; 20] {
    let mut id = [0u8; 20];
    rand::rng().fill(&mut id);
    id
}
