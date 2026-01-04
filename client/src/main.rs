use clap::Parser;
mod cli;
mod dht;
mod downloader;
mod peer;
mod storage;

use cli::Args;
use downloader::Downloader;

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let mut downloader = match Downloader::new(&args.torrent, args.output).await {
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
