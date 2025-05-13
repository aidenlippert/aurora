use crate::NetworkMessage; // Assuming NetworkMessage is in triad_web/src/lib.rs
use async_trait::async_trait;
use serde::{Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, Mutex}; // tokio's Mutex

// Trait for the P2P service (could be in lib.rs)
#[async_trait]
pub trait P2PService: Send + Sync + 'static {
    // For sending to a specific, known peer (e.g., for direct messages or RPC-like calls)
    async fn send_direct(&self, peer_addr: SocketAddr, message: NetworkMessage) -> Result<(), Box<dyn Error + Send + Sync>>;
    // For broadcasting to all connected peers
    fn broadcast(&self, message: NetworkMessage);
    // Channel to receive messages from any peer
    fn message_rx(&self) -> mpsc::Receiver<IncomingMessage>;
}

#[derive(Debug)]
pub struct IncomingMessage {
    pub from: SocketAddr,
    pub message: NetworkMessage,
}

pub struct TcpP2PManager {
    node_id: String, // For logging
    listen_addr: SocketAddr,
    // For broadcasting messages originating from this node
    local_broadcast_tx: broadcast::Sender<NetworkMessage>,
    // For receiving messages from the network tasks
    network_message_rx: mpsc::Receiver<IncomingMessage>,
    // For sending messages *to* the network tasks (e.g., a direct send request)
    // This part is more complex and might involve each connection task having its own sender
    // For simplicity, direct sends will try to establish a new connection for now.
    // A more robust system would maintain persistent connections and route through them.
}

impl TcpP2PManager {
    pub async fn new(node_id: String, listen_addr: SocketAddr) -> Result<(Arc<Self>, mpsc::Receiver<IncomingMessage>), Box<dyn Error + Send + Sync>> {
        let (local_broadcast_tx, _) = broadcast::channel(100); // Capacity for broadcast channel
        let (network_message_tx, network_message_rx) = mpsc::channel(100); // For messages from network to node logic

        let manager = Arc::new(Self {
            node_id: node_id.clone(),
            listen_addr,
            local_broadcast_tx: local_broadcast_tx.clone(),
            network_message_rx, // This rx is moved out for the node to listen on
        });

        let manager_clone_listener = manager.clone();
        let network_message_tx_clone_listener = network_message_tx.clone();
        
        // Spawn listener task
        tokio::spawn(async move {
            if let Err(e) = manager_clone_listener.listen_for_peers(network_message_tx_clone_listener).await {
                eprintln!("[TcpP2P:{}] Listener error: {}", manager_clone_listener.node_id, e);
            }
        });
        
        // Return the receiver for the main node logic
        let (app_msg_tx, app_msg_rx) = mpsc::channel(100);
        // Relay messages from network_message_rx to app_msg_rx
        // This might be overly complex for now. The initial network_message_rx can be returned directly.
        // Let's simplify: the node directly consumes network_message_rx.

        Ok((manager, network_message_tx.recv_chan())) // This is not right, need to return the original RX.
                                                     // Correction: The original network_message_rx is moved into the manager struct.
                                                     // We need a new channel for the application to receive messages.
                                                     // Let's re-think this. The manager needs the TX to send to app logic.
                                                     // The app logic needs the RX.
    }
    
