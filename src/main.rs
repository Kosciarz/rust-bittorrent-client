use std::{env, error::Error, path::Path};

use crate::torrent::Torrent;

mod bencode;
mod torrent;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() < 2 {
        panic!("Invalid argument count");
    }

    let path = args[1].to_string();
    let path = Path::new(&path);
    let torrent = Torrent::load_from_file(path)?;

    println!(
        "Name: {}\nLength: {}\nAnnounce: {:?}\nAnnounce list: {:?}\nPiece length: {}\nInfo hash: {:?}",
        torrent.name(),
        torrent.length(),
        torrent.announce(),
        torrent.announce_list(),
        torrent.piece_length(),
        torrent.info_hash()
    );

    let path = args[2].to_string();
    let path = Path::new(&path);
    Torrent::save_to_file(&torrent, &path)?;

    Ok(())
}
