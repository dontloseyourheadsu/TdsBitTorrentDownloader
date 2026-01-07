// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use client::downloader::Downloader;
use client::magnet;
use std::sync::Arc;
use tauri::State;
use tokio::sync::Mutex;

struct AppState {
    downloader: Mutex<Option<Arc<Downloader>>>,
}

#[derive(serde::Serialize, Clone)]
struct TorrentStatus {
    active: bool,
    downloaded: u64,
    uploaded: u64,
    total: u64,
    progress: f64,
}

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

    // Store in state
    {
        let mut d = state.downloader.lock().await;
        *d = Some(downloader_arc.clone());
    }

    // Spawn the runner
    let d_clone = downloader_arc.clone();
    tokio::spawn(async move {
        d_clone.run().await;
    });

    Ok("Download started".to_string())
}

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

pub fn main() {
    tauri::Builder::default()
        .manage(AppState {
            downloader: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![start_download, get_status])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
