use rand::Rng;
use std::collections::{BTreeMap, HashMap};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use tds_core::bencoding::{Bencode, decode};
use tokio::net::UdpSocket;
use tokio::sync::Mutex;

/// Represents a node in the DHT network.
#[derive(Clone, Debug)]
pub struct Node {
    /// The 20-byte Node ID.
    pub id: [u8; 20],
    /// The socket address of the node.
    pub addr: SocketAddr,
}

/// A simpler implementation of a Distributed Hash Table (DHT) node (Kademlia-like).
///
/// This struct manages the UDP socket for DHT communication, maintains a routing table
/// (list of nodes), and handles peer discovery via `get_peers` and `find_node` queries.
///
/// Note: This is a partial implementation focusing on bootstrapping and basic peer discovery.
pub struct Dht {
    /// The UDP socket used for messaging.
    socket: Arc<UdpSocket>,
    /// Our own Node ID (randomly generated).
    node_id: [u8; 20],
    /// Known DHT nodes (routing table).
    nodes: Arc<Mutex<Vec<Node>>>,
    /// Discovered peers (IP:Port of peers that have the infohash we are looking for).
    peers: Arc<Mutex<Vec<SocketAddrV4>>>,
    /// Active transactions to map responses to queries (Transaction ID -> Query Type).
    transactions: Arc<Mutex<HashMap<Vec<u8>, String>>>, 
}

impl Dht {
    /// Creates a new `Dht` node bound to the specified port.
    ///
    /// # Arguments
    ///
    /// * `port` - The UDP port to bind to. Use `0` to let the OS choose a random port.
    ///
    /// # Returns
    ///
    /// * `Result<Self, ...>` - The created DHT node or an error if binding fails.
    pub async fn new(port: u16) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let socket = UdpSocket::bind(format!("0.0.0.0:{}", port)).await?;
        let mut rng = rand::rng();
        let mut node_id = [0u8; 20];
        rng.fill(&mut node_id);

