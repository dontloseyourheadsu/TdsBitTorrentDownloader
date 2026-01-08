use std::io;
use std::path::PathBuf;
use tokio::fs;

/// Manages the file system storage for downloaded files.
///
/// The `Storage` struct is responsible for determining the download directory,
/// creating it if it doesn't exist, and resolving file paths relative to it.
pub struct Storage {
    /// The root directory where files will be stored.
    pub download_dir: PathBuf,
}

impl Storage {
    /// Creates a new `Storage` instance.
    ///
    /// If a `path` is provided, it uses that as the download directory.
    /// If `path` is `None`, it defaults to a `downloads` directory in the current working directory.
    ///
    /// This function also attempts to create the directory if it does not exist.
    ///
    /// # Arguments
    ///
    /// * `path` - An optional string slice that holds the path to the download directory.
    ///
    /// # Returns
    ///
    /// * `io::Result<Self>` - A result containing the `Storage` instance or an IO error.
    ///
    /// # Examples
    ///
    /// ```
    /// use client::storage::Storage;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     // Note: This example assumes you have write permissions to the current directory
    ///     let storage = Storage::new(Some("./my_downloads".to_string())).await.unwrap();
    ///     assert!(storage.download_dir.ends_with("my_downloads"));
    /// }
    /// ```
    pub async fn new(path: Option<String>) -> io::Result<Self> {
        let download_dir = if let Some(p) = path {
            PathBuf::from(p)
        } else {
            let mut p = std::env::current_dir()?;
            p.push("downloads");
            p
        };

        // Check if path exists and is a directory
        match fs::metadata(&download_dir).await {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "Path exists but is not a directory",
                    ));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                fs::create_dir_all(&download_dir).await?;
            }
            Err(e) => return Err(e),
        }

        Ok(Self { download_dir })
    }

    /// Resolves the full path for a given filename relative to the download directory.
    ///
    /// # Arguments
    ///
    /// * `filename` - The name of the file to resolve.
    ///
    /// # Returns
    ///
    /// * `PathBuf` - The full path to the file.
    ///
    /// # Examples
    ///
    /// ```
    /// use client::storage::Storage;
    /// use std::path::PathBuf;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let storage = Storage::new(None).await.unwrap();
    ///     let path = storage.get_file_path("test.txt");
    ///     // path will be .../downloads/test.txt
    /// }
    /// ```
    pub fn get_file_path(&self, filename: &str) -> PathBuf {
        self.download_dir.join(filename)
    }

    /// Returns the download directory path as a string.
    ///
    /// This uses `to_string_lossy()` so it may replace non-UTF8 characters.
    ///
    /// # Returns
    ///
    /// * `String` - The string representation of the download directory path.
    pub fn get_download_dir_str(&self) -> String {
        self.download_dir.to_string_lossy().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_new_storage_default() {
        // When no path is provided, it should default to "downloads" in current dir.
        let storage = Storage::new(None).await.expect("Failed to create storage");
        assert!(storage.download_dir.ends_with("downloads"));
    }

    #[tokio::test]
    async fn test_new_storage_with_path() {
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let path_str = temp_dir.path().to_str().unwrap().to_string();

        let storage = Storage::new(Some(path_str.clone()))
            .await
            .expect("Failed to create storage");
        assert_eq!(storage.get_download_dir_str(), path_str);
        assert!(storage.download_dir.exists());
    }

    #[tokio::test]
    async fn test_get_file_path() {
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let storage = Storage {
            download_dir: temp_dir.path().to_path_buf(),
        };

        let filename = "test_file.txt";
        let file_path = storage.get_file_path(filename);
        assert_eq!(file_path, temp_dir.path().join(filename));
    }

    #[tokio::test]
    async fn test_storage_creation_fails_on_invalid_path() {
        // A better test for failure might be a file where a dir is expected
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let file_path = temp_dir.path().join("i_am_a_file");
        fs::write(&file_path, "content").await.unwrap();

        // Try to create storage where a file exists with same name
        let result = Storage::new(Some(file_path.to_str().unwrap().to_string())).await;
        assert!(
            result.is_err(),
            "Should return error if path exists and is not a directory"
        );
    }
}
