// sgs_foundations/triad_web/src/lib.rs
#![allow(unused_variables, dead_code, unused_imports)]
use serde::{Serialize, Deserialize};
use std::error::Error;
use libp2p::Multiaddr;
use libp2p::PeerId;
use std::sync::Arc;
use tokio::sync::mpsc;
use std::path::PathBuf;

// --- Existing module declarations ---
pub mod network_behaviour; // Ensure this is public if AuroraTopic is used outside
pub mod libp2p_service;

pub use libp2p_service::{Libp2pService, P2PService, P2PEvent};
// Re-export AuroraTopic if it's defined in network_behaviour and used externally
pub use network_behaviour::AuroraTopic;


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum NetworkMessage {
    // Consensus related
    BlockProposal(Vec<u8>), 
    Transaction(Vec<u8>),   
    // BlockVote(Vec<u8>), // Old - can be removed or kept if used for something else

    // --- NEW Chain Synchronization Messages ---
    BlockRequestRange {
        start_height: u64,
        end_height: Option<u64>, 
        max_blocks_to_send: Option<u32>, 
        requesting_peer_id: String, 
    },
    BlockResponseBatch {
        blocks_data: Vec<Vec<u8>>, 
        from_height: u64,
        to_height: u64,
    },
    NoBlocksInRange {
        requested_start: u64,
        requested_end: Option<u64>,
        responder_peer_id: String, 
    },
    // --- End Chain Synchronization Messages ---

    // --- NEW Basic Consensus Attestation Message ---
    BlockAttestation {
        block_hash: String,
        block_height: u64,
        attestor_peer_id_str: String, // PeerId of the attesting node as String
        // signature: Vec<u8>, // For a real system; mock for now
    },
    // --- End Basic Consensus Attestation Message ---

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
        topic_hash: libp2p::gossipsub::TopicHash, // Keep this for context if needed
        message: NetworkMessage,
    },
    DirectMessage { // Retaining this variant in case direct messaging is implemented later
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
    "TriadWeb Networking Primitives (libp2p based, persistent keys, sync & attestation messages)"
}

pub fn message_summary(msg: &NetworkMessage) -> String {
    match msg {
        NetworkMessage::BlockProposal(data) => format!("BlockProposal(size:{})", data.len()),
        NetworkMessage::Transaction(data) => format!("Transaction(size:{})", data.len()),
        NetworkMessage::BlockAttestation { block_hash, block_height, attestor_peer_id_str, .. } => {
            format!("BlockAttestation(H:{}, Hash:{:.8}..., Attestor:{:.8}..)", block_height, block_hash, attestor_peer_id_str)
        }
        NetworkMessage::NodeStateQuery { .. } => "NodeStateQuery".to_string(),
        NetworkMessage::NodeStateResponse(data) => format!("NodeStateResponse(size:{})", data.len()),
        NetworkMessage::BlockRequestRange { start_height, end_height, .. } => format!("BlockRequestRange(start:{}, end:{:?})", start_height, end_height),
        NetworkMessage::BlockResponseBatch { blocks_data, from_height, to_height } => format!("BlockResponseBatch(count:{}, from:{}, to:{})", blocks_data.len(), from_height, to_height),
        NetworkMessage::NoBlocksInRange { requested_start, requested_end, .. } => format!("NoBlocksInRange(start:{}, end:{:?})", requested_start, requested_end),
        _ => format!("{:?}", msg).chars().take(80).collect(), // Fallback for other types
    }
}