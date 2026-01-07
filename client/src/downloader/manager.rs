use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::io::SeekFrom;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use tds_core::bencoding::{Bencode, decode};
use tds_core::rate_limit::TokenBucket;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::{Mutex, Semaphore, broadcast, mpsc};
use tracker::{TrackerEvent, TrackerRequest, get_tracker_client};

use super::state::{Downloader, PieceStatus};
use crate::dht::Dht;
use crate::peer::{Message, PeerConnection};

pub async fn run(downloader: &Downloader) {
    let mut tracker_urls = Vec::new();
    tracker_urls.push(downloader.torrent.announce.clone());
    if let Some(list) = &downloader.torrent.announce_list {
        for tier in list {
            for url in tier {
                if *url != downloader.torrent.announce {
                    tracker_urls.push(url.clone());
                }
            }
        }
    }

    let request = TrackerRequest {
        info_hash: downloader.torrent.info_hash,
        peer_id: downloader.peer_id,
        port: 6881,
        uploaded: 0,
        downloaded: 0,
        left: downloader.total_length - *downloader.downloaded_bytes.lock().await,
        compact: true,
        no_peer_id: false,
        event: Some(TrackerEvent::Started),
        ip: None,
        numwant: Some(50),
        key: None,
        tracker_id: None,
    };

    let (peer_tx, mut peer_rx) = mpsc::channel(100);

    // Tracker Discovery
    let tracker_tx = peer_tx.clone();
    let request_clone = request.clone();
    let tracker_urls_clone = tracker_urls.clone();
    tokio::spawn(async move {
        for url in tracker_urls_clone {
            println!("Contacting tracker: {}", url);
            let url_clone = url.clone();
            let req_clone = request_clone.clone();
            let res = tokio::task::spawn_blocking(move || {
                if let Some(client) = get_tracker_client(&url_clone) {
                    client.announce(&req_clone).ok()
                } else {
                    None
                }
            })
            .await
            .unwrap();

            if let Some(response) = res {
                println!(
                    "Tracker response from {}: {} peers",
                    url,
                    response.peers.len()
                );
                for peer in response.peers {
                    let _ = tracker_tx.send(peer).await;
                }
            }
        }
    });

    // DHT Discovery
    let dht_tx = peer_tx.clone();
    let info_hash = downloader.torrent.info_hash;
    tokio::spawn(async move {
        match Dht::new(6882).await {
            Ok(dht) => {
                println!("DHT started on port 6882");
                dht.start().await;
                dht.bootstrap().await;

                loop {
                    dht.get_peers(info_hash).await;
                    let peers = dht.get_found_peers().await;
                    if !peers.is_empty() {
                        println!("DHT found {} peers", peers.len());
                        for peer in peers {
                            let _ = dht_tx.send(peer).await;
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                }
            }
            Err(e) => eprintln!("Failed to start DHT: {}", e),
        }
    });

    let mut handles = Vec::new();
    let (tx, _) = broadcast::channel(1);
    let mut completion_rx = tx.subscribe();
    let upload_limiter = Arc::new(Mutex::new(TokenBucket::new(2_000_000.0, 2_000_000.0))); // 2 MB/s
    let uploaded_total = downloader.uploaded_bytes.clone();
    let downloaded_total = downloader.downloaded_bytes.clone();
    let semaphore = Arc::new(Semaphore::new(50));
    let connected_peers = Arc::new(Mutex::new(std::collections::HashSet::new()));

    loop {
        tokio::select! {
            res = peer_rx.recv() => {
                if let Some(peer_addr) = res {
                    let mut connected = connected_peers.lock().await;
                    if connected.contains(&peer_addr) {
                        continue;
                    }
                    connected.insert(peer_addr);
                    drop(connected);

                    let piece_status = downloader.piece_status.clone();
                    let file = downloader.file.clone();
                    let torrent = downloader.torrent.clone();
                    let peer_id = downloader.peer_id;
                    let mut rx = tx.subscribe();
                    let tx = tx.clone();
                    let new_peer_tx = peer_tx.clone();
                    let uploaded_total = uploaded_total.clone();
                    let downloaded_total = downloaded_total.clone();
                    let semaphore = semaphore.clone();
                    let connected_peers = connected_peers.clone();
                    let upload_limiter = upload_limiter.clone();
                    let total_length = downloader.total_length;
                    let piece_count = torrent.pieces.len();

                    handles.push(tokio::spawn(async move {
                        let _permit = semaphore.acquire_owned().await.unwrap();
                        println!("Connecting to {}", peer_addr);

                        let mut peer =
                            match PeerConnection::connect(peer_addr, &torrent.info_hash, &peer_id).await {
                                Ok(p) => p,
                                Err(e) => {
                                    eprintln!("Failed to connect to {}: {}", peer_addr, e);
                                    connected_peers.lock().await.remove(&peer_addr);
                                    return;
                                }
                            };
                        println!("Connected to {}", peer_addr);

                        {
                            let status = piece_status.lock().await;
                            if status.iter().any(|&s| s == PieceStatus::Have) {
                                let mut bitfield = vec![0u8; (status.len() + 7) / 8];
                                for (i, s) in status.iter().enumerate() {
                                    if *s == PieceStatus::Have {
                                        let byte_idx = i / 8;
                                        let bit_idx = 7 - (i % 8);
                                        bitfield[byte_idx] |= 1 << bit_idx;
                                    }
                                }
                                if let Err(e) = peer.send_message(Message::Bitfield(bitfield)).await {
                                    eprintln!("Error sending bitfield to {}: {}", peer_addr, e);
                                    return;
                                }
                            }
                        }

                        if let Err(e) = peer.send_message(Message::Interested).await {
                            eprintln!("Error sending interested to {}: {}", peer_addr, e);
                            return;
                        }

                        // Send Extended Handshake
                        let mut m = BTreeMap::new();
                        m.insert(b"ut_pex".to_vec(), Bencode::Int(1));
                        let mut handshake = BTreeMap::new();
                        handshake.insert(b"m".to_vec(), Bencode::Dict(m));
                        let payload = Bencode::Dict(handshake).encode();
                        if let Err(e) = peer.send_message(Message::Extended { id: 0, payload }).await {
                            eprintln!("Error sending extended handshake to {}: {}", peer_addr, e);
                        }

                        let mut peer_pex_id = None;
                        let mut current_piece_idx: Option<usize> = None;
                        let mut current_piece_data: Vec<u8> = Vec::new();
                        let mut uploaded_session: u64 = 0;
                        let mut blocks_received: usize = 0;
                        let mut blocks_total: usize = 0;

                        loop {
                            let msg = tokio::select! {
                                res = peer.read_message() => {
                                    match res {
                                        Ok(m) => m,
                                        Err(e) => {
                                            eprintln!("Error reading from {}: {}", peer_addr, e);
                                            if let Some(idx) = current_piece_idx {
                                                let mut status = piece_status.lock().await;
                                                if status[idx] == PieceStatus::InProgress {
                                                    status[idx] = PieceStatus::Missing;
                                                }
                                            }
                                            break;
                                        }
                                    }
                                }
                                _ = rx.recv() => {
                                    break;
                                }
                            };

                            match msg {
                                Message::Unchoke => {
                                    println!("{} unchoked us", peer_addr);
                                }
                                Message::Request { index, begin, length } => {
                                    if length > 128 * 1024 {
                                        eprintln!("Requested block too large: {}", length);
                                        continue;
                                    }

                                    let status = piece_status.lock().await;
                                    if status.get(index as usize).map(|&s| s == PieceStatus::Have).unwrap_or(false) {
                                        drop(status);

                                        let mut bucket = upload_limiter.lock().await;
                                        while !bucket.consume(length as f64) {
                                            drop(bucket);
                                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                                            bucket = upload_limiter.lock().await;
                                        }
                                        drop(bucket);

                                        let mut f = file.lock().await;
                                        let offset = (index as u64 * torrent.piece_length) + begin as u64;
                                        if let Err(e) = f.seek(SeekFrom::Start(offset)).await {
                                            eprintln!("Seek error: {}", e);
                                            continue;
                                        }
                                        let mut block = vec![0u8; length as usize];
                                        if let Err(e) = f.read_exact(&mut block).await {
                                            eprintln!("Read error: {}", e);
                                            continue;
                                        }
                                        drop(f);

                                        if let Err(e) = peer.send_message(Message::Piece { index, begin, block }).await {
                                            eprintln!("Error sending piece to {}: {}", peer_addr, e);
                                            break;
                                        }

                                        let mut uploaded = uploaded_total.lock().await;
                                        *uploaded += length as u64;
                                        uploaded_session += length as u64;
                                        println!(
                                            "Uploaded {} bytes to {} (Session: {}, Total: {})",
                                            length, peer_addr, uploaded_session, *uploaded
                                        );
                                    }
                                }
                                Message::Cancel { .. } => {
                                    // TODO: Implement cancel
                                }
                                Message::Piece {
                                    index,
                                    begin,
                                    block,
                                } => {
                                    if let Some(curr) = current_piece_idx {
                                        if curr == index as usize {
                                            let begin = begin as usize;
                                            if begin + block.len() <= current_piece_data.len() {
                                                current_piece_data[begin..begin + block.len()]
                                                    .copy_from_slice(&block);
                                                blocks_received += 1;

                                                if blocks_received == blocks_total {
                                                    let mut hasher = Sha1::new();
                                                    hasher.update(&current_piece_data);
                                                    let hash = hasher.finalize();

                                                    if hash.as_slice() == &torrent.pieces[curr] {
                                                        println!("Piece {} verified from {}!", curr, peer_addr);
                                                        let mut f = file.lock().await;
                                                        let offset = curr as u64 * torrent.piece_length;
                                                        if let Err(e) = f.seek(SeekFrom::Start(offset)).await {
                                                            eprintln!("Seek error: {}", e);
                                                            break;
                                                        }
                                                        if let Err(e) = f.write_all(&current_piece_data).await {
                                                            eprintln!("Write error: {}", e);
                                                            break;
                                                        }

                                                        let mut status = piece_status.lock().await;
                                                        status[curr] = PieceStatus::Have;

                                                        let mut d_total = downloaded_total.lock().await;
                                                        *d_total += current_piece_data.len() as u64;
                                                        println!(
                                                            "Downloaded piece {} from {} (Total: {})",
                                                            curr, peer_addr, *d_total
                                                        );

                                                        current_piece_idx = None;

                                                        if let Err(e) =
                                                            peer.send_message(Message::Have(curr as u32)).await
                                                        {
                                                            eprintln!(
                                                                "Error sending Have to {}: {}",
                                                                peer_addr, e
                                                            );
                                                        }
                                                    } else {
                                                        eprintln!(
                                                            "Piece {} hash mismatch from {}!",
                                                            curr, peer_addr
                                                        );
                                                        let mut status = piece_status.lock().await;
                                                        status[curr] = PieceStatus::Missing;
                                                        current_piece_idx = None;
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                Message::Extended { id, payload } => {
                                    if id == 0 {
                                        let mut pos = 0;
                                        if let Ok(Bencode::Dict(dict)) = decode(&payload, &mut pos) {
                                            if let Some(Bencode::Dict(m)) = dict.get(&b"m"[..]) {
                                                if let Some(Bencode::Int(pex_id)) = m.get(&b"ut_pex"[..]) {
                                                    peer_pex_id = Some(*pex_id as u8);
                                                    println!("Peer {} supports PEX with ID {}", peer_addr, pex_id);
                                                }
                                            }
                                        }
                                    } else if Some(id) == peer_pex_id {
                                        let mut pos = 0;
                                        if let Ok(Bencode::Dict(dict)) = decode(&payload, &mut pos) {
                                            if let Some(Bencode::Bytes(added)) = dict.get(&b"added"[..]) {
                                                for chunk in added.chunks(6) {
                                                    if chunk.len() == 6 {
                                                        let ip = Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3]);
                                                        let port = u16::from_be_bytes([chunk[4], chunk[5]]);
                                                        let addr = SocketAddrV4::new(ip, port);
                                                        println!("PEX found peer: {}", addr);
                                                        let _ = new_peer_tx.send(addr).await;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                _ => {}
                            }

                            if !peer.peer_choking && current_piece_idx.is_none() {
                                let mut idx = None;
                                {
                                    let mut status = piece_status.lock().await;
                                    if status.iter().all(|&s| s == PieceStatus::Have) {
                                        println!("All pieces downloaded!");
                                        let _ = tx.send(());
                                        break;
                                    }

                                    let mut available_pieces = Vec::new();
                                    for (i, s) in status.iter().enumerate() {
                                        if *s == PieceStatus::Missing {
                                            if peer.has_piece(i as u32) {
                                                available_pieces.push(i);
                                            }
                                        }
                                    }

                                    if !available_pieces.is_empty() {
                                        use rand::Rng;
                                        let mut rng = rand::rng();
                                        let random_idx = rng.random_range(0..available_pieces.len());
                                        let i = available_pieces[random_idx];
                                        status[i] = PieceStatus::InProgress;
                                        idx = Some(i);
                                    }
                                }

                                if let Some(i) = idx {
                                    current_piece_idx = Some(i);
                                    let p_len = if i == piece_count - 1 {
                                        let rem = total_length % torrent.piece_length;
                                        if rem == 0 { torrent.piece_length } else { rem }
                                    } else {
                                        torrent.piece_length
                                    };
                                    current_piece_data = vec![0u8; p_len as usize];

                                    let block_size = 16384;
                                    blocks_total = (p_len as usize + block_size - 1) / block_size;
                                    blocks_received = 0;

                                    for b in 0..blocks_total {
                                        let begin = b * block_size;
                                        let len = if begin + block_size > p_len as usize {
                                            p_len as usize - begin
                                        } else {
                                            block_size
                                        };
                                        if let Err(e) = peer
                                            .send_message(Message::Request {
                                                index: i as u32,
                                                begin: begin as u32,
                                                length: len as u32,
                                            })
                                            .await
                                        {
                                            eprintln!("Error sending request to {}: {}", peer_addr, e);
                                            let mut status = piece_status.lock().await;
                                            status[i] = PieceStatus::Missing;
                                            current_piece_idx = None;
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        connected_peers.lock().await.remove(&peer_addr);
                    }));
                }
            }
            _ = completion_rx.recv() => {
                println!("All pieces downloaded! Stopping.");
                break;
            }
            _ = tokio::signal::ctrl_c() => {
                println!("Ctrl+C received, shutting down.");
                break;
            }
        }
    }
}
