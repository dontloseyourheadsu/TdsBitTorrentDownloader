//! BitTorrent Tracker Application Entry Point.
//!
//! This crate implements a simple BitTorrent tracker supporting both HTTP and UDP protocols.
//!
//! # Usage
//!
//! Run the tracker binary. It will start listening on the default port (6969).

use tracker::server::TrackerServer;

/// Main entry point for the tracker application.
///
/// Starts the Tracker Server on port 6969 and awaits its completion.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let server = TrackerServer::new(6969);
    println!("Starting tracker on port 6969...");
    server.start().await?;
    Ok(())
}
