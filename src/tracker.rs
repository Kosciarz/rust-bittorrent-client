use std::{error::Error, time::Duration};

use crate::{
    bencode::{
        ObjectType, decode_object,
        object::{extract_byte_array, extract_num},
    },
    peer::Peer,
};

pub struct Tracker {
    url: String,
    interval: Duration,
    peers: Vec<Peer>,
}

#[derive(Debug)]
pub struct AnnounceInfo<'a> {
    info_hash: &'a [u8],
    client_id: &'a [u8; 20],
    peer_port: u16,
    downloaded: u64,
    left: u64,
    uploaded: u64,
}

impl<'a> AnnounceInfo<'a> {
    pub fn new(
        info_hash: &'a [u8],
        client_id: &'a [u8; 20],
        peer_port: u16,
        downloaded: u64,
        left: u64,
        uploaded: u64,
    ) -> Self {
        Self {
            info_hash,
            client_id,
            peer_port,
            downloaded,
            left,
            uploaded,
        }
    }
}

#[derive(Debug)]
pub struct AnnounceResult {
    complete: i64,
    incomplete: i64,
    interval: i64,
    peers: Vec<u8>,
}

impl Tracker {
    pub fn new(url: String) -> Self {
        Self {
            url,
            interval: Duration::ZERO,
            peers: Vec::new(),
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn interval(&self) -> Duration {
        self.interval
    }

    pub fn set_interval(&mut self, interval: Duration) {
        self.interval = interval;
    }

    pub fn peers(&self) -> &Vec<Peer> {
        &self.peers
    }

    pub fn announce(&mut self, announce_info: &AnnounceInfo) -> Result<(), Box<dyn Error>> {
        let url_encoded_info_hash = urlencoding::encode_binary(announce_info.info_hash);
        let url_encoded_client_id = urlencoding::encode_binary(announce_info.client_id);

        let url = format!(
            "{}?info_hash={}&peer_id={}&port={}&uploaded={}&downloaded={}&left={}&compact=1",
            self.url,
            url_encoded_info_hash,
            url_encoded_client_id,
            announce_info.peer_port,
            announce_info.uploaded,
            announce_info.downloaded,
            announce_info.left,
        );

        let announce_result = self.send_announce_request(&url)?;

        println!(
            "Complete: {}\nIncomplete: {}\nInterval: {}\nPeers: {:?}",
            announce_result.complete,
            announce_result.incomplete,
            announce_result.interval,
            announce_result.peers
        );

        self.set_interval(Duration::from_secs(u64::try_from(
            announce_result.interval,
        )?));

        let peer = Peer::from_bytes(&announce_result.peers);
        self.peers.push(peer);

        for peer in &mut self.peers {
            peer.connect(announce_info.info_hash, announce_info.client_id)?;
        }

        Ok(())
    }

    fn send_announce_request(&self, url: &str) -> Result<AnnounceResult, Box<dyn Error>> {
        let res = reqwest::blocking::get(url)?.bytes()?;
        let obj = decode_object(&res);

        println!("Announce result: {:?}", &res);

        match obj.object_type() {
            ObjectType::Dictionary(d) => {
                let complete = extract_num(&d, b"complete")?;
                let incomplete = extract_num(&d, b"incomplete")?;
                let interval = extract_num(&d, b"interval")?;
                let peers = extract_byte_array(&d, b"peers")?;

                Ok(AnnounceResult {
                    complete,
                    incomplete,
                    interval,
                    peers,
                })
            }
            _ => Err("Expected a dictionary".into()),
        }
    }
}

impl std::fmt::Debug for Tracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tracker")
            .field("address", &self.url)
            .finish()
    }
}
