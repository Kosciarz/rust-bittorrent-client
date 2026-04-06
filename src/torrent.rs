use std::collections::BTreeMap;

use crate::bencode::{
    Object, extract_dict, extract_num, extract_pieces, extract_str, object::ExtractError,
};

#[derive(Debug)]
pub struct Torrent {
    announce: String,
    name: String,
    length: u64,
    piece_length: u64,
    pieces: Vec<[u8; 20]>,
    info_hash: [u8; 20],
}

impl TryFrom<Object> for Torrent {
    type Error = Box<dyn std::error::Error>;

    fn try_from(obj: Object) -> Result<Self, Self::Error> {
        let dict = match obj {
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

        let info_hash = compute_info_hash(info_obj)?;

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

fn compute_info_hash(info_obj: &BTreeMap<Vec<u8>, Object>) -> Result<[u8; 20], String> {
    Ok([0u8; 20])
}

// impl Torrent {
//     fn from_bencoding_object(object: &Object) -> Self {
//         let mut result = Torrent;

//         match object {
//             Object::Dictionary(dictionary) => {
//                 result.announce = if dictionary.contains_key("announce".as_bytes()) {
//                     if let Object::ByteArray(announce) = &dictionary["announce".as_bytes()] {
//                         String::from_utf8(announce.clone()).unwrap()
//                     } else {
//                         panic!()
//                     }
//                 } else {
//                     panic!("Missing announce");
//                 };

//                 if dictionary.contains_key("info".as_bytes()) {
//                     match dictionary["info".as_bytes()] {
//                         Object::Dictionary(info) => {
//                             let length = if info.contains_key("length".as_bytes()) {
//                                 info["length".as_bytes()]
//                             } else {
//                                 panic!("Missing length");
//                             };

//                             let name = if info.contains_key("name".as_bytes()) {
//                                 info["name".as_bytes()]
//                             } else {
//                                 panic!("Missing name")
//                             };

//                             let piece_length = if info.contains_key("piece_length".as_bytes()) {
//                                 info["piece_length".as_bytes()]
//                             } else {
//                                 panic!("Missing piece length");
//                             };
//                         }
//                         _ => panic!("Missing info section"),
//                     }
//                 } else {
//                     panic!("Missing info section");
//                 };

//                 todo!()
//             }
//             _ => panic!("Unreachable code"),
//         }
//     }
// }
