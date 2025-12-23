use super::bencode::Bencode;
use super::decoder::decode;
use std::io;

pub fn find_info_slice(input: &[u8]) -> io::Result<&[u8]> {
    let mut pos = 0;

    if pos >= input.len() || input[pos] != b'd' {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a dict"));
    }
    pos += 1;

    while pos < input.len() && input[pos] != b'e' {
        // decode key
        let _ = pos;
        let key = match decode(input, &mut pos) {
            Ok(k) => k,
            Err(e) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Failed to decode key in info_slice: {}", e),
                ));
            }
        };
        let key_bytes = match key {
            Bencode::Bytes(b) => b,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid key in info_slice, expected bytes",
                ));
            }
        };

        if key_bytes == b"info" {
            let info_start = pos;
            match decode(input, &mut pos) {
                Ok(_) => {}
                Err(e) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("Failed to decode info value: {}", e),
                    ));
                }
            }
            let info_end = pos;
            return Ok(&input[info_start..info_end]);
        } else {
            match decode(input, &mut pos) {
                Ok(_) => {}
                Err(e) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("Failed to skip value in info_slice: {}", e),
                    ));
                }
            }
        }
    }

    Err(io::Error::new(io::ErrorKind::NotFound, "info not found"))
}
