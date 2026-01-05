use std::net::SocketAddrV4;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[derive(Debug)]
pub enum Message {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    Bitfield(Vec<u8>),
    Request {
        index: u32,
        begin: u32,
        length: u32,
    },
    Piece {
        index: u32,
        begin: u32,
        block: Vec<u8>,
    },
    Cancel {
        index: u32,
        begin: u32,
        length: u32,
    },
    Extended {
        id: u8,
        payload: Vec<u8>,
    },
}

pub struct PeerConnection {
    addr: SocketAddrV4,
    stream: TcpStream,
    pub peer_id: [u8; 20],
    pub peer_choking: bool,
    pub peer_interested: bool,
    pub am_choking: bool,
    pub am_interested: bool,
    pub bitfield: Vec<u8>,
}

impl PeerConnection {
    pub fn has_piece(&self, index: u32) -> bool {
        let byte_index = (index / 8) as usize;
        let bit_index = 7 - (index % 8);
        if byte_index < self.bitfield.len() {
            (self.bitfield[byte_index] >> bit_index) & 1 == 1
        } else {
            false
        }
    }

    pub async fn connect(
        addr: SocketAddrV4,
        info_hash: &[u8; 20],
        client_id: &[u8; 20],
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut stream =
            tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(addr)).await??;

        // Handshake
        let mut handshake = Vec::new();
        handshake.push(19);
        handshake.extend_from_slice(b"BitTorrent protocol");
        let mut reserved = [0u8; 8];
        reserved[5] |= 0x10; // Extension protocol bit
        handshake.extend_from_slice(&reserved);
        handshake.extend_from_slice(info_hash);
        handshake.extend_from_slice(client_id);

        stream.write_all(&handshake).await?;

        let mut response = vec![0u8; 68];
        stream.read_exact(&mut response).await?;

        if response[0] != 19 || &response[1..20] != b"BitTorrent protocol" {
            return Err("Invalid handshake".into());
        }

        if &response[28..48] != info_hash {
            return Err("Info hash mismatch".into());
        }

        let mut peer_id = [0u8; 20];
        peer_id.copy_from_slice(&response[48..68]);

        Ok(Self {
            addr,
            stream,
            peer_id,
            peer_choking: true,
            peer_interested: false,
            am_choking: true,
            am_interested: false,
            bitfield: Vec::new(),
        })
    }

    pub async fn send_message(
        &mut self,
        msg: Message,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match msg {
            Message::KeepAlive => {
                self.stream.write_u32(0).await?;
            }
            Message::Choke => {
                self.stream.write_u32(1).await?;
                self.stream.write_u8(0).await?;
            }
            Message::Unchoke => {
                self.stream.write_u32(1).await?;
                self.stream.write_u8(1).await?;
            }
            Message::Interested => {
                self.stream.write_u32(1).await?;
                self.stream.write_u8(2).await?;
            }
            Message::NotInterested => {
                self.stream.write_u32(1).await?;
                self.stream.write_u8(3).await?;
            }
            Message::Have(index) => {
                self.stream.write_u32(5).await?;
                self.stream.write_u8(4).await?;
                self.stream.write_u32(index).await?;
            }
            Message::Request {
                index,
                begin,
                length,
            } => {
                self.stream.write_u32(13).await?;
                self.stream.write_u8(6).await?;
                self.stream.write_u32(index).await?;
                self.stream.write_u32(begin).await?;
                self.stream.write_u32(length).await?;
            }
            Message::Piece {
                index,
                begin,
                block,
            } => {
                let len = 9 + block.len() as u32;
                self.stream.write_u32(len).await?;
                self.stream.write_u8(7).await?;
                self.stream.write_u32(index).await?;
                self.stream.write_u32(begin).await?;
                self.stream.write_all(&block).await?;
            }
            Message::Bitfield(bitfield) => {
                let len = 1 + bitfield.len() as u32;
                self.stream.write_u32(len).await?;
                self.stream.write_u8(5).await?;
                self.stream.write_all(&bitfield).await?;
            }
            Message::Extended { id, payload } => {
                let len = 2 + payload.len() as u32;
                self.stream.write_u32(len).await?;
                self.stream.write_u8(20).await?;
                self.stream.write_u8(id).await?;
                self.stream.write_all(&payload).await?;
            }
            _ => {} // Implement others as needed
        }
        Ok(())
    }

    pub async fn read_message(
        &mut self,
    ) -> Result<Message, Box<dyn std::error::Error + Send + Sync>> {
        let fut = async {
            let len = self.stream.read_u32().await?;
            if len == 0 {
                return Ok(Message::KeepAlive);
            }

            let id = self.stream.read_u8().await?;
            match id {
                0 => {
                    self.peer_choking = true;
                    Ok(Message::Choke)
                }
                1 => {
                    self.peer_choking = false;
                    Ok(Message::Unchoke)
                }
                2 => {
                    self.peer_interested = true;
                    Ok(Message::Interested)
                }
                3 => {
                    self.peer_interested = false;
                    Ok(Message::NotInterested)
                }
                4 => {
                    let index = self.stream.read_u32().await?;
                    let byte_index = (index / 8) as usize;
                    let bit_index = 7 - (index % 8);
                    if byte_index >= self.bitfield.len() {
                        self.bitfield.resize(byte_index + 1, 0);
                    }
                    self.bitfield[byte_index] |= 1 << bit_index;
                    Ok(Message::Have(index))
                }
                5 => {
                    let mut payload = vec![0u8; (len - 1) as usize];
                    self.stream.read_exact(&mut payload).await?;
                    self.bitfield = payload.clone();
                    Ok(Message::Bitfield(payload))
                }
                6 => {
                    let index = self.stream.read_u32().await?;
                    let begin = self.stream.read_u32().await?;
                    let length = self.stream.read_u32().await?;
                    Ok(Message::Request {
                        index,
                        begin,
                        length,
                    })
                }
                7 => {
                    let index = self.stream.read_u32().await?;
                    let begin = self.stream.read_u32().await?;
                    let mut block = vec![0u8; (len - 9) as usize];
                    self.stream.read_exact(&mut block).await?;
                    Ok(Message::Piece {
                        index,
                        begin,
                        block,
                    })
                }
                8 => {
                    let index = self.stream.read_u32().await?;
                    let begin = self.stream.read_u32().await?;
                    let length = self.stream.read_u32().await?;
                    Ok(Message::Cancel {
                        index,
                        begin,
                        length,
                    })
                }
                20 => {
                    let ext_id = self.stream.read_u8().await?;
                    let mut payload = vec![0u8; (len - 2) as usize];
                    self.stream.read_exact(&mut payload).await?;
                    Ok(Message::Extended {
                        id: ext_id,
                        payload,
                    })
                }
                _ => {
                    // Skip unknown message
                    let mut buf = vec![0u8; (len - 1) as usize];
                    self.stream.read_exact(&mut buf).await?;
                    Err(format!("Unknown message id: {}", id).into())
                }
            }
        };

        match tokio::time::timeout(Duration::from_secs(30), fut).await {
            Ok(res) => res,
            Err(_) => Err("Read timeout".into()),
        }
    }
}
