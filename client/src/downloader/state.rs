use crate::storage::Storage;
use std::sync::Arc;
use tokio::fs::File;
use tokio::sync::Mutex;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PieceStatus {
    Missing,
    InProgress,
    Have,
}

pub struct Downloader {
    pub torrent: Arc<tds_core::Torrent>,
    pub peer_id: [u8; 20],
    pub storage: Storage,
    pub file: Arc<Mutex<File>>,
    pub piece_status: Arc<Mutex<Vec<PieceStatus>>>,
    pub downloaded_bytes: Arc<Mutex<u64>>,
    pub uploaded_bytes: Arc<Mutex<u64>>,
    pub total_length: u64,
}
