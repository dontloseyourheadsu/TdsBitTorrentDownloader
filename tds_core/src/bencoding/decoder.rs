use std::collections::BTreeMap;
use std::io;

#[derive(Debug)]
pub enum Bencode {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<Bencode>),
    Dict(BTreeMap<Vec<u8>, Bencode>),
}

impl Bencode {
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
