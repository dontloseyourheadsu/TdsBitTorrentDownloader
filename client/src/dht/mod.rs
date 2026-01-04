use rand::Rng;
use std::collections::{BTreeMap, HashMap};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use tds_core::bencoding::{Bencode, decode};
use tokio::net::UdpSocket;
use tokio::sync::Mutex;

#[derive(Clone, Debug)]
pub struct Node {
    pub id: [u8; 20],
    pub addr: SocketAddr,
}

pub struct Dht {
    socket: Arc<UdpSocket>,
    node_id: [u8; 20],
    nodes: Arc<Mutex<Vec<Node>>>,
    peers: Arc<Mutex<Vec<SocketAddrV4>>>,
    transactions: Arc<Mutex<HashMap<Vec<u8>, String>>>, // transaction_id -> query_type
}

impl Dht {
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
                        eprintln!("DHT UDP read error: {}", e);
                    }
                }
            }
        });
    }

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

    pub async fn get_found_peers(&self) -> Vec<SocketAddrV4> {
        let mut guard = self.peers.lock().await;
        let peers = guard.clone();
        guard.clear();
        peers
    }
}
