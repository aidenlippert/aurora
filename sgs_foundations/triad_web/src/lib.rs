#![allow(unused_variables, dead_code, unused_imports)]
use serde::{Serialize, Deserialize};
use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;

pub mod tcp_p2p;

// Re-export key types and traits from tcp_p2p
pub use tcp_p2p::{AsyncP2PService, IncomingMessage, TcpP2PManager};

// Generic Network Message for P2P communication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkMessage {
    // Consensus related
    BlockProposal(Vec<u8>), // Serialized ecliptic_concordance::Block
    Transaction(Vec<u8>),   // Serialized ecliptic_concordance::TransactionPayload (or full Tx)
    BlockVote(Vec<u8>),     // Serialized vote for a block (for more advanced consensus)

    // Peer Discovery & Management (Basic)
    Ping(String), // Ping with a nonce
    Pong(String), // Pong with the same nonce
    Identify(String), // Node ID
    PeerListRequest,
    PeerListResponse(Vec<String>), // List of peer SocketAddr as strings

    // Application / Node Specific Queries & Responses
    NodeStateQuery, // Request for node's current state summary
    NodeStateResponse(Vec<u8>), // Serialized NodeStateSummary

    // For more generic application messages if needed
    Custom {
        message_type: String,
        payload: Vec<u8>,
    }
}

// This is the main function that nodes will call to initialize their P2P stack
pub async fn initialize_p2p_service(
    node_id: String, 
    listen_addr_str: &str, 
    initial_peers_str: Option<String>
) -> Result<(Arc<TcpP2PManager>, mpsc::Receiver<IncomingMessage>), Box<dyn Error + Send + Sync>> {
    TcpP2PManager::new(node_id, listen_addr_str, initial_peers_str).await
}

pub fn status() -> &'static str {
    "TriadWeb Networking Primitives (TCP P2P Implemented)"
}
