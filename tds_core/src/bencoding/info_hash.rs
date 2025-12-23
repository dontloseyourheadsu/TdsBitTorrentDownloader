pub use sha1::{Digest, Sha1};

pub fn info_hash(info_bytes: &[u8]) -> [u8; 20] {
    let mut hasher = Sha1::new();
    hasher.update(info_bytes);
    let result = hasher.finalize();
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&result);
    hash
}
