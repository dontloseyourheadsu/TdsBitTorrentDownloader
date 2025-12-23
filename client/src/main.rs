use tds_core::parse_torrent;

fn main() {
    match parse_torrent("example.torrent") {
        Ok(_) => println!("Torrent parsed successfully!"),
        Err(e) => eprintln!("Error parsing torrent: {}", e),
    }
}
