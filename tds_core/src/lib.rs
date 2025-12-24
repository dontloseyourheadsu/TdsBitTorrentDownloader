pub mod bencoding;

use bencoding::{Bencode, decode, find_info_slice, info_hash};
use std::io::{self, Read};

#[derive(Debug)]
pub struct Torrent {
    pub announce: String,
    pub info_hash: [u8; 20],
}

pub fn parse_torrent(path: &str) -> io::Result<Torrent> {
    match std::fs::File::open(path) {
        Ok(mut file) => {
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)?;

            let mut pos = 0;
            let root = decode(&buf, &mut pos)?;

            let info_bytes = find_info_slice(&buf)?;
            let hash = info_hash(info_bytes);

            let announce = if let Bencode::Dict(ref dict) = root {
                match dict.get(&b"announce"[..]) {
                    Some(Bencode::Bytes(bytes)) => String::from_utf8_lossy(bytes).to_string(),
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "Missing or invalid announce URL",
                        ));
                    }
                }
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Torrent file root is not a dictionary",
                ));
            };

            Ok(Torrent {
                announce,
                info_hash: hash,
            })
        }
        Err(_) => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Could not open the specified file",
        )),
    }
}
