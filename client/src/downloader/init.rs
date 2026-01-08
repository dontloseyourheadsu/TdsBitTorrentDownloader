use super::state::{Downloader, PieceStatus};
use crate::storage::Storage;
use rand::Rng;
use sha1::{Digest, Sha1};
use std::io::SeekFrom;
use std::sync::Arc;
use tds_core::Torrent;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Mutex;

/// Initializes a `Downloader` instance from a parsed `Torrent`.
///
/// This function:
/// 1. Initializes the `Storage` for the download directory.
/// 2. Generates a random Peer ID.
/// 3. Calculates the total length of the torrent content.
/// 4. Opens (or creates) the target file(s) for writing.
/// 5. Pre-allocates the file size if necessary.
/// 6. Initializes the piece status vector to `Missing`.
///
/// # Arguments
///
/// * `torrent` - The parsed torrent metadata.
/// * `output_path` - Optional custom output directory path.
///
/// # Returns
///
/// * `Result<Downloader, ...>` - The initialized downloader or an error.
pub async fn from_torrent(
    torrent: Torrent,
    output_path: Option<String>,
) -> Result<Downloader, Box<dyn std::error::Error + Send + Sync>> {
    let storage = match Storage::new(output_path).await {
        Ok(s) => s,
        Err(e) => return Err(format!("Failed to initialize storage: {}", e).into()),
    };
    println!("Download directory: {}", storage.get_download_dir_str());

    println!("Torrent parsed successfully!");
    println!("Info Hash: {:x?}", torrent.info_hash);

    let peer_id = {
        let mut rng = rand::rng();
        let mut id = [0u8; 20];
        rng.fill(&mut id);
        let prefix = b"-TD0001-";
        id[..8].copy_from_slice(prefix);
        id
    };

    let total_length = if let Some(len) = torrent.length {
        len
    } else if let Some(files) = &torrent.files {
        files.iter().map(|f| f.length).sum()
    } else {
        0
    };

    println!("Total length: {}", total_length);

    let file_path = if torrent.length.is_none() {
        println!("Multi-file torrents not fully supported yet, writing to 'output.bin'");
        storage.get_file_path("output.bin")
    } else {
        storage.get_file_path(&torrent.name)
    };

    let mut file = tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&file_path)
        .await?;

    let file_len = file.metadata().await?.len();
    if file_len != total_length {
        file.set_len(total_length).await?;
    }

    let piece_count = torrent.pieces.len();
    let piece_status_vec = vec![PieceStatus::Missing; piece_count];

    Ok(Downloader {
        torrent: Arc::new(torrent),
        peer_id,
        storage,
        file: Arc::new(Mutex::new(file)),
        piece_status: Arc::new(Mutex::new(piece_status_vec)),
        downloaded_bytes: Arc::new(Mutex::new(0)),
        uploaded_bytes: Arc::new(Mutex::new(0)),
        total_length,
    })
}

/// Checks for existing data on disk and updates piece status.
///
/// This function iterates through all pieces defined in the torrent:
/// 1. Reads the corresponding byte range from the file.
/// 2. Computes the SHA-1 hash.
/// 3. Compares it with the hash in the torrent metadata.
/// 4. If they match, marks the piece as `Have` and updates `downloaded_bytes`.
///
/// # Arguments
///
/// * `downloader` - The downloader instance to check.
pub async fn check_existing_data(
    downloader: &Downloader,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("Checking existing data...");
    let piece_count = downloader.torrent.pieces.len();
    let mut file = downloader.file.lock().await;
    let file_len = file.metadata().await?.len();
    let mut piece_status = downloader.piece_status.lock().await;

    for i in 0..piece_count {
        let offset = i as u64 * downloader.torrent.piece_length;
        let len = if i == piece_count - 1 {
            let rem = downloader.total_length % downloader.torrent.piece_length;
            if rem == 0 {
                downloader.torrent.piece_length
            } else {
                rem
            }
        } else {
            downloader.torrent.piece_length
        };

        if offset + len <= file_len {
            if let Err(_) = file.seek(SeekFrom::Start(offset)).await {
                continue;
            }
            let mut buf = vec![0u8; len as usize];
            if file.read_exact(&mut buf).await.is_ok() {
                let mut hasher = Sha1::new();
                hasher.update(&buf);
                let hash = hasher.finalize();
                if hash.as_slice() == &downloader.torrent.pieces[i] {
                    piece_status[i] = PieceStatus::Have;
                    *downloader.downloaded_bytes.lock().await += len;
                }
            }
        }
    }
    println!(
        "Resuming download. Found {}/{} pieces.",
        piece_status
            .iter()
            .filter(|&&s| s == PieceStatus::Have)
            .count(),
        piece_count
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_from_torrent_basic() {
        let dir = tempdir().unwrap();
        let path_str = dir.path().to_str().unwrap().to_string();

        let torrent = Torrent {
            announce: "http://tracker.com".to_string(),
            announce_list: None,
            info_hash: [0u8; 20],
            name: "test_file.txt".to_string(),
            pieces: vec![[0u8; 20]],
            piece_length: 1024,
            length: Some(1024),
            files: None,
        };

        let result = from_torrent(torrent, Some(path_str.clone())).await;
        assert!(result.is_ok());

        let downloader = result.unwrap();
        assert_eq!(downloader.total_length, 1024);
        assert_eq!(downloader.piece_status.lock().await.len(), 1);
        
        // File should exist and be 1024 bytes
        let file_path = dir.path().join("test_file.txt");
        assert!(file_path.exists());
        let metadata = tokio::fs::metadata(file_path).await.unwrap();
        assert_eq!(metadata.len(), 1024);
    }

    #[tokio::test]
    async fn test_check_existing_data() {
        let dir = tempdir().unwrap();
        let path_str = dir.path().to_str().unwrap().to_string();

        // Create a dummy piece (All 'A's)
        let piece_data = vec![b'A'; 10];
        let mut hasher = Sha1::new();
        hasher.update(&piece_data);
        let hash = hasher.finalize().to_vec();
        let mut hash_arr = [0u8; 20];
        hash_arr.copy_from_slice(&hash);

        let torrent = Torrent {
            announce: "http://tracker.com".to_string(),
            announce_list: None,
            info_hash: [0u8; 20],
            name: "existing.txt".to_string(),
            pieces: vec![hash_arr],
            piece_length: 10,
            length: Some(10),
            files: None,
        };

        // Write the valid data to the file first
        let file_path = dir.path().join("existing.txt");
        tokio::fs::write(&file_path, &piece_data).await.unwrap();

        let downloader = from_torrent(torrent, Some(path_str)).await.unwrap();
        
        // Status should be missing initially
        {
            let status = downloader.piece_status.lock().await;
            assert_eq!(status[0], PieceStatus::Missing);
        }

        // Run check
        check_existing_data(&downloader).await.unwrap();

        // Status should be Have
        {
            let status = downloader.piece_status.lock().await;
            assert_eq!(status[0], PieceStatus::Have);
        }
    }
}
