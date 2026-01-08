//! Shared definitions and traits for BitTorrent Tracker interaction.
//!
//! This library provides the `TrackerClient` trait for communicating with trackers,
//! as well as data structures like `TrackerRequest` and `TrackerResponse`.
//! It also includes a factory function `get_tracker_client` to instantiate the correct client based on the URL scheme.

use std::net::SocketAddrV4;

pub mod http;
pub mod server;
pub mod udp;

use http::HttpTracker;
use udp::UdpTracker;

/// Represents a request to a tracker.
#[derive(Debug, Clone)]
pub struct TrackerRequest {
    /// The Info Hash of the torrent.
    pub info_hash: [u8; 20],
    /// The client's Peer ID.
    pub peer_id: [u8; 20],
    /// The port the client is listening on.
    pub port: u16,
    /// Total amount uploaded so far.
    pub uploaded: u64,
    /// Total amount downloaded so far.
    pub downloaded: u64,
    /// Number of bytes left to download.
    pub left: u64,
    /// Whether to request a compact peer list.
    pub compact: bool,
    /// Whether to omit the peer id in the response.
    pub no_peer_id: bool,
    /// Optional event (started, stopped, completed).
    pub event: Option<TrackerEvent>,
    /// Optional IP address of the client.
    pub ip: Option<std::net::IpAddr>,
    /// Number of peers the client would like to receive.
    pub numwant: Option<u32>,
    /// Optional key for identity verification.
    pub key: Option<u32>,
    /// Optional tracker ID.
    pub tracker_id: Option<String>,
}

/// Events sent to the tracker.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrackerEvent {
    /// The download has just started.
    Started,
    /// The download has been stopped.
    Stopped,
    /// The download has completed.
    Completed,
}

/// Represents a response from a tracker.
#[derive(Debug, Clone)]
pub struct TrackerResponse {
    /// The interval in seconds the client should wait before sending the next request.
    pub interval: u32,
    /// List of peers received from the tracker.
    pub peers: Vec<SocketAddrV4>,
    /// Number of seeders (complete peers).
    pub complete: Option<u32>,
    /// Number of leechers (incomplete peers).
    pub incomplete: Option<u32>,
}

/// Trait for a Tracker Client.
pub trait TrackerClient {
    /// Sends an announce request to the tracker.
    fn announce(&self, request: &TrackerRequest) -> Result<TrackerResponse, String>;
}

/// Factory function to create a Tracker Client based on the URL.
///
/// Supports 'http(s)' and 'udp' schemes.
///
/// # Arguments
/// * `url` - The tracker URL.
///
/// # Returns
/// An `Option` containing a boxed `TrackerClient` if the scheme is supported.
pub fn get_tracker_client(url: &str) -> Option<Box<dyn TrackerClient>> {
    if url.starts_with("http://") || url.starts_with("https://") {
        Some(Box::new(HttpTracker::new(url)))
    } else if url.starts_with("udp://") {
        Some(Box::new(UdpTracker::new(url)))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_tracker_client_http() {
        let client = get_tracker_client("http://example.com/announce");
        assert!(client.is_some());
    }

    #[test]
    fn test_get_tracker_client_https() {
        let client = get_tracker_client("https://example.com/announce");
        assert!(client.is_some());
    }

    #[test]
    fn test_get_tracker_client_udp() {
        let client = get_tracker_client("udp://example.com:80");
        assert!(client.is_some());
    }

    #[test]
    fn test_get_tracker_client_unsupported() {
        let client = get_tracker_client("ftp://example.com");
        assert!(client.is_none());
    }
}
