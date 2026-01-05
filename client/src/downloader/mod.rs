mod state;
mod init;
mod manager;

pub use state::{Downloader, PieceStatus};

impl Downloader {
    pub async fn new(
        torrent_path: &str,
        output_path: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        init::new(torrent_path, output_path).await
    }

    pub async fn check_existing_data(
        &mut self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        init::check_existing_data(self).await
    }

    pub async fn run(&mut self) {
        manager::run(self).await
    }
}
