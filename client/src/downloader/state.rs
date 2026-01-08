use crate::storage::Storage;
use std::sync::Arc;
use tokio::fs::File;
use tokio::sync::Mutex;

/// Represents the download status of a specific piece of the torrent.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PieceStatus {
    /// The piece has not been downloaded yet.
    Missing,
    /// The piece is currently being downloaded from a peer.
    InProgress,
    /// The piece has been successfully downloaded and verified.
    Have,
}

/// The main structure managing the download state of a torrent.
///
/// It holds shared state accessible by multiple components (tracker manager, peer connections, etc.),
/// including the torrent metadata, storage file handle, and bitfield of piece statuses.
pub struct Downloader {
    /// The parsed torrent metadata.
    pub torrent: Arc<tds_core::Torrent>,
    /// The unique Peer ID generated for this client session.
    pub peer_id: [u8; 20],
    /// The storage manager handle.
    pub storage: Storage,
    /// The open file handle where data is written.
    /// Wrapped in a Mutex for concurrent access.
    pub file: Arc<Mutex<File>>,
    /// A vector tracking the status of each piece.
    /// Wrapped in a Mutex for concurrent updates.
    pub piece_status: Arc<Mutex<Vec<PieceStatus>>>,
    /// total number of bytes downloaded in this session.
    pub downloaded_bytes: Arc<Mutex<u64>>,
    /// total number of bytes uploaded in this session.
    pub uploaded_bytes: Arc<Mutex<u64>>,
    /// The total size of the torrent content in bytes.
    pub total_length: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_piece_status_equality() {
        assert_eq!(PieceStatus::Missing, PieceStatus::Missing);
        assert_ne!(PieceStatus::Missing, PieceStatus::Have);
    }

    #[test]
    fn test_piece_status_debug() {
        let status = PieceStatus::InProgress;
        assert_eq!(format!("{:?}", status), "InProgress");
    }
}
