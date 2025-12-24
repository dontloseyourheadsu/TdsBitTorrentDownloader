use std::net::SocketAddrV4;

pub mod http;
pub mod udp;

use http::HttpTracker;
use udp::UdpTracker;

#[derive(Debug, Clone)]
pub struct TrackerRequest {
    pub info_hash: [u8; 20],
    pub peer_id: [u8; 20],
    pub port: u16,
    pub uploaded: u64,
    pub downloaded: u64,
    pub left: u64,
    pub compact: bool,
    pub no_peer_id: bool,
    pub event: Option<TrackerEvent>,
    pub ip: Option<std::net::IpAddr>,
    pub numwant: Option<u32>,
    pub key: Option<u32>,
    pub tracker_id: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum TrackerEvent {
    Started,
    Stopped,
    Completed,
}

#[derive(Debug, Clone)]
pub struct TrackerResponse {
    pub interval: u32,
    pub peers: Vec<SocketAddrV4>,
    pub complete: Option<u32>,   // seeders
    pub incomplete: Option<u32>, // leechers
}

pub trait TrackerClient {
    fn announce(&self, request: &TrackerRequest) -> Result<TrackerResponse, String>;
}

pub fn get_tracker_client(url: &str) -> Option<Box<dyn TrackerClient>> {
    if url.starts_with("http://") || url.starts_with("https://") {
        Some(Box::new(HttpTracker::new(url)))
    } else if url.starts_with("udp://") {
        Some(Box::new(UdpTracker::new(url)))
    } else {
        None
    }
}
