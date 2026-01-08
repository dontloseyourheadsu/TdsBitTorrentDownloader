//! Tauri Backend for tds-ui.
//!
//! Handles communication between the frontend and the `client` library.
//! Manages the state of the active downloader.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use client::downloader::Downloader;
use client::magnet;
use std::sync::Arc;
use tauri::State;
use tokio::sync::Mutex;

/// Application state managed by Tauri.
struct AppState {
    /// The currently active downloader instance, if any.
    downloader: Mutex<Option<Arc<Downloader>>>,
}

/// Status of the current torrent download.
#[derive(serde::Serialize, Clone)]
struct TorrentStatus {
    /// Whether a download is active.
    active: bool,
    /// Bytes downloaded.
    downloaded: u64,
    /// Bytes uploaded (mock).
    uploaded: u64,
    /// Total size of the torrent.
    total: u64,
    /// Progress percentage (0.0 to 100.0).
    progress: f64,
}

/// Starts a download from a torrent file or magnet link.
///
/// # Arguments
/// * `state` - The application state.
/// * `torrent_input` - File path or Magnet URI.
/// * `output_path` - Optional directory to save files to.
///
/// # Returns
/// "Download started" on success, or an error message.
#[tauri::command]
async fn start_download(
    state: State<'_, AppState>,
    torrent_input: String,
    output_path: Option<String>,
) -> Result<String, String> {
    println!("Starting download for: {}", torrent_input);

    let torrent_struct = if torrent_input.starts_with("magnet:") {
        match magnet::resolve(&torrent_input).await {
            Ok(info_bytes) => {
                let mut wrapper = Vec::new();
                wrapper.extend_from_slice(b"d8:announce0:4:info");
                wrapper.extend_from_slice(&info_bytes);
                wrapper.push(b'e');

                match tds_core::parse_torrent_from_bytes(&wrapper) {
                    Ok(t) => t,
                    Err(e) => return Err(format!("Error parsing resolved metadata: {}", e)),
                }
            }
            Err(e) => return Err(format!("Error resolving magnet link: {}", e)),
        }
    } else {
        match tds_core::parse_torrent(&torrent_input) {
            Ok(t) => t,
            Err(e) => return Err(format!("Error parsing torrent file: {}", e)),
        }
    };

    let downloader = match Downloader::from_torrent(torrent_struct, output_path).await {
        Ok(d) => d,
        Err(e) => return Err(format!("Error initializing downloader: {}", e)),
    };

    if let Err(e) = downloader.check_existing_data().await {
        return Err(format!("Error checking existing data: {}", e));
    }

    let downloader_arc = Arc::new(downloader);

    {
        let mut d = state.downloader.lock().await;
        *d = Some(downloader_arc.clone());
    }

    let d_clone = downloader_arc.clone();
    tokio::spawn(async move {
        d_clone.run().await;
    });

    Ok("Download started".to_string())
}

/// Retrieves the current status of the download.
///
/// # Arguments
/// * `state` - The application state.
///
/// # Returns
/// A `TorrentStatus` struct containing progress, downloaded bytes, etc.
#[tauri::command]
async fn get_status(state: State<'_, AppState>) -> Result<TorrentStatus, String> {
    let d_lock = state.downloader.lock().await;

    if let Some(downloader) = &*d_lock {
        let downloaded = *downloader.downloaded_bytes.lock().await;
        let uploaded = *downloader.uploaded_bytes.lock().await;
        let total = downloader.total_length;

        let progress = if total > 0 {
            (downloaded as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        Ok(TorrentStatus {
            active: true,
            downloaded,
            uploaded,
            total,
            progress,
        })
    } else {
        Ok(TorrentStatus {
            active: false,
            downloaded: 0,
            uploaded: 0,
            total: 0,
            progress: 0.0,
        })
    }
}

/// Main entry point for the Tauri application.
pub fn main() {
    tauri::Builder::default()
        .manage(AppState {
            downloader: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![start_download, get_status])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
