mod init;
mod manager;
mod state;

pub use state::{Downloader, PieceStatus};

impl Downloader {
    pub async fn new(
        torrent_path: &str,
        output_path: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let torrent = tds_core::parse_torrent(torrent_path)
            .map_err(|e| format!("Error parsing torrent: {}", e))?;
        init::from_torrent(torrent, output_path).await
    }

    pub async fn from_torrent(
        torrent: tds_core::Torrent,
        output_path: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        init::from_torrent(torrent, output_path).await
    }

    pub async fn check_existing_data(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        init::check_existing_data(self).await
    }

    pub async fn run(&self) {
        manager::run(self).await
    }
}
