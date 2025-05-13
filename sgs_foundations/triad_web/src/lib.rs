#![allow(unused_variables, dead_code, unused_imports)]
use serde::{Serialize, Deserialize};
use std::error::Error;
// use std::net::SocketAddr; // No longer needed for this signature
use libp2p::Multiaddr;
use libp2p::PeerId;
use std::sync::Arc;
use tokio::sync::mpsc;
use std::path::PathBuf; // For passing data_dir for keys

pub mod network_behaviour;
pub mod libp2p_service;

pub use libp2p_service::{Libp2pService, P2PService, P2PEvent}; // P2PEvent might be internal only

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum NetworkMessage {
    BlockProposal(Vec<u8>), 
    Transaction(Vec<u8>),   
    BlockVote(Vec<u8>),     
    NodeStateQuery { responder_peer_id: Option<String> }, 
    NodeStateResponse(Vec<u8>), 
    GenericRequest { request_id: String, payload: Vec<u8> },
    GenericResponse { request_id: String, payload: Vec<u8> },
    Custom {
        message_type: String,
        payload: Vec<u8>,
    }
}

#[derive(Debug)]
pub enum AppP2PEvent {
    GossipsubMessage {
        source: PeerId, 
        topic_hash: libp2p::gossipsub::TopicHash,
        message: NetworkMessage, 
    },
    DirectMessage { // Placeholder if direct messaging protocol is added
        source: PeerId,
        message: NetworkMessage,
    },
    PeerConnected(PeerId),
    PeerDisconnected(PeerId),
}

// UPDATED signature to include data_dir
pub async fn initialize_p2p_service(
    node_id_for_logs: String, 
    data_dir: PathBuf, // ADDED: For key storage
    listen_multiaddrs: Vec<Multiaddr>, 
    bootstrap_peers: Vec<Multiaddr>
) -> Result<(Arc<Libp2pService>, mpsc::Receiver<AppP2PEvent>), Box<dyn Error + Send + Sync>> {
    Libp2pService::new(node_id_for_logs, data_dir, listen_multiaddrs, bootstrap_peers).await
}

pub fn status() -> &'static str {
    "TriadWeb Networking Primitives (libp2p based, persistent keys)"
}

pub fn message_summary(msg: &NetworkMessage) -> String {
    match msg {
        NetworkMessage::BlockProposal(data) => format!("BlockProposal(size:{})", data.len()),
        NetworkMessage::Transaction(data) => format!("Transaction(size:{})", data.len()),
        NetworkMessage::NodeStateQuery { .. } => "NodeStateQuery".to_string(),
        NetworkMessage::NodeStateResponse(data) => format!("NodeStateResponse(size:{})", data.len()),
        NetworkMessage::GenericRequest{ request_id, payload } => format!("GenericRequest(id:{}, size:{})", request_id, payload.len()),
        NetworkMessage::GenericResponse{ request_id, payload } => format!("GenericResponse(id:{}, size:{})", request_id, payload.len()),
        _ => format!("{:?}", msg).chars().take(80).collect(),
    }
}