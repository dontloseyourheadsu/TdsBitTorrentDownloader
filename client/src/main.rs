use clap::Parser;
use rand::Rng;
use sha1::{Digest, Sha1};
use std::io::SeekFrom;
use std::sync::Arc;
use tds_core::parse_torrent;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::{Mutex, broadcast};
use tracker::{TrackerEvent, TrackerRequest, get_tracker_client};

mod peer;
use peer::{Message, PeerConnection};
mod storage;
use storage::Storage;

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

    let request = TrackerRequest {
        info_hash: torrent.info_hash,
        peer_id,
        port: 6881,
        uploaded: 0,
        downloaded: 0,
        left: total_length,
        compact: true,
        no_peer_id: false,
        event: Some(TrackerEvent::Started),
        ip: None,
        numwant: Some(50),
        key: None,
        tracker_id: None,
    };

    let peers = tokio::task::spawn_blocking(move || {
        let mut all_peers = Vec::new();
        for url in tracker_urls {
            println!("Contacting tracker: {}", url);
            if let Some(client) = get_tracker_client(&url) {
                match client.announce(&request) {
                    Ok(response) => {
                        println!(
                            "Tracker response received from {}! Found {} peers",
                            url,
                            response.peers.len()
                        );
                        all_peers.extend(response.peers);
                        if !all_peers.is_empty() {
                            return Some(all_peers);
                        }
                    }
                    Err(e) => eprintln!("Tracker error ({}): {}", url, e),
                }
            }
        }
        if all_peers.is_empty() {
            None
        } else {
            Some(all_peers)
        }
    })
    .await
    .unwrap();

    let peers = match peers {
        Some(p) => p,
        None => {
            eprintln!("Failed to contact any tracker.");
            return;
        }
    };

    println!("Found {} peers.", peers.len());

    let piece_count = torrent.pieces.len();
    let piece_status = Arc::new(Mutex::new(vec![PieceStatus::Missing; piece_count]));

    let file_path = if torrent.length.is_none() {
        println!("Multi-file torrents not fully supported yet, writing to 'output.bin'");
        storage.get_file_path("output.bin")
    } else {
        storage.get_file_path(&torrent.name)
    };

    let file = tokio::fs::File::create(&file_path).await.unwrap();
    file.set_len(total_length).await.unwrap();
    let file = Arc::new(Mutex::new(file));

    let mut handles = Vec::new();
    let torrent_arc = Arc::new(torrent);
    let (tx, _) = broadcast::channel(1);

    for peer_addr in peers {
        let piece_status = piece_status.clone();
        let file = file.clone();
        let torrent = torrent_arc.clone();
        let peer_id = peer_id;
        let mut rx = tx.subscribe();
        let tx = tx.clone();

        handles.push(tokio::spawn(async move {
            println!("Connecting to {}", peer_addr);
            let mut peer =
                match PeerConnection::connect(peer_addr, &torrent.info_hash, &peer_id).await {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("Failed to connect to {}: {}", peer_addr, e);
                        return;
                    }
                };
            println!("Connected to {}", peer_addr);

            if let Err(e) = peer.send_message(Message::Interested).await {
                eprintln!("Error sending interested to {}: {}", peer_addr, e);
                return;
            }

            let mut current_piece_idx: Option<usize> = None;
            let mut current_piece_data: Vec<u8> = Vec::new();
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

                                            current_piece_idx = None;
                                        } else {
                                            eprintln!(
                                                "Piece {} hash mismatch from {}!",
                                                curr, peer_addr
                                            );
                                            let mut status = piece_status.lock().await;
                                            status[curr] = PieceStatus::Missing;
                                            current_piece_idx = None;
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
        }));
    }

    for h in handles {
        let _ = h.await;
    }
    println!("Download finished.");
}
