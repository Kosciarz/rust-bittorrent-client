use anyhow::Result;

mod bencode;
mod client;
mod peer;
mod torrent;
mod tracker;

#[tokio::main]
async fn main() -> Result<()> {
    let client = client::Client::new();
    client.run().await
}
