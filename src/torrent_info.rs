use std::{collections::BTreeMap, fs, io, path::Path};

use anyhow::{Context, Result, anyhow, bail};
use sha1::{Digest, Sha1};
use url::Url;

use crate::{
    bencode::{
        self, Object, ObjectType, decode_object,
        object::{extract_byte_array, extract_dict, extract_list, extract_num, extract_str},
    },
    piece::PieceInfo,
};

#[derive(Debug, Clone)]
pub struct TorrentInfo {
    // core download fields
    pub length: u64,
    pub info_hash: [u8; 20],
    pub piece_length: u32,
    pub pieces: Vec<PieceInfo>,

    // metadata (only for serialization/display)
    pub announce: Url,
    pub announce_list: Vec<Vec<Url>>,
    pub name: String,
    pub comment: String,
    pub created_by: String,
    pub creation_date: u64,
}

impl TorrentInfo {
    pub fn piece_hashes(&self) -> Vec<[u8; 20]> {
        self.pieces.iter().map(|p| p.hash).collect()
    }

    pub async fn from_file(path: &Path) -> Result<TorrentInfo> {
        let bytes = fs::read(path)?;
        let obj = decode_object(&bytes);
        TorrentInfo::from_object(obj).await
    }

    pub async fn save_to_file(&self, path: &Path) -> io::Result<()> {
        let obj = Object::from_torrent(self);
        let bytes = bencode::encode_object(&obj);
        tokio::fs::write(
            format!(
                "{}/{}.torrent",
                path.to_string_lossy().to_string(),
                self.name
            ),
            bytes,
        )
        .await?;
        Ok(())
    }

    async fn from_object(object: Object) -> Result<Self> {
        let dict = match object.object_type() {
            ObjectType::Dictionary(d) => d,
            _ => bail!("top level object is not a dictionary"),
        };

        let announce =
            Url::parse(&extract_str(&dict, b"announce")?).context("invalid announce URL")?;
        let announce_list = extract_announce_list(&dict)?;
        let comment = extract_str(&dict, b"comment")?;
        let created_by = extract_str(&dict, b"created by")?;
        let creation_date = u64::try_from(extract_num(&dict, b"creation date")?)
            .map_err(|_| anyhow!("creation date is negative or too large"))?;

        let info_obj = extract_dict(&dict, b"info")?;
        let name = extract_str(&info_obj, b"name")?;
        let total_length = u64::try_from(extract_num(&info_obj, b"length")?)
            .map_err(|_| anyhow!("length is negative or too large"))?;
        let piece_length = u32::try_from(extract_num(&info_obj, b"piece length")?)
            .map_err(|_| anyhow!("piece length is negative or too large"))?;
        let piece_hashes = extract_pieces(&info_obj)?;

        let info_hash = compute_info_hash(&dict)?;

        let mut pieces = Vec::with_capacity(piece_hashes.len());
        for (i, hash) in piece_hashes.iter().enumerate() {
            let length = if i == piece_hashes.len() - 1 {
                let last_piece_length = total_length - ((piece_length as u64) * (i as u64));
                assert!(
                    last_piece_length > 0 && last_piece_length <= piece_length as u64,
                    "last piece length {last_piece_length} is out of range"
                );
                last_piece_length as u32
            } else {
                piece_length as u32
            };

            pieces.push(PieceInfo {
                index: i,
                length,
                hash: *hash,
            });
        }

        Ok(TorrentInfo {
            info_hash,
            pieces: pieces.clone(),
            piece_length,
            length: total_length,
            name,
            announce,
            announce_list,
            comment,
            created_by,
            creation_date,
        })
    }
}

fn extract_announce_list(dict: &BTreeMap<Vec<u8>, Object>) -> Result<Vec<Vec<Url>>> {
    let tiers = extract_list(dict, b"announce-list")?;

    let mut announce_list = Vec::new();

    for tier in tiers {
        let mut trackers = Vec::new();

        let list = match tier.object_type() {
            ObjectType::List(l) => l,
            _ => bail!("expected key announce-list to be of type list"),
        };

        for obj in list {
            let bytes = match obj.object_type() {
                ObjectType::ByteArray(b) => b,
                _ => bail!("Expected key announce-list to be byte string",),
            };

            let url = String::from_utf8(bytes.to_vec())?;
            trackers.push(Url::parse(&url)?);
        }

        announce_list.push(trackers);
    }

    Ok(announce_list)
}

fn compute_info_hash(dict: &BTreeMap<Vec<u8>, Object>) -> Result<[u8; 20]> {
    let info = dict
        .get(b"info".as_slice())
        .ok_or(anyhow!("missing key info"))?;
    Ok(Sha1::digest(&info.bytes()).into())
}

fn extract_pieces(info_dict: &BTreeMap<Vec<u8>, Object>) -> Result<Vec<[u8; 20]>> {
    let arr = extract_byte_array(info_dict, b"pieces")?;
    chunk_array::<20>(&arr)
}

fn chunk_array<const N: usize>(data: &[u8]) -> Result<Vec<[u8; N]>> {
    if data.len() % N != 0 {
        return Err(anyhow!("length {} is not a mupliple of {N}", data.len()));
    }

    let mut result = Vec::with_capacity(data.len() / N);

    for chunk in data.chunks(N) {
        let mut arr = [0u8; N];
        arr.copy_from_slice(chunk);
        result.push(arr);
    }

    Ok(result)
}
