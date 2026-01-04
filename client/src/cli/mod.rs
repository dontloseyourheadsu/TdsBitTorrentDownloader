use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Path to the torrent file
    #[arg(short, long, default_value = "example.torrent")]
    pub torrent: String,

    /// Output directory for downloaded files
    #[arg(short, long)]
    pub output: Option<String>,
}
