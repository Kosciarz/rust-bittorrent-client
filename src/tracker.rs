use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use url::Url;

use crate::bencode::{
    ObjectType, decode_object,
    object::{extract_byte_array, extract_num},
};

#[derive(Debug)]
pub struct AnnounceStats {
    pub uploaded: u64,
    pub downloaded: u64,
    pub left: u64,
}

#[derive(Debug)]
pub struct Tracker {
    url: Url,
    interval: Mutex<Duration>,
    last_announce: Mutex<Option<Instant>>,
}

impl Tracker {
    pub fn new(url: Url) -> Self {
        Self {
            url,
            interval: Mutex::new(Duration::from_secs(1800)),
            last_announce: Mutex::new(None),
        }
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn interval(&self) -> Duration {
        *self.interval.lock().unwrap()
    }

    pub fn is_due(&self) -> bool {
        let last_announce = self.last_announce.lock().unwrap();
        let interval = self.interval.lock().unwrap();
        match *last_announce {
            Some(l) => l + *interval <= Instant::now(),
            None => true,
        }
    }

    pub async fn announce(
        &self,
        info_hash: &[u8; 20],
        peer_id: &[u8; 20],
        port: u16,
        stats: &AnnounceStats,
    ) -> Result<Vec<SocketAddr>> {
        let url = self.build_announce_url(info_hash, peer_id, port, stats);
        let res = reqwest::get(url).await?.bytes().await?;
        let obj = decode_object(&res);

        let dict = match obj.object_type() {
            ObjectType::Dictionary(d) => d,
            _ => return Err(anyhow!("Expected a dictionary")),
        };

        *self.interval.lock().unwrap() =
            Duration::from_secs(extract_num(&dict, b"interval")? as u64);
        *self.last_announce.lock().unwrap() = Some(Instant::now());

        let peers_bytes = extract_byte_array(&dict, b"peers")?;

        Ok(Self::parse_compact_peers(&peers_bytes))
    }

    fn build_announce_url(
        &self,
        info_hash: &[u8; 20],
        peer_id: &[u8; 20],
        port: u16,
        stats: &AnnounceStats,
    ) -> Url {
        let info_hash = urlencoding::encode_binary(info_hash);
        let peer_id = urlencoding::encode_binary(peer_id);

        let mut url = self.url.clone();
        url.query_pairs_mut()
            .append_pair("port", &port.to_string())
            .append_pair("uploaded", &stats.uploaded.to_string())
            .append_pair("downloaded", &stats.downloaded.to_string())
            .append_pair("left", &stats.left.to_string())
            .append_pair("compact", "1");

        let query = format!(
            "{}&info_hash={}&peer_id={}",
            url.query().unwrap_or(""),
            info_hash,
            peer_id
        );
        url.set_query(Some(&query));
        url
    }

    fn parse_compact_peers(bytes: &[u8]) -> Vec<SocketAddr> {
        bytes
            .chunks_exact(6)
            .map(|chunk| {
                let ip = Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3]);
                let port = u16::from_be_bytes([chunk[4], chunk[5]]);
                SocketAddr::from((ip, port))
            })
            .collect()
    }
}
