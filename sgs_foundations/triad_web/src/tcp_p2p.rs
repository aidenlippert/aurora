use crate::NetworkMessage; // Assumes NetworkMessage is in triad_web/src/lib.rs
use async_trait::async_trait;
use serde::{Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, Mutex as TokioMutex}; // Use tokio's Mutex
use uuid::Uuid; // For generating unique connection IDs for logging

// Renamed P2PService for clarity as it's now async focused
#[async_trait]
pub trait AsyncP2PService: Send + Sync + 'static {
    async fn send_direct(&self, target_peer_addr: SocketAddr, message: NetworkMessage) -> Result<(), Box<dyn Error + Send + Sync>>;
    fn broadcast(&self, message: NetworkMessage) -> Result<(), broadcast::error::SendError<NetworkMessage>>;
    // The application will get the receiver from the new_simple function
}

#[derive(Debug)]
pub struct IncomingMessage {
    pub from_addr: SocketAddr,
    pub message: NetworkMessage,
}

// Manages all TCP P2P interactions for a node
pub struct TcpP2PManager {
    node_id_log_prefix: String,
    local_broadcast_tx: broadcast::Sender<NetworkMessage>,
    // Stores active outbound connections to peers to avoid reconnecting for every direct send.
    // Key: peer_addr, Value: Sender to the task handling that specific peer connection.
    // This is for sending *to* specific peers if we have an established connection.
    // Broadcasting is simpler via local_broadcast_tx.
    // For direct sends in this simplified version, we might just establish a new connection.
    // active_connections: Arc<TokioMutex<HashMap<SocketAddr, mpsc::Sender<NetworkMessage>>>>,
}

