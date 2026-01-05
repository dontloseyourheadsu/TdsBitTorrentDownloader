use clap::Parser;
mod cli;
mod dht;
mod downloader;
mod magnet;
mod peer;
mod storage;

use cli::Args;
use downloader::Downloader;
use tds_core::Torrent;

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let torrent_struct = if args.torrent.starts_with("magnet:") {
        println!("Magnet link detected, resolving metadata...");
        match magnet::resolve(&args.torrent).await {
            Ok(info_bytes) => {
                println!("Metadata resolved! Parsing...");
                // Construct a synthetic .torrent file content in memory
                // structure: d8:announce0:4:info<INFO_BYTES>e
                let mut wrapper = Vec::new();
                wrapper.extend_from_slice(b"d8:announce0:4:info");
                wrapper.extend_from_slice(&info_bytes);
                wrapper.push(b'e');

                match tds_core::parse_torrent_from_bytes(&wrapper) {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("Error parsing resolved metadata: {}", e);
                        return;
                    }
                }
            }
            Err(e) => {
                eprintln!("Error resolving magnet link: {}", e);
                return;
            }
        }
    } else {
        match tds_core::parse_torrent(&args.torrent) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Error parsing torrent file: {}", e);
                return;
            }
        }
    };

    let mut downloader = match Downloader::from_torrent(torrent_struct, args.output).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error initializing downloader: {}", e);
            return;
        }
    };

    if let Err(e) = downloader.check_existing_data().await {
        eprintln!("Error checking existing data: {}", e);
        return;
    }

    downloader.run().await;
    println!("Download finished.");
}
