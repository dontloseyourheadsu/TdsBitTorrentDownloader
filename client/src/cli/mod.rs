use clap::Parser;

/// Command line arguments for the TDS BitTorrent Downloader client.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Path to the torrent file or magnet link.
    ///
    /// If arguments start with `magnet:?`, it is treated as a magnet URI.
    /// Otherwise, it is treated as a file path to a .torrent file.
    #[arg(short, long, default_value = "example.torrent")]
    pub torrent: String,

    /// Output directory for downloaded files.
    ///
    /// If not specified, downloads will save to a `downloads` folder in the current directory.
    #[arg(short, long)]
    pub output: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_cli() {
        use clap::CommandFactory;
        Args::command().debug_assert();
    }
}
