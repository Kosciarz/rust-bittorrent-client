use std::collections::BTreeMap;
use std::vec::Vec;

#[derive(Debug)]
pub enum Object {
    Number(i64),
    ByteArray(Vec<u8>),
    List(Vec<Object>),
    Dictionary(BTreeMap<Vec<u8>, Object>),
}

type Key = String;
type KeyType = String;

#[derive(Debug)]
pub enum ExtractError {
    MissingKey(Key),
    InvalidKey(Key, KeyType),
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
    dict: &'a BTreeMap<Vec<u8>, Object>,
    key: &[u8],
) -> Result<(&'a Object, String), ExtractError> {
    let key_str = String::from_utf8_lossy(key).to_string();
    let obj = dict
        .get(key)
        .ok_or(ExtractError::MissingKey(key_str.clone()))?;
    Ok((obj, key_str))
}

pub fn extract_num(dict: &BTreeMap<Vec<u8>, Object>, key: &[u8]) -> Result<i64, ExtractError> {
    let (obj, key_str) = get_value(dict, key)?;

    match obj {
        Object::Number(num) => Ok(*num),
        _ => Err(ExtractError::InvalidKey(key_str, String::from("number"))),
    }
}

pub fn extract_str(dict: &BTreeMap<Vec<u8>, Object>, key: &[u8]) -> Result<String, ExtractError> {
    let (obj, key_str) = get_value(dict, key)?;

    match obj {
        Object::ByteArray(bytes) => {
            String::from_utf8(bytes.clone()).map_err(|err| ExtractError::InvalidUtf8(err))
        }
        _ => Err(ExtractError::InvalidKey(
            key_str,
            String::from("byte array"),
        )),
    }
}

pub fn extract_dict<'a>(
    dict: &'a BTreeMap<Vec<u8>, Object>,
    key: &[u8],
) -> Result<&'a BTreeMap<Vec<u8>, Object>, ExtractError> {
    let (obj, key_str) = get_value(dict, key)?;

    match obj {
        Object::Dictionary(dict) => Ok(dict),
        _ => Err(ExtractError::InvalidKey(
            key_str,
            String::from("dictionary"),
        )),
    }
}

pub fn extract_pieces<const N: usize>(
    dict: &BTreeMap<Vec<u8>, Object>,
) -> Result<Vec<[u8; N]>, ExtractError> {
    let (obj, key_str) = get_value(dict, b"pieces")?;

    match obj {
        Object::ByteArray(data) => chunk_array::<N>(data),
        _ => Err(ExtractError::InvalidKey(key_str, String::from("string"))),
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
