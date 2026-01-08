//! Simple HTTP Tracker Server Implementation.
//!
//! This module implements a basic BitTorrent tracker server that handles HTTP GET announce requests.
//! It maintains a list of peers for each torrent info hash and performs rate limiting based on IP address.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tds_core::TokenBucket;
use tds_core::bencoding::Bencode;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use url::Url;

/// Holds the in-memory state of the tracker.
pub struct TrackerState {
    /// Maps InfoHash (hex string) to a list of Peers.
    pub torrents: HashMap<String, Vec<Peer>>,
    /// Rate limit buckets per IP address.
    pub rate_limits: HashMap<IpAddr, TokenBucket>,
}

/// Represents a peer connected to the tracker.
#[derive(Clone, Debug)]
pub struct Peer {
    /// Peer ID.
    pub id: String,
    /// Peer IP address.
    pub ip: IpAddr,
    /// Peer port.
    pub port: u16,
    /// Last time this peer announced.
    pub last_seen: Instant,
}

/// The main Tracker Server struct.
#[derive(Clone)]
pub struct TrackerServer {
    /// Shared state guarded by a Mutex.
    pub state: Arc<Mutex<TrackerState>>,
    /// The port to listen on.
    pub port: u16,
    /// Flag to control the server loop.
    pub running: Arc<Mutex<bool>>,
}

impl TrackerServer {
    /// Creates a new `TrackerServer` instance.
    ///
    /// # Arguments
    /// * `port` - The port to listen on.
    pub fn new(port: u16) -> Self {
        Self {
            state: Arc::new(Mutex::new(TrackerState {
                torrents: HashMap::new(),
                rate_limits: HashMap::new(),
            })),
            port,
            running: Arc::new(Mutex::new(false)),
        }
    }

    /// Starts the tracker server.
    ///
    /// This function binds to the configured port and starts accepting incoming TCP connections.
    /// It runs until `running` is set to false or an error occurs.
    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = TcpListener::bind(format!("0.0.0.0:{}", self.port)).await?;
        println!("Tracker server listening on 0.0.0.0:{}", self.port);

        {
            let mut running = self.running.lock().await;
            *running = true;
        }

        let state = self.state.clone();
        let running = self.running.clone();

        loop {
            // Check if we should stop
            if !*running.lock().await {
                break;
            }

            match tokio::time::timeout(Duration::from_secs(1), listener.accept()).await {
                Ok(Ok((stream, addr))) => {
                    let state_clone = state.clone();
                    tokio::spawn(async move {
                        handle_connection(stream, addr, state_clone).await;
                    });
                }
                Ok(Err(e)) => {
                    eprintln!("Accept error: {}", e);
                }
                Err(_) => {
                    // Timeout, loop back to check running state
                    continue;
                }
            }
        }
        Ok(())
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    peer_addr: SocketAddr,
    state: Arc<Mutex<TrackerState>>,
) {
    let peer_ip = peer_addr.ip();

    // 1. Rate Limiting Check
    {
        let mut guard = state.lock().await;
        let bucket = guard
            .rate_limits
            .entry(peer_ip)
            .or_insert_with(|| TokenBucket::new(5.0, 0.5));

        if !bucket.consume(1.0) {
            let response =
                "HTTP/1.1 429 Too Many Requests\r\nConnection: close\r\n\r\nRate limit exceeded";
            let _ = stream.write_all(response.as_bytes()).await;
            return;
        }
    }

    let mut buffer = [0; 2048];
    let bytes_read = match stream.read(&mut buffer).await {
        Ok(n) if n > 0 => n,
        _ => return,
    };

    let request_str = String::from_utf8_lossy(&buffer[..bytes_read]);

    let lines: Vec<&str> = request_str.lines().collect();
    if lines.is_empty() {
        return;
    }
    let first_line = lines[0];
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 2 || parts[0] != "GET" {
        return;
    }

    let path = parts[1];

    if path.starts_with("/announce") {
        handle_announce(stream, path, peer_ip, state).await;
    } else {
        let response = "HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n";
        let _ = stream.write_all(response.as_bytes()).await;
    }
}

