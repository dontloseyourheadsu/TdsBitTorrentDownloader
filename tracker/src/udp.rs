//! UDP Tracker Client implementation.

use super::{TrackerClient, TrackerEvent, TrackerRequest, TrackerResponse};
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use rand::Rng;
use std::io::{Cursor, Write};
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::Duration;

/// Client for communicating with UDP trackers (BEP 15).
pub struct UdpTracker {
    url: String,
}

impl UdpTracker {
    /// Creates a new `UdpTracker`.
    ///
    /// # Arguments
    /// * `url` - The UDP URL of the tracker (e.g., `udp://tracker.opentrackr.org:1337`).
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
        }
    }
}

impl TrackerClient for UdpTracker {
    /// Sends an announce request to the UDP tracker.
    ///
    /// Implementation details:
    /// 1. Sends a Connect Request.
    /// 2. Receives a Connect Response with a Connection ID.
    /// 3. Sends an Announce Request using the Connection ID.
    /// 4. Receives an Announce Response.
    fn announce(&self, request: &TrackerRequest) -> Result<TrackerResponse, String> {
        // Parse URL to get host and port
        let url_parsed = url::Url::parse(&self.url).map_err(|e| e.to_string())?;
        let host = url_parsed.host_str().ok_or("Missing host")?;
        let port = url_parsed.port().ok_or("Missing port")?;
        let addr_str = format!("{}:{}", host, port);

        let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
        socket
            .set_read_timeout(Some(Duration::from_secs(15)))
            .map_err(|e| e.to_string())?;
        socket.connect(&addr_str).map_err(|e| e.to_string())?;

        let mut rng = rand::rng();
        let transaction_id: u32 = rng.random();

        // 1. Connect
        let protocol_id: u64 = 0x41727101980;
        let action_connect: u32 = 0;

        let mut connect_req = Vec::new();
        connect_req.write_u64::<BigEndian>(protocol_id).unwrap();
        connect_req.write_u32::<BigEndian>(action_connect).unwrap();
        connect_req.write_u32::<BigEndian>(transaction_id).unwrap();

        socket.send(&connect_req).map_err(|e| e.to_string())?;

        let mut buf = [0u8; 16];
        let (amt, _) = socket.recv_from(&mut buf).map_err(|e| e.to_string())?;
        if amt < 16 {
            return Err("Invalid connect response size".to_string());
        }

        let mut rdr = Cursor::new(&buf[..amt]);
        let action = rdr.read_u32::<BigEndian>().unwrap();
        let res_transaction_id = rdr.read_u32::<BigEndian>().unwrap();

        if res_transaction_id != transaction_id {
            return Err("Transaction ID mismatch".to_string());
        }
        if action != 0 {
            return Err(format!("Expected action 0, got {}", action));
        }

        let connection_id = rdr.read_u64::<BigEndian>().unwrap();

        // 2. Announce
        let action_announce: u32 = 1;
        let transaction_id: u32 = rng.random(); // New transaction ID

        let mut announce_req = Vec::new();
        announce_req.write_u64::<BigEndian>(connection_id).unwrap();
        announce_req
            .write_u32::<BigEndian>(action_announce)
            .unwrap();
        announce_req.write_u32::<BigEndian>(transaction_id).unwrap();
        announce_req.write_all(&request.info_hash).unwrap();
        announce_req.write_all(&request.peer_id).unwrap();
        announce_req
            .write_u64::<BigEndian>(request.downloaded)
            .unwrap();
        announce_req.write_u64::<BigEndian>(request.left).unwrap();
        announce_req
            .write_u64::<BigEndian>(request.uploaded)
            .unwrap();

        let event_id = match request.event {
            None => 0,
            Some(TrackerEvent::Completed) => 1,
            Some(TrackerEvent::Started) => 2,
            Some(TrackerEvent::Stopped) => 3,
        };
        announce_req.write_u32::<BigEndian>(event_id).unwrap();

        announce_req.write_u32::<BigEndian>(0).unwrap(); // IP address (0 default)
        let key: u32 = rng.random();
        announce_req.write_u32::<BigEndian>(key).unwrap();
        announce_req.write_i32::<BigEndian>(-1).unwrap(); // num_want (-1 default)
        announce_req.write_u16::<BigEndian>(request.port).unwrap();

        socket.send(&announce_req).map_err(|e| e.to_string())?;

        let mut buf = [0u8; 4096]; // Larger buffer for peers
        let (amt, _) = socket.recv_from(&mut buf).map_err(|e| e.to_string())?;

        if amt < 20 {
            return Err("Invalid announce response size".to_string());
        }

        let mut rdr = Cursor::new(&buf[..amt]);
        let action = rdr.read_u32::<BigEndian>().unwrap();
        let res_transaction_id = rdr.read_u32::<BigEndian>().unwrap();

        if res_transaction_id != transaction_id {
            return Err("Transaction ID mismatch in announce".to_string());
        }

        if action == 3 {
            // Error
            let msg_bytes = &buf[8..amt];
            let msg = String::from_utf8_lossy(msg_bytes);
            return Err(format!("Tracker error: {}", msg));
        }

        if action != 1 {
            return Err(format!("Expected action 1, got {}", action));
        }

        let interval = rdr.read_u32::<BigEndian>().unwrap();
        let leechers = rdr.read_u32::<BigEndian>().unwrap();
        let seeders = rdr.read_u32::<BigEndian>().unwrap();

        let mut peers = Vec::new();
        while rdr.position() < amt as u64 {
            if amt as u64 - rdr.position() < 6 {
                break;
            }
            let ip_int = rdr.read_u32::<BigEndian>().unwrap();
            let port = rdr.read_u16::<BigEndian>().unwrap();
            let ip = Ipv4Addr::from(ip_int);
            peers.push(SocketAddrV4::new(ip, port));
        }

        Ok(TrackerResponse {
            interval,
            peers,
            complete: Some(seeders),
            incomplete: Some(leechers),
        })
    }
}
