#![allow(unused_variables, dead_code, unused_imports)]
use serde::{Serialize, Deserialize};

pub mod tcp_p2p;

// Re-export key types and traits from tcp_p2p
pub use tcp_p2p::{AsyncP2PService, IncomingMessage, TcpP2PManager};


// Generic Network Message for P2P communication
// This needs to be accessible by both tcp_p2p.rs and other crates using triad_web.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkMessage {
    BlockProposal(Vec<u8>), // Serialized Block from ecliptic_concordance
    Transaction(Vec<u8>),   // Serialized TransactionPayload from ecliptic_concordance
    PeerDiscoveryRequest,       // TODO
    PeerDiscoveryResponse(Vec<String>), // TODO: List of peer SocketAddr as strings
    NodeStateQuery, // For CLI to request state
    NodeStateResponse(Vec<u8>), // Serialized node state summary
    Custom {
        message_type: String,
        payload: Vec<u8>,
    }
}

// This is the main function that nodes will call to initialize their P2P stack
pub fn initialize_p2p_service(
    node_id: String, 
    listen_addr_str: &str, 
    initial_peers_str: Option<String>
) -> Result<(Arc<TcpP2PManager>, tokio::sync::mpsc::Receiver<IncomingMessage>), Box<dyn std::error::Error + Send + Sync>> {
    TcpP2PManager::new(node_id, listen_addr_str, initial_peers_str)
}


pub fn status() -> &'static str {
    "TriadWeb Networking Primitives (TCP P2P Implemented)"
}