async fn handle_announce(
    mut stream: TcpStream,
    path: &str,
    ip: IpAddr,
    state: Arc<Mutex<TrackerState>>,
) {
    let url = match Url::parse(&format!("http://localhost{}", path)) {
        Ok(u) => u,
        Err(_) => {
            let response = "HTTP/1.1 400 Bad Request\r\n\r\nInvalid URL";
            let _ = stream.write_all(response.as_bytes()).await;
            return;
        }
    };

    let params: HashMap<_, _> = url.query_pairs().collect();

    let info_hash = match params.get("info_hash") {
        Some(h) => h.to_string(),
        None => {
            let response = "HTTP/1.1 400 Bad Request\r\n\r\nMissing info_hash";
            let _ = stream.write_all(response.as_bytes()).await;
            return;
        }
    };

    let port = params
        .get("port")
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(0);
    let peer_id = params
        .get("peer_id")
        .map(|id| id.to_string())
        .unwrap_or_default();

    let mut response_peers = Vec::new();
    {
        let mut guard = state.lock().await;
        let swarm = guard
            .torrents
            .entry(info_hash.clone())
            .or_insert_with(Vec::new);

        swarm.retain(|p| p.last_seen.elapsed() < Duration::from_secs(3600));

        let mut found = false;
        for peer in swarm.iter_mut() {
            if peer.id == peer_id {
                peer.last_seen = Instant::now();
                peer.ip = ip;
                peer.port = port;
                found = true;
                break;
            }
        }

        if !found {
            swarm.push(Peer {
                id: peer_id,
                ip,
                port,
                last_seen: Instant::now(),
            });
        }

        for p in swarm.iter().take(50) {
            response_peers.push(p.clone());
        }
    }

    let mut peers_bytes = Vec::new();
    for p in response_peers {
        if let IpAddr::V4(ipv4) = p.ip {
            peers_bytes.extend_from_slice(&ipv4.octets());
            peers_bytes.extend_from_slice(&p.port.to_be_bytes());
        }
    }

    use std::collections::BTreeMap;
    let mut resp_dict = BTreeMap::new();
    resp_dict.insert(b"interval".to_vec(), Bencode::Int(1800));
    resp_dict.insert(b"peers".to_vec(), Bencode::Bytes(peers_bytes));

    let resp_bencode = Bencode::Dict(resp_dict);
    let body = resp_bencode.encode();

    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );

    let _ = stream.write_all(header.as_bytes()).await;
    let _ = stream.write_all(&body).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tracker_state_peer_management() {
        let state = Arc::new(Mutex::new(TrackerState {
            torrents: HashMap::new(),
            rate_limits: HashMap::new(),
        }));

        let info_hash = "infohash1".to_string();
        let peer_id = "peer1".to_string();
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        let port = 6881;

        // Simulate announce
        {
            let mut guard = state.lock().await;
            let swarm = guard.torrents.entry(info_hash.clone()).or_insert_with(Vec::new);
            swarm.push(Peer {
                id: peer_id.clone(),
                ip,
                port,
                last_seen: Instant::now(),
            });
        }

        // Verify peer added
        {
            let guard = state.lock().await;
            let swarm = guard.torrents.get(&info_hash).expect("Swarm should exist");
            assert_eq!(swarm.len(), 1);
            assert_eq!(swarm[0].id, peer_id);
        }

        // Simulate duplicate announce (update existing)
        {
            let mut guard = state.lock().await;
            let swarm = guard.torrents.get_mut(&info_hash).unwrap();
            let mut found = false;
            for peer in swarm.iter_mut() {
                if peer.id == peer_id {
                    peer.port = 6882; // Changed port
                    found = true;
                    break;
                }
            }
            if !found {
                 swarm.push(Peer {
                    id: peer_id.clone(),
                    ip,
                    port: 6882,
                    last_seen: Instant::now(),
                });
            }
        }
        
         // Verify peer updated
        {
            let guard = state.lock().await;
            let swarm = guard.torrents.get(&info_hash).unwrap();
            assert_eq!(swarm.len(), 1);
            assert_eq!(swarm[0].port, 6882);
        }
    }
}
