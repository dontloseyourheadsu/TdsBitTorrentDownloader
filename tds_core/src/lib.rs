pub mod bencoding;
pub mod rate_limit;

use bencoding::{Bencode, decode, find_info_slice, info_hash};
pub use rate_limit::TokenBucket;
use std::io::{self, Read};

#[derive(Debug, Clone)]
pub struct FileInfo {
    pub length: u64,
    pub path: Vec<String>,
}

#[derive(Debug)]
pub struct Torrent {
    pub announce: String,
    pub announce_list: Option<Vec<Vec<String>>>,
    pub info_hash: [u8; 20],
    pub piece_length: u64,
    pub pieces: Vec<[u8; 20]>,
    pub name: String,
    pub length: Option<u64>,
    pub files: Option<Vec<FileInfo>>,
}

pub fn parse_torrent(path: &str) -> io::Result<Torrent> {
    match std::fs::File::open(path) {
        Ok(mut file) => {
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)?;
            parse_torrent_from_bytes(&buf)
        }
        Err(e) => Err(e),
    }
}

pub fn parse_torrent_from_bytes(buf: &[u8]) -> io::Result<Torrent> {
    let mut pos = 0;
    let root = decode(buf, &mut pos)?;

    let info_bytes = find_info_slice(buf)?;
    let hash = info_hash(info_bytes);

    if let Bencode::Dict(ref dict) = root {
        let announce = match dict.get(&b"announce"[..]) {
            Some(Bencode::Bytes(bytes)) => String::from_utf8_lossy(bytes).to_string(),
            _ => String::new(), // Handle missing announce for magnet links later or return error?
                                // For now, let's allow empty announce if we want, or default.
                                // But original code enforced it.
                                // Let's keep strictness or make it optional?
                                // If I construct the bytes in magnet resolver, I can provide a dummy announce.
        };

        let announce_list = if let Some(Bencode::List(list)) = dict.get(&b"announce-list"[..]) {
            let mut tiers = Vec::new();
            for tier in list {
                if let Bencode::List(urls) = tier {
                    let mut tier_urls = Vec::new();
                    for url in urls {
                        if let Bencode::Bytes(bytes) = url {
                            tier_urls.push(String::from_utf8_lossy(bytes).to_string());
                        }
                    }
                    if !tier_urls.is_empty() {
                        tiers.push(tier_urls);
                    }
                }
            }
            if tiers.is_empty() { None } else { Some(tiers) }
        } else {
            None
        };

        let info_dict = match dict.get(&b"info"[..]) {
            Some(Bencode::Dict(d)) => d,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Missing info dictionary",
                ));
            }
        };

        let name = match info_dict.get(&b"name"[..]) {
            Some(Bencode::Bytes(b)) => String::from_utf8_lossy(b).to_string(),
            _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "Missing name")),
        };

        let piece_length = match info_dict.get(&b"piece length"[..]) {
            Some(Bencode::Int(i)) => *i as u64,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Missing piece length",
                ));
            }
        };

        let pieces_bytes = match info_dict.get(&b"pieces"[..]) {
            Some(Bencode::Bytes(b)) => b,
            _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "Missing pieces")),
        };

        if pieces_bytes.len() % 20 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid pieces length",
            ));
        }

        let mut pieces = Vec::new();
        for chunk in pieces_bytes.chunks(20) {
            let mut hash = [0u8; 20];
            hash.copy_from_slice(chunk);
            pieces.push(hash);
        }

        let length = match info_dict.get(&b"length"[..]) {
            Some(Bencode::Int(i)) => Some(*i as u64),
            _ => None,
        };

        let files = if let Some(Bencode::List(files_list)) = info_dict.get(&b"files"[..]) {
            let mut files = Vec::new();
            for file in files_list {
                if let Bencode::Dict(f) = file {
                    let len = match f.get(&b"length"[..]) {
                        Some(Bencode::Int(i)) => *i as u64,
                        _ => continue,
                    };
                    let path_list = match f.get(&b"path"[..]) {
                        Some(Bencode::List(l)) => l,
                        _ => continue,
                    };
                    let mut path = Vec::new();
                    for p in path_list {
                        if let Bencode::Bytes(b) = p {
                            path.push(String::from_utf8_lossy(b).to_string());
                        }
                    }
                    files.push(FileInfo { length: len, path });
                }
            }
            Some(files)
        } else {
            None
        };

        if length.is_none() && files.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Missing length or files",
            ));
        }

        Ok(Torrent {
            announce,
            announce_list,
            info_hash: hash,
            piece_length,
            pieces,
            name,
            length,
            files,
        })
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Torrent file root is not a dictionary",
        ));
    }
}
