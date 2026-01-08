//! Utilities for calculating the Info Hash of a torrent.

pub use sha1::{Digest, Sha1};

/// Calculates the SHA-1 hash of the "info" dictionary from a torrent file.
///
/// # Arguments
/// * `info_bytes` - The raw bytes of the "info" dictionary.
///
/// # Returns
/// A 20-byte array containing the SHA-1 hash.
pub fn info_hash(info_bytes: &[u8]) -> [u8; 20] {
    let mut hasher = Sha1::new();
    hasher.update(info_bytes);
    let result = hasher.finalize();
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&result);
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_info_hash() {
        // "test" in SHA1 is a94a8fe5ccb19ba61c4c0873d391e987982fbbd3
        let data = b"test";
        let hash = info_hash(data);
        
        let expected = [
            0xa9, 0x4a, 0x8f, 0xe5, 0xcc, 0xb1, 0x9b, 0xa6, 0x1c, 0x4c,
            0x08, 0x73, 0xd3, 0x91, 0xe9, 0x87, 0x98, 0x2f, 0xbb, 0xd3
        ];
        
        assert_eq!(hash, expected);
    }
}
