pub mod bencoding;

use bencoding::{decode, find_info_slice, info_hash};
use std::io::{self, Read};

pub fn parse_torrent(path: &str) -> io::Result<()> {
    match std::fs::File::open(path) {
        Ok(mut file) => {
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)?;

            let mut pos = 0;
            let root = decode(&buf, &mut pos)?;

            let info_bytes = find_info_slice(&buf)?;
            let hash = info_hash(info_bytes);

            println!("Parsed torrent:");
            println!("info_hash: {:x?}", hash);
            println!("{:#?}", root);

            Ok(())
        }
        Err(_) => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Could not open the specified file",
        )),
    }
}
