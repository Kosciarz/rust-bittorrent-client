use anyhow::Result;

mod bencode;
mod client;
mod file_writer;
mod peer;
mod piece;
mod piece_assembler;
mod torrent_info;
mod torrent_session;
mod tracker;

#[tokio::main]
async fn main() -> Result<()> {
    let client = client::Client::new();
    client.run().await
}
