use std::{collections::BTreeMap, error::Error, fs, path::Path};

use sha1::{Digest, Sha1};

use crate::bencode::{
    self, ExtractError, Object, ObjectType, decode_object,
    object::{self, extract_byte_array, extract_dict, extract_list, extract_num, extract_str},
};

pub struct Tracker {
    pub address: String,
}

impl Tracker {
    fn new(address: String) -> Self {
        Self { address }
    }
}

impl std::fmt::Debug for Tracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tracker")
            .field("address", &self.address)
            .finish()
    }
}

pub struct Torrent {
    announce: Tracker,
    announce_list: Vec<Tracker>,
    name: String,
    length: u64,
    piece_length: u64,
    pieces: Vec<[u8; 20]>,
    info_hash: [u8; 20],
}

impl Torrent {
    fn new(
        tracker: Tracker,
        announce_list: Vec<Tracker>,
        name: String,
        length: u64,
        piece_length: u64,
        pieces: Vec<[u8; 20]>,
        info_hash: [u8; 20],
    ) -> Self {
        Self {
            announce: tracker,
            announce_list,
            name,
            length,
            piece_length,
            pieces,
            info_hash,
        }
    }

    pub fn announce(&self) -> &Tracker {
        &self.announce
    }

    pub fn announce_list(&self) -> &Vec<Tracker> {
        &self.announce_list
    }

    pub fn name(&self) -> &String {
        &self.name
    }

    pub fn length(&self) -> u64 {
        self.length
    }

    pub fn piece_length(&self) -> u64 {
        self.piece_length
    }

    pub fn pieces(&self) -> &Vec<[u8; 20]> {
        &self.pieces
    }

    pub fn info_hash(&self) -> &[u8; 20] {
        &self.info_hash
    }
}

impl TryFrom<Object> for Torrent {
    type Error = Box<dyn std::error::Error>;

    fn try_from(object: Object) -> Result<Self, Self::Error> {
        let dict = match object.object_type() {
            ObjectType::Dictionary(d) => d,
            _ => return Err("Top level object is not a dictionary".into()),
        };

        let announce = Tracker::new(extract_str(&dict, b"announce")?);
        let announce_list = extract_announce_list(&dict)?;

        let info_obj = extract_dict(&dict, b"info")?;
        let name = extract_str(&info_obj, b"name")?;
        let length = u64::try_from(extract_num(&info_obj, b"length")?)
            .map_err(|_| "length is negative or too large")?;
        let piece_length = u64::try_from(extract_num(&info_obj, b"piece length")?)
            .map_err(|_| "piece length is negative or too large")?;
        let pieces = extract_pieces(&info_obj)?;

        let info_parsed = dict
            .get(b"info".as_slice())
            .ok_or(ExtractError::MissingKey("info".into()))?;
        let info_hash = compute_info_hash(&info_parsed);

        Ok(Torrent::new(
            announce,
            announce_list,
            name,
            length,
            piece_length,
            pieces,
            info_hash,
        ))
    }
}

fn extract_announce_list(dict: &BTreeMap<Vec<u8>, Object>) -> Result<Vec<Tracker>, ExtractError> {
    let tiers = extract_list(dict, b"announce-list")?;

    let mut trackers = Vec::new();

    for tier in tiers {
        let list = match tier.object_type() {
            ObjectType::List(l) => l,
            _ => {
                return Err(ExtractError::InvalidKey(
                    "announce-list".into(),
                    "list".into(),
                ));
            }
        };

        for obj in list {
            let bytes = match obj.object_type() {
                ObjectType::ByteArray(b) => b,
                _ => {
                    return Err(ExtractError::InvalidKey(
                        "announce-list".into(),
                        "byte string".into(),
                    ));
                }
            };

            let url =
                String::from_utf8(bytes.to_vec()).map_err(|err| ExtractError::InvalidUtf8(err))?;

            trackers.push(Tracker::new(url));
        }
    }

    Ok(trackers)
}

fn compute_info_hash(info_parsed: &Object) -> [u8; 20] {
    Sha1::digest(&info_parsed.bytes()).into()
}

fn extract_pieces(info_dict: &BTreeMap<Vec<u8>, Object>) -> Result<Vec<[u8; 20]>, ExtractError> {
    let arr = extract_byte_array(info_dict, b"pieces")?;
    chunk_array::<20>(&arr)
}

fn chunk_array<const N: usize>(data: &[u8]) -> Result<Vec<[u8; N]>, ExtractError> {
    if data.len() % N != 0 {
        return Err(ExtractError::InvalidPiecesLength(data.len(), N));
    }

    let mut result = Vec::with_capacity(data.len() / N);

    for chunk in data.chunks(N) {
        let mut arr = [0u8; N];
        arr.copy_from_slice(chunk);
        result.push(arr);
    }

    Ok(result)
}
