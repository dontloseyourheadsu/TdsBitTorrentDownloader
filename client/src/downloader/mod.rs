//! The downloader module coordinates the entire downloading process.
//!
//! It manages state, initialization, peer connections, and the main event loop.

mod init;
mod manager;
mod state;

pub use state::{Downloader, PieceStatus};

impl Downloader {
    /// Creates a new `Downloader` from a torrent file.
    ///
    /// # Arguments
    ///
    /// * `torrent_path` - The path to the `.torrent` file.
    /// * `output_path` - Optional path to save the downloaded file. Defaults to the name in the torrent file.
    pub async fn new(
        torrent_path: &str,
        output_path: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let torrent = tds_core::parse_torrent(torrent_path)
            .map_err(|e| format!("Error parsing torrent: {}", e))?;
        init::from_torrent(torrent, output_path).await
    }

    /// Creates a new `Downloader` from a parsed `Torrent` struct.
    pub async fn from_torrent(
        torrent: tds_core::Torrent,
        output_path: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        init::from_torrent(torrent, output_path).await
    }

    /// Checks the integrity of existing file data.
    ///
    /// If the output file already exists, this function verifies the hashes of 
    /// the pieces and marks them as `Have` if they are correct.
    pub async fn check_existing_data(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        init::check_existing_data(self).await
    }

    /// Starts the main download loop.
    ///
    /// This function blocks until the download is complete or an error occurs.
    pub async fn run(&self) {
        manager::run(self).await
    }
}
