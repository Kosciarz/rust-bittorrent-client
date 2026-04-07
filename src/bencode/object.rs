use std::collections::BTreeMap;

use crate::bencode::decode::Parsed;

#[derive(Debug)]
pub enum Object<'data> {
    Number(i64),
    ByteArray(Vec<u8>),
    List(Vec<Parsed<'data>>),
    Dictionary(BTreeMap<Vec<u8>, Parsed<'data>>),
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

fn get_value<'a>(
    dict: &'a BTreeMap<Vec<u8>, Parsed>,
    key: &[u8],
) -> Result<&'a Parsed<'a>, ExtractError> {
    dict.get(key).ok_or(ExtractError::MissingKey(
        String::from_utf8_lossy(key).to_string(),
    ))
}

pub fn extract_num(dict: &BTreeMap<Vec<u8>, Parsed>, key: &[u8]) -> Result<i64, ExtractError> {
    let value = get_value(dict, key)?;

    match &value.object {
        Object::Number(num) => Ok(*num),
        _ => Err(ExtractError::InvalidKey(
            String::from_utf8_lossy(key).to_string(),
            String::from("number"),
        )),
    }
}

pub fn extract_str(dict: &BTreeMap<Vec<u8>, Parsed>, key: &[u8]) -> Result<String, ExtractError> {
    let value = get_value(dict, key)?;

    match &value.object {
        Object::ByteArray(bytes) => {
            String::from_utf8(bytes.clone()).map_err(|err| ExtractError::InvalidUtf8(err))
        }
        _ => Err(ExtractError::InvalidKey(
            String::from_utf8_lossy(key).to_string(),
            String::from("byte array"),
        )),
    }
}

pub fn extract_dict<'a>(
    dict: &'a BTreeMap<Vec<u8>, Parsed>,
    key: &[u8],
) -> Result<&'a BTreeMap<Vec<u8>, Parsed<'a>>, ExtractError> {
    let value = get_value(dict, key)?;

    match &value.object {
        Object::Dictionary(d) => Ok(d),
        _ => Err(ExtractError::InvalidKey(
            String::from_utf8_lossy(key).to_string(),
            String::from("dictionary"),
        )),
    }
}

pub fn extract_pieces<const N: usize>(
    dict: &BTreeMap<Vec<u8>, Parsed>,
) -> Result<Vec<[u8; N]>, ExtractError> {
    let value = get_value(dict, b"pieces")?;

    match &value.object {
        Object::ByteArray(b) => chunk_array::<N>(b),
        _ => Err(ExtractError::InvalidKey(
            String::from_utf8_lossy(b"pieces").to_string(),
            String::from("string"),
        )),
    }
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
