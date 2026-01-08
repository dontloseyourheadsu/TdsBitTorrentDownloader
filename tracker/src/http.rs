//! HTTP Tracker Client implementation.

use super::{TrackerClient, TrackerEvent, TrackerRequest, TrackerResponse};
use std::net::{Ipv4Addr, SocketAddrV4};
use tds_core::bencoding::{decode, Bencode};

/// Client for communicating with HTTP/HTTPS trackers.
pub struct HttpTracker {
    url: String,
}

impl HttpTracker {
    /// Creates a new `HttpTracker`.
    ///
    /// # Arguments
    /// * `url` - The URL of the tracker announce endpoint.
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
        }
    }
}

impl TrackerClient for HttpTracker {
    /// Sends an announce request to the HTTP tracker.
    ///
    /// This uses a blocking HTTP request (reqwest::blocking) to contact the tracker.
    fn announce(&self, request: &TrackerRequest) -> Result<TrackerResponse, String> {
        let info_hash_encoded =
            form_urlencoded::byte_serialize(&request.info_hash).collect::<String>();
        let peer_id_encoded = form_urlencoded::byte_serialize(&request.peer_id).collect::<String>();

        let mut params = form_urlencoded::Serializer::new(String::new());
        params
            .append_pair("port", &request.port.to_string())
            .append_pair("uploaded", &request.uploaded.to_string())
            .append_pair("downloaded", &request.downloaded.to_string())
            .append_pair("left", &request.left.to_string())
            .append_pair("compact", if request.compact { "1" } else { "0" });

        if request.no_peer_id {
            params.append_pair("no_peer_id", "1");
        }

        if let Some(event) = request.event {
            let event_str = match event {
                TrackerEvent::Started => "started",
                TrackerEvent::Stopped => "stopped",
                TrackerEvent::Completed => "completed",
            };
            params.append_pair("event", event_str);
        }

        if let Some(numwant) = request.numwant {
            params.append_pair("numwant", &numwant.to_string());
        }

        if let Some(key) = request.key {
            params.append_pair("key", &key.to_string());
        }

        if let Some(tracker_id) = &request.tracker_id {
            params.append_pair("trackerid", tracker_id);
        }

        let params_str = params.finish();

        let separator = if self.url.contains('?') { "&" } else { "?" };
        let full_url = format!(
            "{}{separator}info_hash={}&peer_id={}&{}",
            self.url, info_hash_encoded, peer_id_encoded, params_str
        );

        let response = reqwest::blocking::get(&full_url).map_err(|e| e.to_string())?;
        let bytes = response.bytes().map_err(|e| e.to_string())?;

        let mut pos = 0;
        let bencode = decode(&bytes, &mut pos).map_err(|e| e.to_string())?;

        parse_http_response(bencode)
    }
}

fn parse_http_response(root: Bencode) -> Result<TrackerResponse, String> {
    if let Bencode::Dict(dict) = root {
        if let Some(Bencode::Bytes(failure)) = dict.get(&b"failure reason"[..]) {
            return Err(String::from_utf8_lossy(failure).to_string());
        }

        let interval = match dict.get(&b"interval"[..]) {
            Some(Bencode::Int(i)) => *i as u32,
            _ => return Err("Missing or invalid interval".to_string()),
        };

        let complete = match dict.get(&b"complete"[..]) {
            Some(Bencode::Int(i)) => Some(*i as u32),
            _ => None,
        };

        let incomplete = match dict.get(&b"incomplete"[..]) {
            Some(Bencode::Int(i)) => Some(*i as u32),
            _ => None,
        };

        let peers = match dict.get(&b"peers"[..]) {
            Some(Bencode::Bytes(b)) => {
                // Compact model
                let mut peers = Vec::new();
                for chunk in b.chunks(6) {
                    if chunk.len() == 6 {
                        let ip = Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3]);
                        let port = u16::from_be_bytes([chunk[4], chunk[5]]);
                        peers.push(SocketAddrV4::new(ip, port));
                    }
                }
                peers
            }
            Some(Bencode::List(l)) => {
                // Dictionary model
                let mut peers = Vec::new();
                for item in l {
                    if let Bencode::Dict(d) = item {
                        let ip_bytes = match d.get(&b"ip"[..]) {
                            Some(Bencode::Bytes(b)) => b,
                            _ => continue,
                        };
                        let ip_str = String::from_utf8_lossy(ip_bytes);
                        let ip: Ipv4Addr = match ip_str.parse() {
                            Ok(addr) => addr,
                            Err(_) => continue,
                        };

                        let port = match d.get(&b"port"[..]) {
                            Some(Bencode::Int(i)) => *i as u16,
                            _ => continue,
                        };
                        peers.push(SocketAddrV4::new(ip, port));
                    }
                }
                peers
            }
            _ => Vec::new(), // Some trackers might return empty peers or omit it if empty?
        };

        Ok(TrackerResponse {
            interval,
            peers,
            complete,
            incomplete,
        })
    } else {
        Err("Invalid response format".to_string())
    }
}
