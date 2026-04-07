use std::{env, error::Error, fs, path::Path};

use crate::{bencode::encode_object, torrent::Torrent};

mod bencode;
mod torrent;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() < 2 {
        panic!("Invalid argument count");
    }

    let path = args[1].to_string();
    let path = Path::new(&path);

    let bytes = fs::read(path)?;
    let parsed = bencode::decode_object(&bytes);

    assert_eq!(encode_object(&parsed), bytes);

    let torrent = Torrent::try_from(parsed)?;

    println!(
        "Name: {}\nLength: {}\nAnnounce: {}\nPiece length: {}\nInfo hash: {:?}",
        torrent.name, torrent.length, torrent.announce, torrent.piece_length, torrent.info_hash
    );

    Ok(())
}
