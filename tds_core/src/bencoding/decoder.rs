//! Bencode decoder/encoder.

use std::collections::BTreeMap;
use std::io;

/// Represents a Bencoded value.
#[derive(Debug, PartialEq, Clone)]
pub enum Bencode {
    /// An integer value.
    Int(i64),
    /// A byte array (string).
    Bytes(Vec<u8>),
    /// A list of Bencoded values.
    List(Vec<Bencode>),
    /// A dictionary mapping byte arrays to Bencoded values.
    Dict(BTreeMap<Vec<u8>, Bencode>),
}

impl Bencode {
    /// Encodes the Bencode value into a byte vector.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.encode_into(&mut buf);
        buf
    }

    fn encode_into(&self, buf: &mut Vec<u8>) {
        match self {
            Bencode::Int(i) => {
                buf.push(b'i');
                buf.extend_from_slice(i.to_string().as_bytes());
                buf.push(b'e');
            }
            Bencode::Bytes(b) => {
                buf.extend_from_slice(b.len().to_string().as_bytes());
                buf.push(b':');
                buf.extend_from_slice(b);
            }
            Bencode::List(l) => {
                buf.push(b'l');
                for item in l {
                    item.encode_into(buf);
                }
                buf.push(b'e');
            }
            Bencode::Dict(d) => {
                buf.push(b'd');
                for (k, v) in d {
                    buf.extend_from_slice(k.len().to_string().as_bytes());
                    buf.push(b':');
                    buf.extend_from_slice(k);
                    v.encode_into(buf);
                }
                buf.push(b'e');
            }
        }
    }
}

/// Decodes a Bencoded value from a byte slice.
///
/// # Arguments
/// * `input` - The byte slice to decode.
/// * `pos` - A mutable reference to the current position in the input.
pub fn decode(input: &[u8], pos: &mut usize) -> io::Result<Bencode> {
    if *pos >= input.len() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "EOF reached"));
    }
    match input[*pos] {
        b'i' => {
            *pos += 1;
            let start = *pos;
            while *pos < input.len() && input[*pos] != b'e' {
                *pos += 1;
            }
            if *pos >= input.len() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "EOF while parsing integer",
                ));
            }
            let num_str = std::str::from_utf8(&input[start..*pos]).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Invalid UTF-8 in integer: {}", e),
                )
            })?;
            let num = num_str.parse::<i64>().map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Invalid integer format: {}", e),
                )
            })?;
            *pos += 1;
            Ok(Bencode::Int(num))
        }

        b'l' => {
            *pos += 1;
            let mut list = Vec::new();
            while *pos < input.len() && input[*pos] != b'e' {
                let item = match decode(input, pos) {
                    Ok(val) => val,
                    Err(e) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("Failed to decode list item at pos {}: {}", *pos, e),
                        ));
                    }
                };
                list.push(item);
            }
            if *pos >= input.len() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "EOF while parsing list",
                ));
            }
            *pos += 1;
            Ok(Bencode::List(list))
        }

        b'd' => {
            *pos += 1;
            let mut dict = BTreeMap::new();
            while *pos < input.len() && input[*pos] != b'e' {
                let key_obj = match decode(input, pos) {
                    Ok(val) => val,
                    Err(e) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("Failed to decode dict key at pos {}: {}", *pos, e),
                        ));
                    }
                };
                let key = match key_obj {
                    Bencode::Bytes(b) => b,
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "dict key must be bytes",
                        ));
                    }
                };
                let val = match decode(input, pos) {
                    Ok(val) => val,
                    Err(e) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("Failed to decode dict value at pos {}: {}", *pos, e),
                        ));
                    }
                };
                dict.insert(key, val);
            }
            if *pos >= input.len() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "EOF while parsing dict",
                ));
            }
            *pos += 1;
            Ok(Bencode::Dict(dict))
        }

        b'0'..=b'9' => {
            let start = *pos;
            while *pos < input.len() && input[*pos] != b':' {
                *pos += 1;
            }
            if *pos >= input.len() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "EOF while parsing string length",
                ));
            }
            let len_str = std::str::from_utf8(&input[start..*pos]).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Invalid UTF-8 in string length: {}", e),
                )
            })?;
            let len = len_str.parse::<usize>().map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Invalid string length format: {}", e),
                )
            })?;
            *pos += 1;

            if *pos + len > input.len() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "String length exceeds input",
                ));
            }

            let bytes = input[*pos..*pos + len].to_vec();
            *pos += len;
            Ok(Bencode::Bytes(bytes))
        }

        c => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid bencode char: '{}' at pos {}", c as char, *pos),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_int() {
        let val = Bencode::Int(42);
        let encoded = val.encode();
        assert_eq!(encoded, b"i42e");

        let mut pos = 0;
        let decoded = decode(&encoded, &mut pos).unwrap();
        assert_eq!(decoded, val);
    }

    #[test]
    fn test_encode_decode_string() {
        let val = Bencode::Bytes(b"hello".to_vec());
        let encoded = val.encode();
        assert_eq!(encoded, b"5:hello");

        let mut pos = 0;
        let decoded = decode(&encoded, &mut pos).unwrap();
        assert_eq!(decoded, val);
    }

    #[test]
    fn test_encode_decode_list() {
        let val = Bencode::List(vec![Bencode::Int(1), Bencode::Bytes(b"a".to_vec())]);
        let encoded = val.encode();
        assert_eq!(encoded, b"li1e1:ae");

        let mut pos = 0;
        let decoded = decode(&encoded, &mut pos).unwrap();
        assert_eq!(decoded, val);
    }

    #[test]
    fn test_encode_decode_dict() {
        let mut map = BTreeMap::new();
        map.insert(b"a".to_vec(), Bencode::Int(1));
        map.insert(b"b".to_vec(), Bencode::Bytes(b"c".to_vec()));
        let val = Bencode::Dict(map);
        let encoded = val.encode();
        // Keys sorted alphabetically: a, b
        assert_eq!(encoded, b"d1:ai1e1:b1:ce");

        let mut pos = 0;
        let decoded = decode(&encoded, &mut pos).unwrap();
        assert_eq!(decoded, val);
    }

    #[test]
    fn test_decode_invalid() {
        let mut pos = 0;
        assert!(decode(b"i42", &mut pos).is_err()); // Missing 'e'
    }
}
