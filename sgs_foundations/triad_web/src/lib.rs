#![allow(unused_variables, dead_code, unused_imports)]
use serde::{Serialize, Deserialize};
use std::sync::mpsc::{Sender, Receiver, channel}; // For in-memory mock communication
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use once_cell::sync::Lazy;

// Generic Network Message for P2P communication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkMessage {
    BlockProposal(Vec<u8>), // Serialized Block
    Transaction(Vec<u8>),   // Serialized ConcordanceTransaction
    PeerDiscoveryRequest,
    PeerDiscoveryResponse(Vec<String>), // List of peer addresses
    Custom {
        message_type: String,
        payload: Vec<u8>,
    }
}

// Trait for a P2P service
// #[async_trait::async_trait] // For later async implementation
pub trait P2PService: Send + Sync {
    fn broadcast(&self, message: NetworkMessage);
    fn send_to_peer(&self, peer_id: &str, message: NetworkMessage) -> Result<(), String>;
    // fn register_message_handler(&mut self, handler: Box<dyn Fn(NetworkMessage, String) + Send + Sync>);
    // In a real system, receiving messages would be event-driven or via a polling loop.
    // For this mock, nodes will have their own in-memory channels.
}

// --- Mock In-Memory P2P Service for Local Testnet ---
// This allows nodes (running in same process or threads) to send messages to each other.
pub struct InMemoryP2PNetwork {
    // NodeID -> Sender channel to that node
    peers: Arc<Mutex<HashMap<String, Sender<NetworkMessage>>>>,
}

impl InMemoryP2PNetwork {
    pub fn new() -> Self {
        InMemoryP2PNetwork {
            peers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn register_peer(&self, peer_id: String, sender: Sender<NetworkMessage>) {
        let mut peers_guard = self.peers.lock().unwrap();
        peers_guard.insert(peer_id, sender);
    }

    pub fn unregister_peer(&self, peer_id: &str) {
        let mut peers_guard = self.peers.lock().unwrap();
        peers_guard.remove(peer_id);
    }
}

impl P2PService for InMemoryP2PNetwork {
    fn broadcast(&self, message: NetworkMessage) {
        println!("[InMemoryP2P] Broadcasting message: {:?}", message_summary(&message));
        let peers_guard = self.peers.lock().unwrap();
        for (peer_id, sender) in peers_guard.iter() {
            if let Err(e) = sender.send(message.clone()) {
                eprintln!("[InMemoryP2P] Error broadcasting to peer {}: {}", peer_id, e);
            }
        }
    }

    fn send_to_peer(&self, peer_id: &str, message: NetworkMessage) -> Result<(), String> {
        println!("[InMemoryP2P] Sending message to peer {}: {:?}", peer_id, message_summary(&message));
        let peers_guard = self.peers.lock().unwrap();
        if let Some(sender) = peers_guard.get(peer_id) {
            sender.send(message).map_err(|e| format!("Error sending to peer {}: {}", peer_id, e))
        } else {
            Err(format!("Peer {} not found.", peer_id))
        }
    }
}

fn message_summary(msg: &NetworkMessage) -> String {
    match msg {
        NetworkMessage::BlockProposal(data) => format!("BlockProposal(size:{})", data.len()),
        NetworkMessage::Transaction(data) => format!("Transaction(size:{})", data.len()),
        NetworkMessage::PeerDiscoveryRequest => "PeerDiscoveryRequest".to_string(),
        NetworkMessage::PeerDiscoveryResponse(peers) => format!("PeerDiscoveryResponse(count:{})", peers.len()),
        NetworkMessage::Custom{message_type, payload} => format!("Custom(type:{}, size:{})", message_type, payload.len()),
    }
}

// Placeholder: TriadWeb itself might provide access to different network tiers
pub fn get_graviton_edge_network_interface() -> Arc<InMemoryP2PNetwork> {
    // For now, a global singleton for local testing
    static GRAVITON_NETWORK: Lazy<Arc<InMemoryP2PNetwork>> = Lazy::new(|| Arc::new(InMemoryP2PNetwork::new()));
    GRAVITON_NETWORK.clone()
}

pub fn status() -> &'static str {
    "TriadWeb Networking Primitives (Mocked)"
}
