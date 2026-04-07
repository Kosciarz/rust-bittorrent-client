use sha1::{Digest, Sha1};

use crate::bencode::{Object, decode::Parsed, extract_dict, extract_num, extract_pieces, extract_str, object::ExtractError};

#[derive(Debug)]
pub struct Torrent {
    pub announce: String,
    pub name: String,
    pub length: u64,
    pub piece_length: u64,
    pub pieces: Vec<[u8; 20]>,
    pub info_hash: [u8; 20],
}

impl<'a> TryFrom<Parsed<'a>> for Torrent {
    type Error = Box<dyn std::error::Error>;

    fn try_from(parsed: Parsed) -> Result<Self, Self::Error> {
        let dict = match parsed.object {
            Object::Dictionary(d) => d,
            _ => return Err("Top level object is not a dictionary".into()),
        };

        let announce = extract_str(&dict, b"announce")?;

        let info_obj = extract_dict(&dict, b"info")?;
        let name = extract_str(&info_obj, b"name")?;
        let length = u64::try_from(extract_num(&info_obj, b"length")?)
            .map_err(|_| "length is negative or too large")?;
        let piece_length = u64::try_from(extract_num(&info_obj, b"piece length")?)
            .map_err(|_| "piece length is negative or too large")?;
        let pieces = extract_pieces::<20>(&info_obj)?;

        let info_parsed = dict.get(b"info".as_slice()).ok_or(ExtractError::MissingKey("info".into()))?;
        let info_hash = compute_info_hash(&info_parsed);

        Ok(Torrent {
            announce,
            name,
            length,
            piece_length,
            pieces,
            info_hash,
        })
    }
}

pub fn compute_info_hash(info_parsed: &Parsed) -> [u8; 20] {
    Sha1::digest(&info_parsed.data).into()
}
