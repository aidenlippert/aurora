use crate::NetworkMessage;
use async_trait::async_trait;
use std::collections::HashMap;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, Mutex as TokioMutex};
use tokio::time::timeout;
use std::time::Duration;
use uuid::Uuid;

const CONNECTION_TIMEOUT_SECS: u64 = 10;
const READ_TIMEOUT_SECS: u64 = 60; // Longer timeout for reads
const MAX_MESSAGE_SIZE_BYTES: u32 = 10 * 1024 * 1024; // 10MB

#[async_trait]
pub trait AsyncP2PService: Send + Sync + 'static {
    async fn send_direct(&self, target_peer_addr: SocketAddr, message: NetworkMessage) -> Result<(), Box<dyn Error + Send + Sync>>;
    fn broadcast(&self, message: NetworkMessage) -> Result<(), String>; // Changed return type for broadcast error
    // Receiver is obtained from initialize_p2p_service
}

#[derive(Debug)]
pub struct IncomingMessage {
    pub from_addr: SocketAddr,
    pub message: NetworkMessage,
}

pub struct TcpP2PManager {
    node_id_log_prefix: String,
    local_broadcast_tx: broadcast::Sender<NetworkMessage>,
    // For active outgoing connections to specific peers (if we want to reuse them)
    // This can get complex with reconnection logic. For now, send_direct might make new connections.
    // active_outbound_connections: Arc<TokioMutex<HashMap<SocketAddr, mpsc::Sender<NetworkMessage>>>>,
}

impl TcpP2PManager {
    pub async fn new(
        node_id: String,
        listen_addr_str: &str,
        initial_peers_str: Option<String>,
    ) -> Result<(Arc<Self>, mpsc::Receiver<IncomingMessage>), Box<dyn Error + Send + Sync>> {
        
        let listen_addr: SocketAddr = listen_addr_str.parse()?;
        let (local_broadcast_tx, _) = broadcast::channel(256);
        let (app_message_tx, app_message_rx) = mpsc::channel(256);

        let manager_node_id_log_prefix = format!("[TcpP2P:{}]", node_id);

        let manager = Arc::new(Self {
            node_id_log_prefix: manager_node_id_log_prefix.clone(),
            local_broadcast_tx: local_broadcast_tx.clone(),
        });

        // --- Spawn Listener Task ---
        let listener_log_prefix = manager.node_id_log_prefix.clone();
        let listener_broadcast_tx_clone = local_broadcast_tx.clone(); // For new connections to subscribe
        let listener_app_message_tx_clone = app_message_tx.clone();   // For new connections to send received msgs
        
        tokio::spawn(async move {
            match TcpListener::bind(listen_addr).await {
                Ok(listener) => {
                    println!("{} Listening on {}", listener_log_prefix, listen_addr);
                    loop {
                        match listener.accept().await {
                            Ok((stream, addr)) => {
                                println!("{} Accepted connection from: {}", listener_log_prefix, addr);
                                let conn_app_tx = listener_app_message_tx_clone.clone();
                                let conn_broadcast_rx = listener_broadcast_tx_clone.subscribe();
                                let conn_handler_log_prefix = listener_log_prefix.clone();
                                tokio::spawn(async move {
                                    handle_peer_connection(conn_handler_log_prefix, stream, addr, conn_app_tx, conn_broadcast_rx).await;
                                });
                            }
                            Err(e) => eprintln!("{} Error accepting connection: {}", listener_log_prefix, e),
                        }
                    }
                }
                Err(e) => eprintln!("{} Failed to bind listener on {}: {}", listener_log_prefix, listen_addr, e),
            }
        });

        // --- Spawn Initial Peer Connection Task ---
        if let Some(peers_str) = initial_peers_str {
            let connector_log_prefix = manager.node_id_log_prefix.clone();
            let connector_broadcast_tx_clone = local_broadcast_tx.clone();
            let connector_app_message_tx_clone = app_message_tx.clone();
            
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(1000)).await; // Give listener a moment
                let peer_addrs: Vec<SocketAddr> = peers_str.split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();

                for peer_addr in peer_addrs {
                    if peer_addr == listen_addr { continue; }
                    
                    println!("{} Attempting to connect to peer: {}", connector_log_prefix, peer_addr);
                    match timeout(Duration::from_secs(CONNECTION_TIMEOUT_SECS), TcpStream::connect(peer_addr)).await {
                        Ok(Ok(stream)) => {
                            println!("{} Connected to peer: {}", connector_log_prefix, peer_addr);
                            let conn_app_tx = connector_app_message_tx_clone.clone();
                            let conn_broadcast_rx = connector_broadcast_tx_clone.subscribe();
                            let conn_handler_log_prefix = connector_log_prefix.clone();
                            tokio::spawn(async move {
                                handle_peer_connection(conn_handler_log_prefix, stream, peer_addr, conn_app_tx, conn_broadcast_rx).await;
                            });
                        }
                        Ok(Err(e)) => eprintln!("{} Failed to connect to peer {}: {}", connector_log_prefix, peer_addr, e),
                        Err(_) => eprintln!("{} Timeout connecting to peer {}", connector_log_prefix, peer_addr),
                    }
                }
            });
        }
        Ok((manager, app_message_rx))
    }
}

