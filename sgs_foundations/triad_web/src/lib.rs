// sgs_foundations/triad_web/src/lib.rs
#![allow(unused_variables, dead_code, unused_imports)]
use serde::{Serialize, Deserialize};
use std::error::Error;
use libp2p::Multiaddr;
use libp2p::PeerId;
use std::sync::Arc;
use tokio::sync::mpsc;
use std::path::PathBuf;

use ecliptic_concordance::{Attestation as ConsensusAttestation, AuroraTransaction}; // Ensure AuroraTransaction is imported

pub mod network_behaviour;
pub mod libp2p_service;

pub use libp2p_service::{Libp2pService, P2PService, P2PEvent};
pub use network_behaviour::AuroraTopic;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum NetworkMessage {
    BlockProposal(Vec<u8>), 
    Transaction(AuroraTransaction),   // <<< --- THIS MUST BE AuroraTransaction
    
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
    
    BlockAttestation(ConsensusAttestation), 

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
    "TriadWeb Networking Primitives (libp2p based, with signed attestations)"
}

pub fn message_summary(msg: &NetworkMessage) -> String {
    match msg {
        NetworkMessage::BlockProposal(data) => format!("BlockProposal(size:{})", data.len()),
        NetworkMessage::Transaction(aurora_tx) => format!("Transaction({:?})", aurora_tx), // Correctly uses aurora_tx
        NetworkMessage::BlockAttestation(att) => {
            format!("BlockAttestation(H:{}, Hash:{:.8}..., AttestorPK:{:.8}..)", 
                    att.block_height, att.block_hash, att.attestor_pk_hex()) 
        }
        // ... rest of message_summary ...
        NetworkMessage::NodeStateQuery { .. } => "NodeStateQuery".to_string(),
        NetworkMessage::NodeStateResponse(data) => format!("NodeStateResponse(size:{})", data.len()),
        NetworkMessage::BlockRequestRange { start_height, end_height, .. } => format!("BlockRequestRange(start:{}, end:{:?})", start_height, end_height),
        NetworkMessage::BlockResponseBatch { blocks_data, from_height, to_height } => format!("BlockResponseBatch(count:{}, from:{}, to:{})", blocks_data.len(), from_height, to_height),
        NetworkMessage::NoBlocksInRange { requested_start, requested_end, .. } => format!("NoBlocksInRange(start:{}, end:{:?})", requested_start, requested_end),
        _ => format!("{:?}", msg).chars().take(80).collect(),
    }
}