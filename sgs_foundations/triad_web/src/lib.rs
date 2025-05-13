#![allow(unused_variables, dead_code, unused_imports)]
use serde::{Serialize, Deserialize};
use std::error::Error;
use libp2p::Multiaddr;
use libp2p::PeerId;
use std::sync::Arc;
use tokio::sync::mpsc;
use std::path::PathBuf; 

// --- Existing module declarations ---
pub mod network_behaviour;
pub mod libp2p_service;

pub use libp2p_service::{Libp2pService, P2PService, P2PEvent};

// ecliptic_concordance::Block will be needed for BlockResponseBatch
// but we avoid direct dependency here if NetworkMessage just carries Vec<u8>
// However, for clarity in the enum variant, it's good to know the type.
// We'll assume Block is defined elsewhere and serialized/deserialized correctly.

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum NetworkMessage {
    // Consensus related
    BlockProposal(Vec<u8>), // Serialized ecliptic_concordance::Block
    Transaction(Vec<u8>),   // Serialized ecliptic_concordance::TransactionPayload
    BlockVote(Vec<u8>),     

    // --- NEW Chain Synchronization Messages ---
    BlockRequestRange { 
        start_height: u64,
        // count: u32, // Alternative: request a certain number of blocks
        end_height: Option<u64>, // Request up to this height (inclusive)
        max_blocks_to_send: Option<u32>, // Max blocks peer should send in one batch
        requesting_peer_id: String, // PeerId of the requester as string
    },
    BlockResponseBatch { 
        blocks_data: Vec<Vec<u8>>, // Each Vec<u8> is a serialized Block
        from_height: u64,
        to_height: u64,
    },
    NoBlocksInRange { 
        requested_start: u64, 
        requested_end: Option<u64>,
        responder_peer_id: String, // PeerId of the responder
    },
    // --- End Chain Synchronization Messages ---

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
    DirectMessage { 
        source: PeerId,
        message: NetworkMessage,
    },
    PeerConnected(PeerId),
    PeerDisconnected(PeerId),
}

pub async fn initialize_p2p_service(
    node_id_for_logs: String, 
    data_dir: PathBuf, 
    listen_multiaddrs: Vec<Multiaddr>, 
    bootstrap_peers: Vec<Multiaddr>
) -> Result<(Arc<Libp2pService>, mpsc::Receiver<AppP2PEvent>), Box<dyn Error + Send + Sync>> {
    Libp2pService::new(node_id_for_logs, data_dir, listen_multiaddrs, bootstrap_peers).await
}

pub fn status() -> &'static str {
    "TriadWeb Networking Primitives (libp2p based, persistent keys, sync messages)"
}

pub fn message_summary(msg: &NetworkMessage) -> String {
    match msg {
        NetworkMessage::BlockProposal(data) => format!("BlockProposal(size:{})", data.len()),
        NetworkMessage::Transaction(data) => format!("Transaction(size:{})", data.len()),
        NetworkMessage::NodeStateQuery { .. } => "NodeStateQuery".to_string(),
        NetworkMessage::NodeStateResponse(data) => format!("NodeStateResponse(size:{})", data.len()),
        NetworkMessage::BlockRequestRange { start_height, end_height, .. } => format!("BlockRequestRange(start:{}, end:{:?})", start_height, end_height),
        NetworkMessage::BlockResponseBatch { blocks_data, from_height, to_height } => format!("BlockResponseBatch(count:{}, from:{}, to:{})", blocks_data.len(), from_height, to_height),
        NetworkMessage::NoBlocksInRange { requested_start, requested_end, .. } => format!("NoBlocksInRange(start:{}, end:{:?})", requested_start, requested_end),
        NetworkMessage::GenericRequest{ request_id, payload } => format!("GenericRequest(id:{}, size:{})", request_id, payload.len()),
        NetworkMessage::GenericResponse{ request_id, payload } => format!("GenericResponse(id:{}, size:{})", request_id, payload.len()),
        _ => format!("{:?}", msg).chars().take(80).collect(),
    }
}