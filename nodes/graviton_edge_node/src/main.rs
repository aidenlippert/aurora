use ecliptic_concordance::{
    sequencer_create_block, submit_transaction_payload, validate_and_apply_block,
    Block, TransactionPayload, ConcordanceTransaction // Added ConcordanceTransaction
};
use triad_web::{NetworkMessage, P2PService, initialize_tcp_p2p_service, IncomingMessage}; // Use TCP P2P
use serde::{Serialize, Deserialize};
use serde_json;
use clap::Parser;
use std::fs::OpenOptions;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::net::SocketAddr;
use std::collections::HashSet;
use tokio::sync::mpsc; // For message passing within the node

const SEQUENCER_ID_PREFIX: &str = "sequencer-"; // Allow multiple sequencers by convention if needed

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(long, help = "Unique Node ID for this instance")]
    node_id: String,

    #[clap(long, help = "Listen address and port for P2P communication, e.g., 127.0.0.1:8001")]
    listen_addr: String,

    #[clap(long, value_delimiter = ',', help = "Comma-separated list of peer addresses to connect to, e.g., 127.0.0.1:8000,127.0.0.1:8002")]
    peers: Option<String>,
    
    #[clap(long, help = "Path to the data directory for this node")]
    data_dir: PathBuf,
}

// Node's local state for consensus (not shared static anymore)
struct NodeConsensusState {
    current_height: u64,
    last_block_hash: String,
    pending_transactions: Vec<ConcordanceTransaction>, // Using the correct type
}