        Ok(Self {
            socket: Arc::new(socket),
            node_id,
            nodes: Arc::new(Mutex::new(Vec::new())),
            peers: Arc::new(Mutex::new(Vec::new())),
            transactions: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Starts the DHT node's listening loop in a background task.
    ///
    /// This task listens for incoming UDP messages, parses them, and updates
    /// the internal state (nodes and peers) or responds to queries (ping).
    pub async fn start(&self) {
        let socket = self.socket.clone();
        let nodes = self.nodes.clone();
        let peers = self.peers.clone();
        let transactions = self.transactions.clone();
        let my_id = self.node_id;

        tokio::spawn(async move {
            let mut buf = [0u8; 65536];
            loop {
                match socket.recv_from(&mut buf).await {
                    Ok((len, src)) => {
                        let data = &buf[..len];
                        if let Ok(bencode) = decode(data, &mut 0) {
                            Self::handle_message(
                                bencode,
                                src,
                                &nodes,
                                &peers,
                                &transactions,
                                &socket,
                                my_id,
                            )
                            .await;
                        }
                    }
                    Err(e) => {
                        // eprintln!("DHT UDP read error: {}", e);
                    }
                }
            }
        });
    }

    /// Handles an incoming decoded KRPC message.
    ///
    /// Dispatches based on message type ('y'):
    /// * 'r' (response): Updates routing table or peer list.
    /// * 'q' (query): Responds to pings.
    async fn handle_message(
        msg: Bencode,
        src: SocketAddr,
        nodes: &Arc<Mutex<Vec<Node>>>,
        peers: &Arc<Mutex<Vec<SocketAddrV4>>>,
        _transactions: &Arc<Mutex<HashMap<Vec<u8>, String>>>,
        socket: &Arc<UdpSocket>,
        my_id: [u8; 20],
    ) {
        if let Bencode::Dict(dict) = msg {
            // Check transaction ID
            let t = match dict.get(&b"t"[..]) {
                Some(Bencode::Bytes(b)) => b.clone(),
                _ => return,
            };

            // Check message type
            let y = match dict.get(&b"y"[..]) {
                Some(Bencode::Bytes(b)) => b,
                _ => return,
            };

            if y == b"r" {
                // Response
                if let Some(Bencode::Dict(r)) = dict.get(&b"r"[..]) {
                    // Extract nodes or peers
                    if let Some(Bencode::Bytes(nodes_bytes)) = r.get(&b"nodes"[..]) {
                        Self::parse_nodes(nodes_bytes, nodes).await;
                    }
                    if let Some(Bencode::List(values)) = r.get(&b"values"[..]) {
                        Self::parse_peers(values, peers).await;
                    }
                }
            } else if y == b"q" {
                // Query (we should respond to ping at least)
                if let Some(Bencode::Bytes(q)) = dict.get(&b"q"[..]) {
                    if q == b"ping" {
                        Self::send_ping_response(socket, src, &t, my_id).await;
                    }
                }
            }
        }
    }

    /// Parses the compact node info string (26 bytes per node) and updates the routing table.
    async fn parse_nodes(data: &[u8], nodes: &Arc<Mutex<Vec<Node>>>) {
        // Each node is 26 bytes: 20 bytes ID + 6 bytes IP/Port
        let mut guard = nodes.lock().await;
        for chunk in data.chunks(26) {
            if chunk.len() == 26 {
                let mut id = [0u8; 20];
                id.copy_from_slice(&chunk[0..20]);
                let ip = Ipv4Addr::new(chunk[20], chunk[21], chunk[22], chunk[23]);
                let port = u16::from_be_bytes([chunk[24], chunk[25]]);
                let addr = SocketAddr::V4(SocketAddrV4::new(ip, port));

                // Simple add if not exists
                if !guard.iter().any(|n| n.addr == addr) {
                    guard.push(Node { id, addr });
                }
            }
        }
    }

    /// Parses a list of compact peer info strings (6 bytes per peer) and updates the peer list.
    async fn parse_peers(values: &[Bencode], peers: &Arc<Mutex<Vec<SocketAddrV4>>>) {
        let mut guard = peers.lock().await;
        for val in values {
            if let Bencode::Bytes(b) = val {
                if b.len() == 6 {
                    let ip = Ipv4Addr::new(b[0], b[1], b[2], b[3]);
                    let port = u16::from_be_bytes([b[4], b[5]]);
                    let addr = SocketAddrV4::new(ip, port);
                    if !guard.contains(&addr) {
                        guard.push(addr);
                    }
                }
            }
        }
    }

    /// Sends a 'ping' response.
    ///
    /// # Arguments
    ///
    /// * `socket` - The UDP socket.
    /// * `to` - The address of the pinging node.
    /// * `t` - The transaction ID from the query.
    /// * `my_id` - Our node ID.
    async fn send_ping_response(
        socket: &Arc<UdpSocket>,
        to: SocketAddr,
        t: &[u8],
        my_id: [u8; 20],
    ) {
        let mut dict = BTreeMap::new();
        dict.insert(b"t".to_vec(), Bencode::Bytes(t.to_vec()));
        dict.insert(b"y".to_vec(), Bencode::Bytes(b"r".to_vec()));

        let mut r = BTreeMap::new();
        r.insert(b"id".to_vec(), Bencode::Bytes(my_id.to_vec()));
        dict.insert(b"r".to_vec(), Bencode::Dict(r));

        let msg = Bencode::Dict(dict).encode();
        let _ = socket.send_to(&msg, to).await;
    }

    /// Bootstraps the DHT by querying known public bootstrap nodes.
    ///
    /// This populates the routing table with initial nodes.
    pub async fn bootstrap(&self) {
        let routers = vec![
            "router.bittorrent.com:6881",
            "dht.transmissionbt.com:6881",
            "router.utorrent.com:6881",
        ];

        for router in routers {
            if let Ok(addrs) = tokio::net::lookup_host(router).await {
                for addr in addrs {
                    self.find_node(addr, self.node_id).await;
                }
            }
        }
    }

    /// Sends `get_peers` queries to all known nodes in the routing table for the given info hash.
    ///
    /// # Arguments
    ///
    /// * `info_hash` - The target info hash to find peers for.
    pub async fn get_peers(&self, info_hash: [u8; 20]) {
        // Query all known nodes for peers
        // In a real implementation, we would query closest nodes iteratively
        let nodes = {
            let guard = self.nodes.lock().await;
            guard.clone()
        };

        for node in nodes {
            self.send_get_peers(node.addr, info_hash).await;
        }
    }

    /// Sends a `find_node` query to a specific address.
    async fn find_node(&self, addr: SocketAddr, target: [u8; 20]) {
        let t: [u8; 2] = {
            let mut rng = rand::rng();
            rng.random()
        };

        let mut dict = BTreeMap::new();
        dict.insert(b"t".to_vec(), Bencode::Bytes(t.to_vec()));
        dict.insert(b"y".to_vec(), Bencode::Bytes(b"q".to_vec()));
        dict.insert(b"q".to_vec(), Bencode::Bytes(b"find_node".to_vec()));

        let mut a = BTreeMap::new();
        a.insert(b"id".to_vec(), Bencode::Bytes(self.node_id.to_vec()));
        a.insert(b"target".to_vec(), Bencode::Bytes(target.to_vec()));
        dict.insert(b"a".to_vec(), Bencode::Dict(a));

        let msg = Bencode::Dict(dict).encode();
        let _ = self.socket.send_to(&msg, addr).await;
    }

    /// Sends a `get_peers` query to a specific address.
    async fn send_get_peers(&self, addr: SocketAddr, info_hash: [u8; 20]) {
        let t: [u8; 2] = {
            let mut rng = rand::rng();
            rng.random()
        };

        let mut dict = BTreeMap::new();
        dict.insert(b"t".to_vec(), Bencode::Bytes(t.to_vec()));
        dict.insert(b"y".to_vec(), Bencode::Bytes(b"q".to_vec()));
        dict.insert(b"q".to_vec(), Bencode::Bytes(b"get_peers".to_vec()));

        let mut a = BTreeMap::new();
        a.insert(b"id".to_vec(), Bencode::Bytes(self.node_id.to_vec()));
        a.insert(b"info_hash".to_vec(), Bencode::Bytes(info_hash.to_vec()));
        dict.insert(b"a".to_vec(), Bencode::Dict(a));

        let msg = Bencode::Dict(dict).encode();
        let _ = self.socket.send_to(&msg, addr).await;
    }

    /// Retrieves and clears the list of newly discovered peers.
    ///
    /// # Returns
    ///
    /// * `Vec<SocketAddrV4>` - A list of peer addresses.
    pub async fn get_found_peers(&self) -> Vec<SocketAddrV4> {
        let mut guard = self.peers.lock().await;
        let peers = guard.clone();
        guard.clear();
        peers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parse_nodes() {
         // Construct 26 bytes of node info
         // 20 bytes ID (all 1s)
         // 4 bytes IP (127.0.0.1)
         // 2 bytes Port (8080)
         let mut data = vec![1u8; 20];
         data.extend_from_slice(&[127, 0, 0, 1]);
         data.extend_from_slice(&8080u16.to_be_bytes());
         
         let nodes = Arc::new(Mutex::new(Vec::new()));
         Dht::parse_nodes(&data, &nodes).await;
         
         let guard = nodes.lock().await;
         assert_eq!(guard.len(), 1);
         assert_eq!(guard[0].id, [1u8; 20]);
         if let SocketAddr::V4(v4) = guard[0].addr {
             assert_eq!(v4.ip().to_string(), "127.0.0.1");
             assert_eq!(v4.port(), 8080);
         } else {
             panic!("Address is not V4");
         }
    }

    #[tokio::test]
    async fn test_parse_peers() {
         // 6 bytes compact info
         // 1.1.1.1:6969
         let data = vec![1, 1, 1, 1, 0x1B, 0x39]; 
         let bencode_val = Bencode::Bytes(data);
         let list = vec![bencode_val];
         
         let peers = Arc::new(Mutex::new(Vec::new()));
         Dht::parse_peers(&list, &peers).await;
         
         let guard = peers.lock().await;
         assert_eq!(guard.len(), 1);
         assert_eq!(guard[0].to_string(), "1.1.1.1:6969");
    }
}
