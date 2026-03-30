use std::{path::Path, str};

mod parser;

fn main() {
    parser::decode_file(Path::new("C:/Users/barto/Downloads/lorem.txt.torrent"));

    let bytes = "i1774648139e".as_bytes();
    let mut iter = bytes.iter().copied().peekable();
    if let parser::Object::Number(num) = parser::decode_number(&mut iter) {
        assert_eq!(num, 1774648139);
    }

    let bytes = "10:created by".as_bytes();
    let mut iter = bytes.iter().copied().peekable();
    if let parser::Object::ByteArray(byte_array) = parser::decode_byte_array(&mut iter) {
        assert_eq!(str::from_utf8(&byte_array), Ok("created by"));
    }
}
