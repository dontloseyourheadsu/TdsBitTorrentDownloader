//! Core library for the TDS BitTorrent Client.
//!
//! This library provides data structures and functions for parsing `.torrent` files
//! and handling bencoded data.

pub mod bencoding;
pub mod rate_limit;

use bencoding::{Bencode, decode, find_info_slice, info_hash};
pub use rate_limit::TokenBucket;
use std::io::{self, Read};

/// Information about a single file in a multi-file torrent.
#[derive(Debug, Clone)]
pub struct FileInfo {
    /// The length of the file in bytes.
    pub length: u64,
    /// The path components of the file.
    pub path: Vec<String>,
}

/// Represents the metadata of a torrent.
#[derive(Debug)]
pub struct Torrent {
    /// The URL of the tracker.
    pub announce: String,
    /// Optional list of backup trackers (tier-based).
    pub announce_list: Option<Vec<Vec<String>>>,
    /// The SHA-1 hash of the info dictionary.
    pub info_hash: [u8; 20],
    /// The length of a single piece in bytes.
    pub piece_length: u64,
    /// The list of SHA-1 hashes for each piece.
    pub pieces: Vec<[u8; 20]>,
    /// The name of the file or directory.
    pub name: String,
    /// Total length of the file (single-file mode).
    pub length: Option<u64>,
    /// List of files (multi-file mode).
    pub files: Option<Vec<FileInfo>>,
}

/// Parses a `.torrent` file from the disk.
///
/// # Arguments
///
/// * `path` - The path to the torrent file.
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

/// Parses a torrent from a byte slice.
///
/// # Arguments
///
/// * `buf` - The byte slice containing the bencoded torrent data.
pub fn parse_torrent_from_bytes(buf: &[u8]) -> io::Result<Torrent> {
    let mut pos = 0;
    let root = decode(buf, &mut pos)?;

    let info_bytes = find_info_slice(buf)?;
    let hash = info_hash(info_bytes);

    if let Bencode::Dict(ref dict) = root {
        let announce = match dict.get(&b"announce"[..]) {
            Some(Bencode::Bytes(bytes)) => String::from_utf8_lossy(bytes).to_string(),
            _ => String::new(), 
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn create_dummy_torrent() -> Vec<u8> {
        // Hand-craft a simple single-file torrent structure
        // d8:announce15:http://track.er4:infod6:lengthi12345e4:name8:testfile12:piece lengthi16384e6:pieces20:00000000000000000000ee
        // pieces needs 20 bytes. Let's make it more readable or just raw.
        // announce: http://track.er (15 chars)
        let mut t = "d8:announce15:http://track.er4:infod6:lengthi12345e4:name8:testfile12:piece lengthi16384e6:pieces20:".as_bytes().to_vec();
        t.extend_from_slice(&[b'X'; 20]); // dummy hash
        t.extend_from_slice(b"ee");
        t
    }

    #[test]
    fn test_parse_simple_torrent() {
        let buf = create_dummy_torrent();
        let t = parse_torrent_from_bytes(&buf).expect("Should parse");
        assert_eq!(t.announce, "http://track.er");
        assert_eq!(t.name, "testfile");
        assert_eq!(t.length, Some(12345));
        assert_eq!(t.piece_length, 16384);
        assert_eq!(t.pieces.len(), 1);
        assert!(t.files.is_none());
    }

    #[test]
    fn test_parse_invalid_torrent() {
        let buf = b"invalid";
        let res = parse_torrent_from_bytes(buf);
        assert!(res.is_err());
    }
}