impl TcpP2PManager {
    pub fn new(node_id: String, listen_addr_str: &str, initial_peers_str: Option<String>)
               -> Result<(Arc<Self>, mpsc::Receiver<IncomingMessage>), Box<dyn Error + Send + Sync>> {
        
        let listen_addr: SocketAddr = listen_addr_str.parse()?;
        let (local_broadcast_tx, _) = broadcast::channel(256); // For messages this node wants to send
        let (app_message_tx, app_message_rx) = mpsc::channel(256); // For messages received from network to app logic

        let manager = Arc::new(Self {
            node_id_log_prefix: format!("[TcpP2P:{}]", node_id),
            local_broadcast_tx,
            // active_connections: Arc::new(TokioMutex::new(HashMap::new())),
        });

        // --- Spawn Listener Task ---
        let listener_node_id = node_id.clone();
        let listener_broadcast_tx = manager.local_broadcast_tx.clone();
        let listener_app_message_tx = app_message_tx.clone();
        tokio::spawn(async move {
            match TcpListener::bind(listen_addr).await {
                Ok(listener) => {
                    println!("{} Listening on {}", listener_node_id, listen_addr);
                    loop {
                        match listener.accept().await {
                            Ok((stream, addr)) => {
                                println!("{} Accepted connection from: {}", listener_node_id, addr);
                                let conn_app_tx = listener_app_message_tx.clone();
                                let conn_broadcast_rx = listener_broadcast_tx.subscribe();
                                let conn_node_id = listener_node_id.clone();
                                tokio::spawn(async move {
                                    handle_peer_connection(conn_node_id, stream, addr, conn_app_tx, conn_broadcast_rx).await;
                                });
                            }
                            Err(e) => eprintln!("{} Error accepting connection: {}", listener_node_id, e),
                        }
                    }
                }
                Err(e) => eprintln!("{} Failed to bind listener on {}: {}", listener_node_id, listen_addr, e),
            }
        });

        // --- Spawn Initial Peer Connection Task ---
        if let Some(peers_str) = initial_peers_str {
            let connector_node_id = node_id.clone();
            let connector_broadcast_tx = manager.local_broadcast_tx.clone();
            let connector_app_message_tx = app_message_tx.clone(); // Pass the same tx to all connection handlers
            
            tokio::spawn(async move {
                // Give listener a moment to start (crude)
                tokio::time::sleep(Duration::from_millis(500)).await; 
                let peer_addrs: Vec<SocketAddr> = peers_str.split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();

                for peer_addr in peer_addrs {
                    if peer_addr == listen_addr { continue; } // Don't connect to self
                    
                    println!("{} Attempting to connect to peer: {}", connector_node_id, peer_addr);
                    match TcpStream::connect(peer_addr).await {
                        Ok(stream) => {
                            println!("{} Connected to peer: {}", connector_node_id, peer_addr);
                            let conn_app_tx = connector_app_message_tx.clone();
                            let conn_broadcast_rx = connector_broadcast_tx.subscribe();
                            let conn_node_id = connector_node_id.clone();
                            tokio::spawn(async move {
                                handle_peer_connection(conn_node_id, stream, peer_addr, conn_app_tx, conn_broadcast_rx).await;
                            });
                        }
                        Err(e) => {
                            eprintln!("{} Failed to connect to peer {}: {}", connector_node_id, peer_addr, e);
                        }
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
        println!("{} Attempting direct send to {}", self.node_id_log_prefix, target_peer_addr);
        let mut stream = TcpStream::connect(target_peer_addr).await?;
        println!("{} Connected to {} for direct send.", self.node_id_log_prefix, target_peer_addr);
        send_framed_message(&mut stream, &message).await?;
        Ok(())
    }

    fn broadcast(&self, message: NetworkMessage) -> Result<(), broadcast::error::SendError<NetworkMessage>> {
        // The number of receivers can be 0 if no peers are connected yet or all disconnected.
        // send() returns Err if no active receivers. This is okay.
        let num_receivers = self.local_broadcast_tx.receiver_count();
        if num_receivers > 0 {
             println!("{} Queuing message for broadcast to {} peers: {:?}", self.node_id_log_prefix, num_receivers, message_summary(&message));
        } else {
            // println!("{} No active peers to broadcast to for message: {:?}", self.node_id_log_prefix, message_summary(&message));
        }
        self.local_broadcast_tx.send(message)?; // Ignore error if no subscribers
        Ok(())
    }
}

async fn handle_peer_connection(
    node_id_log_prefix_for_handler: String, // To avoid borrowing self into the task
    stream: TcpStream,
    peer_addr: SocketAddr,
    app_message_tx: mpsc::Sender<IncomingMessage>,
    mut local_broadcast_rx: broadcast::Receiver<NetworkMessage>,
) {
    let handler_log_prefix = format!("[TcpHandler:{}:{}]", node_id_log_prefix_for_handler, peer_addr);
    println!("{} Connection active.", handler_log_prefix);
    let (reader, writer) = stream.into_split(); // Use into_split to get owned halves
    let mut buf_reader = BufReader::new(reader);
    let mut buf_writer = BufWriter::new(writer);

    loop {
        tokio::select! {
            biased; // Prioritize reading incoming messages slightly

            // Read messages from the peer
            result = read_framed_message(&mut buf_reader) => {
                match result {
                    Ok(Some(message)) => {
                        println!("{} Received: {:?}", handler_log_prefix, message_summary(&message));
                        if app_message_tx.send(IncomingMessage { from_addr: peer_addr, message }).await.is_err() {
                            eprintln!("{} App receiver closed. Terminating connection.", handler_log_prefix);
                            return;
                        }
                    }
                    Ok(None) => {
                        println!("{} Connection closed by peer.", handler_log_prefix);
                        return;
                    }
                    Err(e) => {
                        eprintln!("{} Error reading from peer: {}. Terminating connection.", handler_log_prefix, e);
                        return;
                    }
                }
            },
            // Send messages from local broadcast queue to this specific peer
            result = local_broadcast_rx.recv() => {
                match result {
                    Ok(message_to_broadcast) => {
                        println!("{} Broadcasting to peer: {:?}", handler_log_prefix, message_summary(&message_to_broadcast));
                        if let Err(e) = send_framed_message(&mut buf_writer, &message_to_broadcast).await {
                            eprintln!("{} Error sending broadcast to peer: {}. Terminating connection.", handler_log_prefix, e);
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("{} Lagged in broadcast by {} messages for peer.", handler_log_prefix, n);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        eprintln!("{} Broadcast channel closed. Terminating connection.", handler_log_prefix);
                        return;
                    }
                }
            }
        }
    }
}

async fn send_framed_message<W: AsyncWriteExt + Unpin>(stream: &mut W, msg: &NetworkMessage) -> Result<(), std::io::Error> {
    let encoded = bincode::serialize(msg).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let len = encoded.len() as u32;
    stream.write_u32(len).await?;
    stream.write_all(&encoded).await?;
    stream.flush().await?;
    Ok(())
}

async fn read_framed_message<R: AsyncReadExt + Unpin>(stream: &mut R) -> Result<Option<NetworkMessage>, std::io::Error> {
    match stream.read_u32().await {
        Ok(len) => {
            if len == 0 { return Ok(None); }
            if len > 10 * 1024 * 1024 { // Max message size 10MB, sanity check
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Message too large"));
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
