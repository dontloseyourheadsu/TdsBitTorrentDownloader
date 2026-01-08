use crate::dht::Dht;
use crate::peer::{Message, PeerConnection};
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::net::SocketAddrV4;
use std::time::Duration;
use tds_core::bencoding::{Bencode, decode};
use tokio::sync::mpsc;
use url::Url;

/// Resolves a magnet link to the raw bytes of the info dictionary (metadata).
///
/// This process involves:
/// 1. Parsing the magnet link to get the info hash and initial trackers.
/// 2. Starting the DHT node to find peers associated with the info hash.
/// 3. Connecting to discovered peers.
/// 4. Using the BitTorrent Extension Protocol (BEP 10) to request the metadata (ut_metadata).
///
/// # Arguments
///
/// * `magnet_link` - A string containing the magnet URI.
///
/// # Returns
///
/// * `Result<Vec<u8>, ...>` - The raw bytes of the info dictionary if successful, or an error.
///
/// # Errors
///
/// Returns error if:
/// * Magnet link is invalid.
/// * DHT fails to start.
/// * Timeout occurs finding peers or metadata (current timeout 60s).
pub async fn resolve(
    magnet_link: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let (info_hash, _initial_trackers) = parse_magnet_link(magnet_link)?;
    println!(
        "Resolving magnet link for info_hash: {}",
        hex::encode(info_hash)
    );

    // Start DHT
    // Use port 0 to let OS pick a random free port to avoid conflicts
    let dht = std::sync::Arc::new(Dht::new(0).await?);

    dht.start().await;

    // Bootstrap DHT
    println!("Bootstrapping DHT...");
    dht.bootstrap().await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Start searching for peers
    println!("Searching for peers...");
    let dht_search = dht.clone();
    let hash_clone = info_hash;

    // Periodically query DHT
    tokio::spawn(async move {
        loop {
            dht_search.get_peers(hash_clone).await;
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    // We need a channel to receive the metadata result
    let (tx, mut rx) = mpsc::channel(1);

    // Limit concurrency
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(50));
    let mut searched_peers = std::collections::HashSet::new();

    let timeout = tokio::time::sleep(Duration::from_secs(60));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            _ = &mut timeout => {
                return Err("Timeout resolving magnet link".into());
            }
            metadata = rx.recv() => {
                if let Some(data) = metadata {
                    return Ok(data);
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(500)) => {
                 let peers = dht.get_found_peers().await;
                 for peer in peers {
                    if searched_peers.contains(&peer) {
                        continue;
                    }
                    searched_peers.insert(peer);

                    // println!("Found peer: {}", peer);
                    let sem = semaphore.clone();
                    let tx = tx.clone();
                    let info_hash = info_hash;

                    tokio::spawn(async move {
                        if let Ok(_permit) = sem.acquire().await {
                            if let Err(_e) = attempt_metadata_fetch(peer, info_hash, tx).await {
                                // println!("Failed to fetch metadata from {}: {}", peer, e);
                            }
                        }
                    });
                 }
            }
        }
    }
}

/// Attempts to fetch metadata from a single peer using the extension protocol (ut_metadata).
///
/// # Arguments
///
/// * `peer` - The address of the peer to connect to.
/// * `info_hash` - The target info hash.
/// * `tx` - A channel sender to report success.
async fn attempt_metadata_fetch(
    peer: SocketAddrV4,
    info_hash: [u8; 20],
    tx: mpsc::Sender<Vec<u8>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut client_id = [0u8; 20];
    rand::Rng::fill(&mut rand::rng(), &mut client_id);

    let mut peer_conn = match tokio::time::timeout(
        Duration::from_secs(3),
        PeerConnection::connect(peer, &info_hash, &client_id),
    )
    .await
    {
        Ok(res) => res?,
        Err(_) => return Err("Connect timeout".into()),
    };

    // Send our extended handshake
    let mut m = BTreeMap::new();
    let mut ut_metadata_dict = BTreeMap::new();
    ut_metadata_dict.insert(b"ut_metadata".to_vec(), Bencode::Int(2)); // We assign ID 2 for ut_metadata
    m.insert(b"ut_metadata".to_vec(), Bencode::Dict(ut_metadata_dict));

    let mut handshake_payload = BTreeMap::new();
    handshake_payload.insert(b"m".to_vec(), Bencode::Dict(m));

    let msg = Bencode::Dict(handshake_payload).encode();
    peer_conn
        .send_message(Message::Extended {
            id: 0,
            payload: msg,
        })
        .await?; // 0 is always handshake ID

    // Read messages until we get extended handshake response
    let mut ut_metadata_id = 0;
    let mut metadata_size = 0;

    // Wait for handshake response (timeout 10s)
    let handshake_fut = async {
        loop {
            let msg = peer_conn.read_message().await?;
            match msg {
                Message::Extended { id, payload } => {
                    if id == 0 {
                        // Handshake response
                        let mut pos = 0;
                        let root = decode(&payload, &mut pos)?;
                        if let Bencode::Dict(d) = root {
                            if let Some(Bencode::Dict(m)) = d.get(b"m".as_slice()) {
                                if let Some(Bencode::Int(id)) = m.get(b"ut_metadata".as_slice()) {
                                    ut_metadata_id = *id as u8;
                                }
                            }
                            if let Some(Bencode::Int(size)) = d.get(b"metadata_size".as_slice()) {
                                metadata_size = *size as u32;
                            }
                        }
                        return Ok::<(), Box<dyn std::error::Error + Send + Sync>>(());
                    }
                }
                _ => continue,
            }
        }
    };

    tokio::time::timeout(Duration::from_secs(5), handshake_fut).await??;

    if ut_metadata_id == 0 || metadata_size == 0 {
        return Err("Peer does not support ut_metadata or didn't send size".into());
    }

    // Request metadata pieces
    let piece_size = 16 * 1024;
    let num_pieces = (metadata_size + piece_size - 1) / piece_size;
    let mut metadata = vec![0u8; metadata_size as usize];
    let mut received_pieces = 0;

    for i in 0..num_pieces {
        // Request piece i
        let mut req = BTreeMap::new();
        req.insert(b"msg_type".to_vec(), Bencode::Int(0)); // 0 = request
        req.insert(b"piece".to_vec(), Bencode::Int(i as i64));
        let req_bytes = Bencode::Dict(req).encode();

        peer_conn
            .send_message(Message::Extended {
                id: ut_metadata_id,
                payload: req_bytes,
            })
            .await?;
    }

    // Wait for pieces
    let download_fut = async {
        while received_pieces < num_pieces {
            let msg = peer_conn.read_message().await?;
            match msg {
                Message::Extended { id, payload } => {
                    if id == ut_metadata_id {
                        let mut pos = 0;
                        let root = decode(&payload, &mut pos)?;

                        let mut piece_index = 0;
                        if let Bencode::Dict(d) = root {
                            if let Some(Bencode::Int(type_)) = d.get(b"msg_type".as_slice()) {
                                if *type_ == 1 {
                                    // 1 = data
                                    if let Some(Bencode::Int(idx)) = d.get(b"piece".as_slice()) {
                                        piece_index = *idx as u32;
                                    }

                                    // Data starts at pos
                                    if pos < payload.len() {
                                        let data = &payload[pos..];
                                        let start = (piece_index * piece_size) as usize;
                                        let end = std::cmp::min(start + data.len(), metadata.len());
                                        if start < metadata.len() {
                                            metadata[start..end]
                                                .copy_from_slice(&data[0..(end - start)]);
                                            received_pieces += 1;
                                        }
                                    }
                                } else if *type_ == 2 {
                                    return Err::<(), Box<dyn std::error::Error + Send + Sync>>(
                                        "Peer rejected metadata request".into(),
                                    );
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    };

    tokio::time::timeout(Duration::from_secs(10), download_fut).await??;

    // Verify hash
    let mut hasher = Sha1::new();
    hasher.update(&metadata);
    let hash: [u8; 20] = hasher.finalize().into();

    if hash == info_hash {
        println!("Metadata acquired from {}!", peer);
        let _ = tx.send(metadata).await;
        Ok(())
    } else {
        Err("Hash mismatch".into())
    }
}

/// Parses a magnet URI scheme.
///
/// Supports standard `magnet:?xt=urn:btih:<hex_hash>` format.
///
/// # Arguments
///
/// * `uri` - The magnet link string.
///
/// # Returns
///
/// * `Result<([u8; 20], Vec<String>), ...>` - A tuple containing the 20-byte info hash and a list of tracker URLs.
fn parse_magnet_link(
    uri: &str,
) -> Result<([u8; 20], Vec<String>), Box<dyn std::error::Error + Send + Sync>> {
    let url = Url::parse(uri)?;
    if url.scheme() != "magnet" {
        return Err("Not a magnet link".into());
    }

    let mut hash = None;
    let mut trackers = Vec::new();

    for (k, v) in url.query_pairs() {
        if k == "xt" {
            if v.starts_with("urn:btih:") {
                let h = &v["urn:btih:".len()..];
                if h.len() == 40 {
                    let mut arr = [0u8; 20];
                    hex::decode_to_slice(h, &mut arr).map_err(|_| "Invalid hex hash")?;
                    hash = Some(arr);
                } else if h.len() == 32 {
                    return Err("Base32 magnet links not yet supported".into());
                }
            }
        } else if k == "tr" {
            trackers.push(v.to_string());
        }
    }

    if let Some(h) = hash {
        Ok((h, trackers))
    } else {
        Err("Missing info hash".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_magnet_link_valid_hex() {
        let uri = "magnet:?xt=urn:btih:5b635ca35e4d2847a83709033333333333333333&tr=http://tracker.com";
        let res = parse_magnet_link(uri);
        assert!(res.is_ok());
        let (hash, trackers) = res.unwrap();
        assert_eq!(hex::encode(hash), "5b635ca35e4d2847a83709033333333333333333");
        assert_eq!(trackers.len(), 1);
        assert_eq!(trackers[0], "http://tracker.com");
    }

    #[test]
    fn test_parse_magnet_link_multiple_trackers() {
        let uri = "magnet:?xt=urn:btih:5b635ca35e4d2847a83709033333333333333333&tr=http://t1.com&tr=http://t2.com";
        let res = parse_magnet_link(uri);
        assert!(res.is_ok());
        let (_, trackers) = res.unwrap();
        assert_eq!(trackers.len(), 2);
        assert_eq!(trackers[0], "http://t1.com");
        assert_eq!(trackers[1], "http://t2.com");
    }

    #[test]
    fn test_parse_magnet_link_invalid_scheme() {
        assert!(parse_magnet_link("http://google.com").is_err());
    }

    #[test]
    fn test_parse_magnet_link_missing_xt() {
        assert!(parse_magnet_link("magnet:?tr=http://tracker.com").is_err());
    }
    
    #[test]
    fn test_parse_magnet_link_invalid_hex_len() {
        // Too short
        let uri = "magnet:?xt=urn:btih:12345&tr=http://tracker.com";
        assert!(parse_magnet_link(uri).is_ok() == false); // Should just fail to find hash or error
        // Actually code checks for len==40. If len != 40 and != 32, it falls through loops and returns Missing info hash
        match parse_magnet_link(uri) {
             Err(e) => assert_eq!(e.to_string(), "Missing info hash"),
             Ok(_) => panic!("Should have failed"),
        }
    }
}
