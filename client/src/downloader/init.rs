use super::state::{Downloader, PieceStatus};
use crate::storage::Storage;
use rand::Rng;
use sha1::{Digest, Sha1};
use std::io::SeekFrom;
use std::sync::Arc;
use tds_core::Torrent;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Mutex;

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

    let mut rng = rand::rng();
    let mut peer_id = [0u8; 20];
    rng.fill(&mut peer_id);
    let prefix = b"-TD0001-";
    peer_id[..8].copy_from_slice(prefix);

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
        downloaded_bytes: 0,
        total_length,
    })
}

pub async fn check_existing_data(
    downloader: &mut Downloader,
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
                    downloader.downloaded_bytes += len;
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
