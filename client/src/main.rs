use rand::Rng;
use tds_core::parse_torrent;
use tracker::{TrackerEvent, TrackerRequest, get_tracker_client};

fn main() {
    match parse_torrent("example.torrent") {
        Ok(torrent) => {
            println!("Torrent parsed successfully!");
            println!("Announce URL: {}", torrent.announce);
            println!("Info Hash: {:x?}", torrent.info_hash);

            if let Some(client) = get_tracker_client(&torrent.announce) {
                println!("Contacting tracker...");

                let mut rng = rand::rng();
                let mut peer_id = [0u8; 20];
                rng.fill(&mut peer_id);
                // Convention: -<Client ID><Version>-<Random>
                // e.g. -TD0001-
                let prefix = b"-TD0001-";
                peer_id[..8].copy_from_slice(prefix);

                let request = TrackerRequest {
                    info_hash: torrent.info_hash,
                    peer_id,
                    port: 6881, // Standard BitTorrent port
                    uploaded: 0,
                    downloaded: 0,
                    left: 0, // Should be file size, but 0 for now
                    compact: true,
                    no_peer_id: false,
                    event: Some(TrackerEvent::Started),
                    ip: None,
                    numwant: Some(50),
                    key: None,
                    tracker_id: None,
                };

                match client.announce(&request) {
                    Ok(response) => {
                        println!("Tracker response received!");
                        println!("Interval: {}", response.interval);
                        println!("Peers: {}", response.peers.len());
                        for peer in response.peers {
                            println!("  {}", peer);
                        }
                    }
                    Err(e) => eprintln!("Tracker error: {}", e),
                }
            } else {
                eprintln!("Unsupported tracker protocol: {}", torrent.announce);
            }
        }
        Err(e) => eprintln!("Error parsing torrent: {}", e),
    }
}
