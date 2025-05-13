use ecliptic_concordance::{
    ConsensusState as NodeLocalConsensusState, 
    sequencer_create_block, submit_transaction_payload, validate_and_apply_block,
    Block, TransactionPayload, ConcordanceTransaction, get_current_state_summary
};
// Use the P2P service from triad_web
use triad_web::{NetworkMessage, AsyncP2PService, initialize_p2p_service, IncomingMessage};
use serde_json;
use clap::Parser;
use std::fs::OpenOptions;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::sync::Arc; // Use std::sync::Arc for the service trait object
use tokio::sync::Mutex as TokioMutex; // Tokio's Mutex for async state

const SEQUENCER_ID_PREFIX: &str = "sequencer-";

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(long, help = "Unique Node ID for this instance")]
    node_id: String,
    #[clap(long, help = "Listen address and port for P2P, e.g., 127.0.0.1:8001")]
    listen_addr: String,
    #[clap(long, value_delimiter = ',', help = "Comma-separated peer addresses to connect to")]
    peers: Option<String>,
    #[clap(long, help = "Path to the data directory for this node")]
    data_dir: PathBuf,
}

fn load_consensus_state_from_disk(node_id: &str, blockchain_file_path: &Path) -> NodeLocalConsensusState {
    let mut state = NodeLocalConsensusState::new(node_id.to_string());
    if blockchain_file_path.exists() {
        if let Ok(file) = File::open(blockchain_file_path) {
            let reader = io::BufReader::new(file);
            if let Some(Ok(last_line)) = reader.lines().last() {
                if !last_line.trim().is_empty() {
                    if let Ok(last_block) = serde_json::from_str::<Block>(&last_line) {
                        state.current_height = last_block.height;
                        state.last_block_hash = last_block.block_hash;
                        println!("[Node:{}] Restored consensus: Height {}, LastHash {}", node_id, state.current_height, state.last_block_hash);
                    }
                }
            }
        }
    }
    state
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct NodeStateSummary {
    node_id: String,
    height: u64,
    last_block_hash: String,
}


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();
    if !args.data_dir.exists() { std::fs::create_dir_all(&args.data_dir)?; }
    let blockchain_file_path = args.data_dir.join(format!("{}_blockchain.jsonl", args.node_id));
    
    let is_sequencer = args.node_id.starts_with(SEQUENCER_ID_PREFIX);
    println!("[Node:{}] Starting. Listen: {}. Sequencer: {}. ChainFile: {:?}", 
        args.node_id, args.listen_addr, is_sequencer, blockchain_file_path);

    let consensus_state_arc = Arc::new(TokioMutex::new( // Use tokio's Mutex
        load_consensus_state_from_disk(&args.node_id, &blockchain_file_path)
    ));

    // Initialize TCP P2P Service
    let (p2p_service, mut p2p_message_rx) = initialize_p2p_service(
        args.node_id.clone(), 
        &args.listen_addr, 
        args.peers.clone() // Pass peers string to P2P service for it to handle connections
    )?;
    println!("[Node:{}] P2P service initialized.", args.node_id);

    if is_sequencer {
        let node_id_clone = args.node_id.clone();
        let p2p_broadcast_clone = p2p_service.clone(); // Arc clone
        let blockchain_file_path_clone = blockchain_file_path.clone();
        let consensus_state_sequencer_arc = consensus_state_arc.clone();

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;
                let mut local_consensus_state = consensus_state_sequencer_arc.lock().await;
                match sequencer_create_block(&mut local_consensus_state, &node_id_clone) {
                    Ok(new_block) => {
                        println!("[Node:{}:Seq] Created Block H:{}. Broadcasting...", node_id_clone, new_block.height);
                        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&blockchain_file_path_clone) {
                            if writeln!(file, "{}", serde_json::to_string(&new_block).unwrap()).is_err() { /* log error */ }
                        }
                        if let Ok(serialized_block) = bincode::serialize(&new_block) { // Use bincode for network
                            if p2p_broadcast_clone.broadcast(NetworkMessage::BlockProposal(serialized_block)).is_err(){
                                // eprintln!("[Node:{}:Seq] Broadcast failed, no active listeners.", node_id_clone);
                            }
                        } else { eprintln!("[Node:{}:Seq] Error serializing block with bincode.", node_id_clone); }
                    }
                    Err(e) if e == "No pending transactions to create a block." => {}
                    Err(e) => eprintln!("[Node:{}:Seq] Error creating block: {}", node_id_clone, e),
                }
                drop(local_consensus_state);
            }
        });
    }

    println!("[Node:{}] Listening for P2P messages...", args.node_id);
    loop {
        tokio::select! {
            Some(incoming_message) = p2p_message_rx.recv() => {
                // println!("[Node:{}] Received from {:?}: {:?}", args.node_id, incoming_message.from_addr, incoming_message.message);
                match incoming_message.message {
                    NetworkMessage::BlockProposal(serialized_block) => {
                        match bincode::deserialize::<Block>(&serialized_block) { // Use bincode
                            Ok(block) => {
                                if block.proposer_id == args.node_id { continue; }
                                let mut local_consensus_state = consensus_state_arc.lock().await;
                                match validate_and_apply_block(&mut local_consensus_state, &block) {
                                    Ok(()) => {
                                        println!("[Node:{}] Validated Block H:{} from {}", args.node_id, block.height, block.proposer_id);
                                        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&blockchain_file_path){
                                            if writeln!(file, "{}", serde_json::to_string(&block).unwrap()).is_err(){/* log */}
                                        }
                                    }
                                    Err(e) => eprintln!("[Node:{}] Invalid block (H:{}): {}", args.node_id, block.height, e),
                                }
                                drop(local_consensus_state);
                            }
                            Err(e) => eprintln!("[Node:{}] Error deserializing block with bincode: {}", args.node_id, e),
                        }
                    }
                    NetworkMessage::Transaction(serialized_tx_payload_data) => { // This is Vec<u8> from TransactionPayload
                        if is_sequencer {
                             match serde_json::from_slice::<TransactionPayload>(&serialized_tx_payload_data) { // Expect JSON from CLI
                                Ok(tx_payload) => {
                                    let mut local_consensus_state = consensus_state_arc.lock().await;
                                    match submit_transaction_payload(&mut local_consensus_state, tx_payload.data) {
                                        Ok(tx_id) => println!("[Node:{}:Seq] Mempooled TxID: {} via P2P", args.node_id, tx_id),
                                        Err(e) => eprintln!("[Node:{}:Seq] Error submitting P2P payload: {}", args.node_id, e),
                                    }
                                    drop(local_consensus_state);
                                }
                                Err(e) => eprintln!("[Node:{}:Seq] Error deserializing P2P tx payload from JSON: {}", args.node_id, e),
                             }
                        }
                    }
                    NetworkMessage::NodeStateQuery => { // Respond to state query
                        println!("[Node:{}] Received NodeStateQuery from {}", args.node_id, incoming_message.from_addr);
                        let local_consensus_state = consensus_state_arc.lock().await;
                        let summary = NodeStateSummary {
                            node_id: args.node_id.clone(),
                            height: local_consensus_state.current_height,
                            last_block_hash: local_consensus_state.last_block_hash.clone(),
                        };
                        drop(local_consensus_state);
                        if let Ok(serialized_summary) = bincode::serialize(&summary) {
                            // This send_direct needs the actual TcpP2PManager instance
                            let p2p_service_clone = p2p_service.clone();
                            tokio::spawn(async move {
                                if let Err(e) = p2p_service_clone.send_direct(incoming_message.from_addr, NetworkMessage::NodeStateResponse(serialized_summary)).await {
                                    eprintln!("[Node:{}] Error sending state response to {}: {}", args.node_id, incoming_message.from_addr, e);
                                }
                            });
                        }
                    }
                    _ => {}
                }
            }
            else => {
                eprintln!("[Node:{}] P2P message channel closed. Shutting down.", args.node_id);
                break;
            }
        }
    }
    Ok(())
}