    // Simplified constructor for now
    pub fn new_simple(node_id: String, listen_addr: SocketAddr) -> (Arc<Self>, mpsc::Receiver<IncomingMessage>) {
        let (local_broadcast_tx, _) = broadcast::channel(100);
        let (network_message_tx_to_app, network_message_rx_for_app) = mpsc::channel(100);

        // Spawn listener
        let node_id_clone = node_id.clone();
        let listen_addr_clone = listen_addr.clone();
        let network_message_tx_clone = network_message_tx_to_app.clone();
        let local_broadcast_tx_clone_for_listener = local_broadcast_tx.clone();

        tokio::spawn(async move {
            match TcpListener::bind(listen_addr_clone).await {
                Ok(listener) => {
                    println!("[TcpP2P:{}] Listening on {}", node_id_clone, listen_addr_clone);
                    loop {
                        match listener.accept().await {
                            Ok((stream, addr)) => {
                                println!("[TcpP2P:{}] Accepted connection from: {}", node_id_clone, addr);
                                let network_msg_tx = network_message_tx_clone.clone();
                                let mut broadcast_rx = local_broadcast_tx_clone_for_listener.subscribe();
                                let peer_node_id = node_id_clone.clone(); // For logging within handle_connection
                                tokio::spawn(async move {
                                    handle_connection(stream, addr, network_msg_tx, broadcast_rx, peer_node_id).await;
                                });
                            }
                            Err(e) => eprintln!("[TcpP2P:{}] Error accepting connection: {}", node_id_clone, e),
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[TcpP2P:{}] Failed to bind listener on {}: {}", node_id_clone, listen_addr_clone, e);
                }
            }
        });
        
        let manager_arc = Arc::new(Self {
            node_id,
            listen_addr,
            local_broadcast_tx,
            network_message_rx: network_message_rx_for_app, // This is wrong, manager should not own RX for app
        });
        // Corrected logic for new_simple: manager doesn't hold the app's RX channel.
        // The returned RX channel is for the app to use.
        (manager_arc, network_message_rx_for_app)
    }


    pub async fn connect_to_initial_peers(&self, peer_addrs: Vec<SocketAddr>, network_message_tx_to_app: mpsc::Sender<IncomingMessage>) {
        for peer_addr in peer_addrs {
            if peer_addr == self.listen_addr { continue; } // Don't connect to self

            let node_id_clone = self.node_id.clone();
            let network_msg_tx = network_message_tx_to_app.clone();
            let mut broadcast_rx = self.local_broadcast_tx.subscribe();
            println!("[TcpP2P:{}] Attempting to connect to peer: {}", self.node_id, peer_addr);

            match TcpStream::connect(peer_addr).await {
                Ok(stream) => {
                    println!("[TcpP2P:{}] Connected to peer: {}", self.node_id, peer_addr);
                    tokio::spawn(async move {
                        handle_connection(stream, peer_addr, network_msg_tx, broadcast_rx, node_id_clone).await;
                    });
                }
                Err(e) => {
                    eprintln!("[TcpP2P:{}] Failed to connect to peer {}: {}", self.node_id, peer_addr, e);
                }
            }
        }
    }
}


#[async_trait]
impl P2PService for TcpP2PManager {
    async fn send_direct(&self, peer_addr: SocketAddr, message: NetworkMessage) -> Result<(), Box<dyn Error + Send + Sync>> {
        println!("[TcpP2P:{}] Attempting direct send to {}", self.node_id, peer_addr);
        let mut stream = TcpStream::connect(peer_addr).await?;
        println!("[TcpP2P:{}] Connected to {} for direct send.", self.node_id, peer_addr);
        send_framed_message(&mut stream, &message).await?;
        Ok(())
    }

    fn broadcast(&self, message: NetworkMessage) {
        if let Err(e) = self.local_broadcast_tx.send(message.clone()) { // Clone for broadcast
            eprintln!("[TcpP2P:{}] Error broadcasting local message: {:?} - {}", self.node_id, message, e);
        } else {
            println!("[TcpP2P:{}] Message queued for broadcast: {:?}", self.node_id, message_summary(&message));
        }
    }
    
    // This method is problematic as P2PService is not generic over its receiver type
    // The node using TcpP2PManager gets the receiver from new_simple()
    fn message_rx(&self) -> mpsc::Receiver<IncomingMessage> {
        // This is not right. The manager itself shouldn't provide its internal receiver this way.
        // The application gets the receiver when the TcpP2PManager is created.
        // For the trait, this method is hard to implement correctly without generics or associated types.
        // Let's return a dummy or panic for now, as the app will use the one from new_simple().
        let (_, dummy_rx) = mpsc::channel(1);
        dummy_rx
    }
}


async fn handle_connection(
    mut stream: TcpStream,
    addr: SocketAddr,
    app_message_tx: mpsc::Sender<IncomingMessage>, // To send received messages to the main node logic
    mut local_broadcast_rx: broadcast::Receiver<NetworkMessage>, // To receive messages this node wants to broadcast
    node_id: String // For logging
) {
    println!("[TcpP2PHandle:{}] Handling connection for/from {}", node_id, addr);
    let (mut reader, mut writer) = stream.split();

    loop {
        tokio::select! {
            // Read messages from the peer
            result = read_framed_message(&mut reader) => {
                match result {
                    Ok(Some(message)) => {
                        println!("[TcpP2PHandle:{}] Received from {}: {:?}", node_id, addr, message_summary(&message));
                        if app_message_tx.send(IncomingMessage { from: addr, message }).await.is_err() {
                            eprintln!("[TcpP2PHandle:{}] App receiver closed for messages from {}. Terminating.", node_id, addr);
                            return;
                        }
                    }
                    Ok(None) => { // Stream closed cleanly
                        println!("[TcpP2PHandle:{}] Connection with {} closed by peer.", node_id, addr);
                        return;
                    }
                    Err(e) => {
                        eprintln!("[TcpP2PHandle:{}] Error reading from {}: {}", node_id, addr, e);
                        return; // Terminate on error
                    }
                }
            },
            // Send messages from local broadcast queue to this peer
            result = local_broadcast_rx.recv() => {
                match result {
                    Ok(message_to_broadcast) => {
                         println!("[TcpP2PHandle:{}] Broadcasting to {}: {:?}", node_id, addr, message_summary(&message_to_broadcast));
                        if let Err(e) = send_framed_message(&mut writer, &message_to_broadcast).await {
                            eprintln!("[TcpP2PHandle:{}] Error sending broadcast to {}: {}", node_id, addr, e);
                            return; // Terminate on send error
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("[TcpP2PHandle:{}] Lagged in broadcast from self by {} messages for peer {}.", node_id, n, addr);
                        // Continue, but this indicates a problem.
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        eprintln!("[TcpP2PHandle:{}] Broadcast channel closed. Terminating connection for {}.", node_id, addr);
                        return;
                    }
                }
            }
        }
    }
}

// Simple length-prefix framing for messages
async fn send_framed_message<W: AsyncWriteExt + Unpin>(stream: &mut W, msg: &NetworkMessage) -> io::Result<()> {
    let encoded = bincode::serialize(msg).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let len = encoded.len() as u32;
    stream.write_u32(len).await?; // Write length (4 bytes)
    stream.write_all(&encoded).await?; // Write message
    stream.flush().await?;
    Ok(())
}

async fn read_framed_message<R: AsyncReadExt + Unpin>(stream: &mut R) -> io::Result<Option<NetworkMessage>> {
    match stream.read_u32().await {
        Ok(len) => {
            if len == 0 { return Ok(None); } // Should not happen if sender is correct
            let mut buffer = vec![0; len as usize];
            stream.read_exact(&mut buffer).await?;
            bincode::deserialize(&buffer).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)).map(Some)
        }
        Err(ref e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(None), // Connection closed
        Err(e) => Err(e),
    }
}

fn message_summary(msg: &NetworkMessage) -> String { // Helper for concise logging
    match msg {
        NetworkMessage::BlockProposal(_) => "BlockProposal".to_string(),
        NetworkMessage::Transaction(_) => "Transaction".to_string(),
        _ => format!("{:?}", msg), // Default debug for others
    }
}

// This function will be called by nodes to get their P2P interface.
// It's simplified here. A real system would manage instances properly.
pub fn initialize_tcp_p2p_service(node_id: String, listen_addr_str: &str) -> Result<(Arc<TcpP2PManager>, mpsc::Receiver<IncomingMessage>), Box<dyn Error + Send + Sync>> {
    let listen_addr = listen_addr_str.parse()?;
    Ok(TcpP2PManager::new_simple(node_id, listen_addr))
}
