use clap::Parser;
use rand::Rng;
use sha1::{Digest, Sha1};
use std::io::SeekFrom;
use std::sync::Arc;
use tds_core::parse_torrent;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::{Mutex, Semaphore, broadcast, mpsc};
use tracker::{TrackerEvent, TrackerRequest, get_tracker_client};

mod peer;
use peer::{Message, PeerConnection};
mod storage;
use storage::Storage;
mod dht;
use dht::Dht;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the torrent file
    #[arg(short, long, default_value = "example.torrent")]
    torrent: String,

    /// Output directory for downloaded files
    #[arg(short, long)]
    output: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum PieceStatus {
    Missing,
    InProgress,
    Have,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let storage = match Storage::new(args.output).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to initialize storage: {}", e);
            return;
        }
    };
    println!("Download directory: {}", storage.get_download_dir_str());

    let torrent = match parse_torrent(&args.torrent) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Error parsing torrent: {}", e);
            return;
        }
    };

    println!("Torrent parsed successfully!");
    println!("Info Hash: {:x?}", torrent.info_hash);

    let mut tracker_urls = Vec::new();
    tracker_urls.push(torrent.announce.clone());
    if let Some(list) = &torrent.announce_list {
        for tier in list {
            for url in tier {
                if *url != torrent.announce {
                    tracker_urls.push(url.clone());
                }
            }
        }
    }

    let mut rng = rand::rng();
    let mut peer_id = [0u8; 20];
    rng.fill(&mut peer_id);
    let prefix = b"-TD0001-";
    peer_id[..8].copy_from_slice(prefix);

    let total_length = if let Some(len) = torrent.length {
        len
    } else if let Some(files) = &torrent.files {
        files.iter().map(|f| f.length).sum()
    } else {
        0
    };

    println!("Total length: {}", total_length);

    let file_path = if torrent.length.is_none() {
        println!("Multi-file torrents not fully supported yet, writing to 'output.bin'");
        storage.get_file_path("output.bin")
    } else {
        storage.get_file_path(&torrent.name)
    };

    let mut file = tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&file_path)
        .await
        .unwrap();

    let file_len = file.metadata().await.unwrap().len();
    if file_len != total_length {
        file.set_len(total_length).await.unwrap();
    }

    let piece_count = torrent.pieces.len();
    let mut piece_status_vec = vec![PieceStatus::Missing; piece_count];
    let mut downloaded_bytes = 0;

    println!("Checking existing data...");
    use tokio::io::AsyncReadExt;
    for i in 0..piece_count {
        let offset = i as u64 * torrent.piece_length;
        let len = if i == piece_count - 1 {
            let rem = total_length % torrent.piece_length;
            if rem == 0 { torrent.piece_length } else { rem }
        } else {
            torrent.piece_length
        };

        if offset + len <= file_len {
            if let Err(_) = file.seek(SeekFrom::Start(offset)).await {
                continue;
            }
            let mut buf = vec![0u8; len as usize];
            if file.read_exact(&mut buf).await.is_ok() {
                let mut hasher = Sha1::new();
                hasher.update(&buf);
                let hash = hasher.finalize();
                if hash.as_slice() == &torrent.pieces[i] {
                    piece_status_vec[i] = PieceStatus::Have;
                    downloaded_bytes += len;
                }
            }
        }
    }
    println!(
        "Resuming download. Found {}/{} pieces.",
        piece_status_vec
            .iter()
            .filter(|&&s| s == PieceStatus::Have)
            .count(),
        piece_count
    );

    let request = TrackerRequest {
        info_hash: torrent.info_hash,
        peer_id,
        port: 6881,
        uploaded: 0,
        downloaded: 0,
        left: total_length - downloaded_bytes,
        compact: true,
        no_peer_id: false,
        event: Some(TrackerEvent::Started),
        ip: None,
        numwant: Some(50),
        key: None,
        tracker_id: None,
    };

    let (peer_tx, mut peer_rx) = mpsc::channel(100);
    let piece_status = Arc::new(Mutex::new(piece_status_vec));

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
    let info_hash = torrent.info_hash;
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

    let file = Arc::new(Mutex::new(file));
    let mut handles = Vec::new();
    let torrent_arc = Arc::new(torrent);
    let (tx, _) = broadcast::channel(1);
    let mut completion_rx = tx.subscribe();
    let uploaded_total = Arc::new(Mutex::new(0u64));
    let downloaded_total = Arc::new(Mutex::new(downloaded_bytes));
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

                    let piece_status = piece_status.clone();
                    let file = file.clone();
                    let torrent = torrent_arc.clone();
                    let peer_id = peer_id;
                    let mut rx = tx.subscribe();
                    let tx = tx.clone();
                    let uploaded_total = uploaded_total.clone();
                    let downloaded_total = downloaded_total.clone();
                    let semaphore = semaphore.clone();
                    let connected_peers = connected_peers.clone();

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
                                // Reset current piece if any
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

                                            // Tell peer we have this piece
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
                    Message::Request {
                        index,
                        begin,
                        length,
                    } => {
                        // Check if we have the piece
                        let has_piece = {
                            let status = piece_status.lock().await;
                            status
                                .get(index as usize)
                                .map(|&s| s == PieceStatus::Have)
                                .unwrap_or(false)
                        };

                        if has_piece {
                            if length > 128 * 1024 {
                                eprintln!(
                                    "Requested block too large from {}: {}",
                                    peer_addr, length
                                );
                                continue;
                            }

                            let offset = (index as u64 * torrent.piece_length) + begin as u64;
                            let mut buf = vec![0u8; length as usize];

                            let mut f = file.lock().await;
                            if let Err(e) = f.seek(SeekFrom::Start(offset)).await {
                                eprintln!("Seek error reading for upload: {}", e);
                                continue;
                            }
                            // Use read_exact to ensure we get the full block
                            use tokio::io::AsyncReadExt;
                            if let Err(e) = f.read_exact(&mut buf).await {
                                eprintln!("Read error for upload: {}", e);
                                continue;
                            }
                            drop(f);

                            if let Err(e) = peer
                                .send_message(Message::Piece {
                                    index,
                                    begin,
                                    block: buf,
                                })
                                .await
                            {
                                eprintln!("Error sending piece to {}: {}", peer_addr, e);
                                break;
                            }

                            uploaded_session += length as u64;
                            let mut total = uploaded_total.lock().await;
                            *total += length as u64;
                            println!(
                                "Uploaded {} bytes to {} (Session: {}, Total: {})",
                                length, peer_addr, uploaded_session, *total
                            );
                        }
                    }
                    _ => {}
                }

                if !peer.peer_choking && current_piece_idx.is_none() {
                    let mut idx = None;
                    {
                        let mut status = piece_status.lock().await;
                        // Check if all done
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
                                // Reset status
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
    println!("Download finished.");
}