#[async_trait]
impl AsyncP2PService for TcpP2PManager {
    async fn send_direct(&self, target_peer_addr: SocketAddr, message: NetworkMessage) -> Result<(), Box<dyn Error + Send + Sync>> {
        println!("{} Attempting direct send to {}: {:?}", self.node_id_log_prefix, target_peer_addr, message_summary(&message));
        match timeout(Duration::from_secs(CONNECTION_TIMEOUT_SECS), TcpStream::connect(target_peer_addr)).await {
            Ok(Ok(mut stream)) => {
                println!("{} Connected to {} for direct send.", self.node_id_log_prefix, target_peer_addr);
                send_framed_message(&mut stream, &message).await?;
                println!("{} Direct message sent successfully to {}.", self.node_id_log_prefix, target_peer_addr);
                Ok(())
            }
            Ok(Err(e)) => {
                eprintln!("{} Failed to connect to {} for direct send: {}", self.node_id_log_prefix, target_peer_addr, e);
                Err(Box::new(e))
            }
            Err(_) => {
                let err_msg = format!("Timeout connecting to {} for direct send", target_peer_addr);
                eprintln!("{} {}", self.node_id_log_prefix, err_msg);
                Err(err_msg.into())
            }
        }
    }

    fn broadcast(&self, message: NetworkMessage) -> Result<(), String> {
        let num_receivers = self.local_broadcast_tx.receiver_count();
        // if num_receivers > 0 { // This log can be noisy if many peers are connected but not all immediately recv
             println!("{} Queuing message for broadcast ({} current subscribers): {:?}", self.node_id_log_prefix, num_receivers, message_summary(&message));
        // }
        self.local_broadcast_tx.send(message.clone()).map_err(|e| format!("Broadcast send error: {}", e.to_string()))?;
        Ok(())
    }
}

async fn handle_peer_connection(
    node_id_log_prefix_for_handler: String,
    stream: TcpStream,
    peer_addr: SocketAddr,
    app_message_tx: mpsc::Sender<IncomingMessage>,
    mut local_broadcast_rx: broadcast::Receiver<NetworkMessage>,
) {
    let handler_log_prefix = format!("[TcpH:{}:{}]", node_id_log_prefix_for_handler.replace("[TcpP2P:", "").replace("]", ""), peer_addr);
    println!("{} Connection active.", handler_log_prefix);
    
    if let Err(e) = stream.set_nodelay(true) {
        eprintln!("{} Failed to set TCP_NODELAY: {}", handler_log_prefix, e);
    }

    let (reader, writer) = stream.into_split();
    let mut buf_reader = BufReader::new(reader);
    let mut buf_writer = BufWriter::new(writer);

    loop {
        tokio::select! {
            biased;
            result = timeout(Duration::from_secs(READ_TIMEOUT_SECS), read_framed_message(&mut buf_reader)) => {
                match result {
                    Ok(Ok(Some(message))) => {
                        // println!("{} Received: {:?}", handler_log_prefix, message_summary(&message));
                        if app_message_tx.send(IncomingMessage { from_addr: peer_addr, message }).await.is_err() {
                            eprintln!("{} App receiver closed. Terminating for {}.", handler_log_prefix, peer_addr); return;
                        }
                    }
                    Ok(Ok(None)) => { println!("{} Connection with {} closed cleanly by peer.", handler_log_prefix, peer_addr); return; }
                    Ok(Err(e)) => { eprintln!("{} Error reading from {}: {}. Terminating.", handler_log_prefix, peer_addr, e); return; }
                    Err(_) => { eprintln!("{} Timeout reading from {}. Terminating.", handler_log_prefix, peer_addr); return; }
                }
            },
            result = local_broadcast_rx.recv() => {
                match result {
                    Ok(message_to_broadcast) => {
                        // println!("{} Broadcasting to peer: {:?}", handler_log_prefix, message_summary(&message_to_broadcast));
                        if let Err(e) = send_framed_message(&mut buf_writer, &message_to_broadcast).await {
                            eprintln!("{} Error sending broadcast to peer: {}. Terminating.", handler_log_prefix, e); return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => { eprintln!("{} Lagged in broadcast by {} msgs for peer.", handler_log_prefix, n); }
                    Err(broadcast::error::RecvError::Closed) => { eprintln!("{} Broadcast channel closed. Terminating for {}.", handler_log_prefix, peer_addr); return; }
                }
            }
        }
    }
}

async fn send_framed_message<W: AsyncWriteExt + Unpin>(stream: &mut W, msg: &NetworkMessage) -> Result<(), std::io::Error> {
    let encoded = bincode::serialize(msg).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let len = encoded.len() as u32;
    if len > MAX_MESSAGE_SIZE_BYTES {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Message too large to send"));
    }
    stream.write_u32(len).await?;
    stream.write_all(&encoded).await?;
    stream.flush().await?;
    Ok(())
}

async fn read_framed_message<R: AsyncReadExt + Unpin>(stream: &mut R) -> Result<Option<NetworkMessage>, std::io::Error> {
    match stream.read_u32().await {
        Ok(len) => {
            if len == 0 { return Ok(None); } // Or an error, as 0-len is unusual
            if len > MAX_MESSAGE_SIZE_BYTES {
                 eprintln!("[TcpP2P Read] Message length {} exceeds MAX_MESSAGE_SIZE_BYTES {}", len, MAX_MESSAGE_SIZE_BYTES);
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Received message too large"));
            }
            let mut buffer = vec![0; len as usize];
            stream.read_exact(&mut buffer).await?;
            bincode::deserialize(&buffer).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)).map(Some)
        }
        Err(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
        Err(e) => Err(e),
    }
}

fn message_summary(msg: &NetworkMessage) -> String {
    match msg {
        NetworkMessage::BlockProposal(data) => format!("BlockProposal(size:{})", data.len()),
        NetworkMessage::Transaction(data) => format!("Transaction(size:{})", data.len()),
        _ => format!("{:?}", msg),
    }
}
