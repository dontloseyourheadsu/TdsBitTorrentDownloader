//! Utilities for extracting the raw "info" slice from a Bencoded dictionary.

use super::decoder::{Bencode, decode};
use std::io;

/// Finds and returns the raw bytes corresponding to the value of the "info" key in a Bencoded dictionary.
///
/// This function expects the input to be a Bencoded dictionary containing an "info" key.
/// It returns a slice of the input byte array that represents the value associated with "info".
/// This is typically used to calculate the Info Hash.
///
/// # Arguments
/// * `input` - The Bencoded byte array.
///
/// # Returns
/// An `io::Result` containing the slice of the "info" value on success.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_info_slice() {
        // d4:infod3:bar3:baze5:otheri1ee
        let input = b"d4:infod3:bar3:baze5:otheri1ee";
        let info_slice = find_info_slice(input).expect("Should find info slice");
        assert_eq!(info_slice, b"d3:bar3:baze");
    }

    #[test]
    fn test_find_info_slice_not_found() {
        let input = b"d3:bar3:baze";
        let res = find_info_slice(input);
        assert!(res.is_err());
    }
    
    #[test]
    fn test_find_info_slice_invalid_format() {
        let input = b"l3:bare"; // list instead of dict
        let res = find_info_slice(input);
        assert!(res.is_err());
    }
}
