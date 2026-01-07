#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::State;
use tokio::sync::Mutex;
use tracker::server::TrackerServer;

struct AppState {
    tracker: Mutex<Option<TrackerServer>>,
}

#[tauri::command]
async fn start_tracker(
    state: State<'_, AppState>,
    port: u16,
    use_udp: bool,
) -> Result<String, String> {
    if use_udp {
        return Err("UDP Tracker support not yet implemented.".into());
    }

    let mut t_lock = state.tracker.lock().await;
    if let Some(t) = &*t_lock {
        let running = t.running.lock().await;
        if *running {
            return Err("Tracker is already running".into());
        }
    }

    let server = TrackerServer::new(port);

    // Start server in background
    let server_clone = server.clone();
    tokio::spawn(async move {
        if let Err(e) = server_clone.start().await {
            eprintln!("Tracker Server Error: {}", e);
        }
    });

    *t_lock = Some(server);

    Ok(format!("Tracker started on port {}", port))
}

#[tauri::command]
async fn stop_tracker(state: State<'_, AppState>) -> Result<String, String> {
    let t_lock = state.tracker.lock().await;
    if let Some(server) = &*t_lock {
        let mut running = server.running.lock().await;
        if *running {
            *running = false;
            return Ok("Tracker stopping...".into());
        }
    }
    Err("Tracker not running".into())
}

#[tauri::command]
async fn get_tracker_status(state: State<'_, AppState>) -> Result<String, String> {
    let t_lock = state.tracker.lock().await;
    if let Some(server) = &*t_lock {
        let running = server.running.lock().await;
        if *running {
            return Ok("Running".into());
        }
    }
    Ok("Stopped".into())
}

pub fn main() {
    tauri::Builder::default()
        .manage(AppState {
            tracker: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            start_tracker,
            stop_tracker,
            get_tracker_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
