use std::collections::BTreeMap;

use crate::torrent::Torrent;

type Result<T> = std::result::Result<T, ExtractError>;

#[derive(Debug)]
pub enum ObjectType {
    Number(i64),
    ByteArray(Vec<u8>),
    List(Vec<Object>),
    Dictionary(BTreeMap<Vec<u8>, Object>),
}

#[derive(Debug)]
pub struct Object {
    object_type: ObjectType,
    bytes: Vec<u8>,
}

impl Object {
    pub fn new(object_type: ObjectType, bytes: Vec<u8>) -> Self {
        Self { object_type, bytes }
    }

    pub fn object_type(&self) -> &ObjectType {
        &self.object_type
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl From<&Torrent> for Object {
    fn from(torrent: &Torrent) -> Self {
        let mut dict = BTreeMap::new();

        dict.insert(
            b"announce".to_vec(),
            Object::new(
                ObjectType::ByteArray(torrent.announce().address().as_bytes().to_vec()),
                Vec::new(),
            ),
        );
        dict.insert(
            b"announce-list".to_vec(),
            Object::new(convert_announce_list(torrent), Vec::new()),
        );
        dict.insert(
            b"comment".to_vec(),
            Object::new(
                ObjectType::ByteArray(torrent.comment().as_bytes().to_vec()),
                Vec::new(),
            ),
        );
        dict.insert(
            b"created by".to_vec(),
            Object::new(
                ObjectType::ByteArray(torrent.created_by().as_bytes().to_vec()),
                Vec::new(),
            ),
        );
        dict.insert(
            b"creation date".to_vec(),
            Object::new(
                ObjectType::Number(torrent.creation_date() as i64),
                Vec::new(),
            ),
        );
        dict.insert(
            b"info".to_vec(),
            Object::new(convert_info_dictionary(torrent), Vec::new()),
        );

        Object::new(ObjectType::Dictionary(dict), Vec::new())
    }
}

fn convert_announce_list(torrent: &Torrent) -> ObjectType {
    let mut announce_list = Vec::new();

    for tracker in torrent.announce_list() {
        announce_list.push(Object::new(
            ObjectType::List(vec![Object::new(
                ObjectType::ByteArray(tracker.address().as_bytes().to_vec()),
                Vec::new(),
            )]),
            Vec::new(),
        ))
    }

    ObjectType::List(announce_list)
}

fn convert_info_dictionary(torrent: &Torrent) -> ObjectType {
    let mut dict = BTreeMap::new();

    dict.insert(
        b"length".to_vec(),
        Object::new(ObjectType::Number(torrent.length() as i64), Vec::new()),
    );
    dict.insert(
        b"name".to_vec(),
        Object::new(
            ObjectType::ByteArray(torrent.name().as_bytes().to_vec()),
            Vec::new(),
        ),
    );

    dict.insert(
        b"piece length".to_vec(),
        Object::new(
            ObjectType::Number(torrent.piece_length() as i64),
            Vec::new(),
        ),
    );

    let mut pieces: Vec<u8> = Vec::with_capacity(torrent.pieces().len() * 20);
    for piece in torrent.pieces() {
        pieces.extend_from_slice(piece);
    }

    dict.insert(
        b"pieces".to_vec(),
        Object::new(ObjectType::ByteArray(pieces), Vec::new()),
    );

    ObjectType::Dictionary(dict)
}

#[derive(Debug)]
pub enum ExtractError {
    MissingKey(String),
    InvalidKey(String, String),
    InvalidUtf8(std::string::FromUtf8Error),
    InvalidPiecesLength(usize, usize),
}

impl std::fmt::Display for ExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            ExtractError::MissingKey(key) => write!(f, "Missing key '{}'", key),
            ExtractError::InvalidKey(key, key_type) => {
                write!(f, "Key '{}' is not a {}", key, key_type)
            }
            ExtractError::InvalidUtf8(err) => write!(f, "{}", err),
            ExtractError::InvalidPiecesLength(length, multiple) => write!(
                f,
                "Input length ({}) is not a multiple of {}",
                length, multiple
            ),
        }
    }
}

impl std::error::Error for ExtractError {}

impl From<std::string::FromUtf8Error> for ExtractError {
    fn from(err: std::string::FromUtf8Error) -> ExtractError {
        ExtractError::InvalidUtf8(err)
    }
}

fn get_value<'a>(dict: &'a BTreeMap<Vec<u8>, Object>, key: &[u8]) -> Result<&'a Object> {
    dict.get(key).ok_or(ExtractError::MissingKey(
        String::from_utf8_lossy(key).to_string(),
    ))
}

pub fn extract_num(dict: &BTreeMap<Vec<u8>, Object>, key: &[u8]) -> Result<i64> {
    let value = get_value(dict, key)?;

    match value.object_type() {
        ObjectType::Number(num) => Ok(*num),
        _ => Err(ExtractError::InvalidKey(
            String::from_utf8_lossy(key).to_string(),
            "number".into(),
        )),
    }
}

pub fn extract_byte_array(dict: &BTreeMap<Vec<u8>, Object>, key: &[u8]) -> Result<Vec<u8>> {
    let value = get_value(dict, key)?;

    match value.object_type() {
        ObjectType::ByteArray(b) => Ok(b.to_vec()),
        _ => Err(ExtractError::InvalidKey(
            String::from_utf8_lossy(key).to_string(),
            "byte array".into(),
        )),
    }
}

pub fn extract_str(dict: &BTreeMap<Vec<u8>, Object>, key: &[u8]) -> Result<String> {
    String::from_utf8(extract_byte_array(dict, key)?).map_err(|err| ExtractError::InvalidUtf8(err))
}

pub fn extract_list<'a>(
    dict: &'a BTreeMap<Vec<u8>, Object>,
    key: &[u8],
) -> Result<&'a Vec<Object>> {
    let value = get_value(dict, key)?;

    match value.object_type() {
        ObjectType::List(l) => Ok(l),
        _ => Err(ExtractError::InvalidKey(
            String::from_utf8_lossy(key).to_string(),
            "list".into(),
        )),
    }
}

pub fn extract_dict<'a>(
    dict: &'a BTreeMap<Vec<u8>, Object>,
    key: &[u8],
) -> Result<&'a BTreeMap<Vec<u8>, Object>> {
    let value = get_value(dict, key)?;

    match value.object_type() {
        ObjectType::Dictionary(d) => Ok(d),
        _ => Err(ExtractError::InvalidKey(
            String::from_utf8_lossy(key).to_string(),
            "dictionary".into(),
        )),
    }
}
