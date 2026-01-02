use std::io;
use std::path::{Path, PathBuf};
use tokio::fs;

pub struct Storage {
    pub download_dir: PathBuf,
}

impl Storage {
    pub async fn new(path: Option<String>) -> io::Result<Self> {
        let download_dir = if let Some(p) = path {
            PathBuf::from(p)
        } else {
            let mut p = std::env::current_dir()?;
            p.push("downloads");
            p
        };

        if !download_dir.exists() {
            fs::create_dir_all(&download_dir).await?;
        }

        Ok(Self { download_dir })
    }

    pub fn get_file_path(&self, filename: &str) -> PathBuf {
        self.download_dir.join(filename)
    }

    pub fn get_download_dir_str(&self) -> String {
        self.download_dir.to_string_lossy().to_string()
    }
}