impl NodeConsensusState {
    fn new() -> Self {
        NodeConsensusState {
            current_height: 0,
            last_block_hash: "GENESIS_HASH_0.0.1".to_string(),
            pending_transactions: Vec::new(),
        }
    }
    // Load from the end of the blockchain file
    fn load_from_disk(blockchain_file_path: &Path) -> Self {
        let mut state = Self::new();
        if blockchain_file_path.exists() {
            if let Ok(file) = File::open(blockchain_file_path) {
                let reader = io::BufReader::new(file);
                if let Some(Ok(last_line)) = reader.lines().last() {
                    if let Ok(last_block) = serde_json::from_str::<Block>(&last_line) {
                        state.current_height = last_block.height;
                        state.last_block_hash = last_block.block_hash;
                        println!("[Node] Restored state: Height {}, Hash {}", state.current_height, state.last_block_hash);
                    }
                }
            }
        }
        state
    }
}


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if !args.data_dir.exists() {
        std::fs::create_dir_all(&args.data_dir)?;
    }
    let blockchain_file_path = args.data_dir.join(format!("{}_blockchain.jsonl", args.node_id));
    
    let is_sequencer = args.node_id.starts_with(SEQUENCER_ID_PREFIX);
    println!("[Node:{}] Starting. Listen: {}. Sequencer: {}. Chain: {:?}", 
        args.node_id, args.listen_addr, is_sequencer, blockchain_file_path);

    // Initialize node-local consensus state
    let mut consensus_state = NodeConsensusState::load_from_disk(&blockchain_file_path);

    // Initialize TCP P2P Service
    let (p2p_service, mut p2p_message_rx) = initialize_tcp_p2p_service(args.node_id.clone(), &args.listen_addr)?;
    
    // Connect to initial peers
    if let Some(peers_str) = args.peers {
        let peer_socket_addrs: Vec<SocketAddr> = peers_str.split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        // The connect_to_initial_peers function needs to be part of TcpP2PManager or called appropriately.
        // For now, let's assume the TcpP2PManager handles outgoing connections upon start or a separate call.
        // This is simplified; robust peer management is complex.
        // Let's imagine TcpP2PManager::new_simple also tries to connect or has a method.
        // For now, we'll just print what we would connect to.
        println!("[Node:{}] Would attempt to connect to peers: {:?}", args.node_id, peer_socket_addrs);
        // In a real scenario, the P2P service would manage these connections.
        // For this initial version, we'll rely on peers connecting to this node's listener.
    }


    // Sequencer routine
    if is_sequencer {
        let node_id_clone = args.node_id.clone();
        let p2p_broadcast_clone = p2p_service.clone();
        let data_dir_clone = args.data_dir.clone(); // Clone for async task

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(15)).await; // Create block every 15s
                
                // Create a new block with transactions from its *own* pending_transactions
                // The global CONSENSUS_STATE is no longer used here for block creation.
                // Each node must maintain its own state.
                // This requires EclipticConcordance functions to take &mut NodeConsensusState.
                // This is a major refactor of ecliptic_concordance, which we'll defer.
                // HACK for v0.0.1: Sequencer uses the global static for creating blocks,
                // but this isn't truly distributed.
                match ecliptic_concordance::sequencer_create_block(&node_id_clone) {
                    Ok(new_block) => {
                        println!("[Node:{}:Sequencer] Created Block Height: {}. Broadcasting...", node_id_clone, new_block.height);
                        let mut file = OpenOptions::new().create(true).append(true)
                            .open(data_dir_clone.join(format!("{}_blockchain.jsonl", node_id_clone))).unwrap();
                        writeln!(file, "{}", serde_json::to_string(&new_block).unwrap()).unwrap();

                        match serde_json::to_vec(&new_block) {
                            Ok(serialized_block) => {
                                p2p_broadcast_clone.broadcast(NetworkMessage::BlockProposal(serialized_block));
                            }
                            Err(e) => eprintln!("[Node:{}:Sequencer] Error serializing block: {}", node_id_clone, e),
                        }
                    }
                    Err(e) if e == "No pending transactions to create a block." => { /* Log less verbosely or ignore */ }
                    Err(e) => eprintln!("[Node:{}:Sequencer] Error creating block: {}", node_id_clone, e),
                }
            }
        });
    }

    // Main message processing loop
    println!("[Node:{}] Listening for P2P messages...", args.node_id);
    while let Some(incoming_message) = p2p_message_rx.recv().await {
        println!("[Node:{}] Received from {:?}: {:?}", args.node_id, incoming_message.from, incoming_message.message);
        match incoming_message.message {
            NetworkMessage::BlockProposal(serialized_block) => {
                match serde_json::from_slice::<Block>(&serialized_block) {
                    Ok(block) => {
                        if block.proposer_id == args.node_id { continue; } // Don't process own blocks via this path

                        // Follower nodes validate and apply to their *own* state.
                        // This requires validate_and_apply_block to also take &mut NodeConsensusState.
                        // HACK for v0.0.1: Validation uses global static. Not correct for distributed.
                        match ecliptic_concordance::validate_and_apply_block(&block) {
                            Ok(()) => {
                                println!("[Node:{}] Validated and applied Block Height: {} from {}", args.node_id, block.height, block.proposer_id);
                                let mut file = OpenOptions::new().create(true).append(true).open(&blockchain_file_path)?;
                                writeln!(file, "{}", serde_json::to_string(&block)?)?;
                            }
                            Err(e) => eprintln!("[Node:{}] Invalid block (Height: {}): {}", args.node_id, block.height, e),
                        }
                    }
                    Err(e) => eprintln!("[Node:{}] Error deserializing block: {}", args.node_id, e),
                }
            }
            NetworkMessage::Transaction(serialized_tx_payload) => {
                if is_sequencer { // Only sequencer adds to its (global static) mempool for now
                    match serde_json::from_slice::<TransactionPayload>(&serialized_tx_payload) {
                        Ok(tx_payload) => {
                            match submit_transaction_payload(tx_payload.data) {
                                Ok(tx_id) => println!("[Node:{}:Sequencer] Added payload to mempool via P2P, TxID: {}", args.node_id, tx_id),
                                Err(e) => eprintln!("[Node:{}:Sequencer] Error submitting P2P payload: {}", args.node_id, e),
                            }
                        }
                        Err(e) => eprintln!("[Node:{}:Sequencer] Error deserializing P2P tx payload: {}", args.node_id, e),
                     }
                } else {
                    println!("[Node:{}:Follower] Received transaction, but not sequencer. Ignoring.", args.node_id);
                }
            }
            _ => {}
        }
    }
    Ok(())
}
