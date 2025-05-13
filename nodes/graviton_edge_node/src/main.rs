use ecliptic_concordance::{
    // Renamed ConsensusState to NodeLocalConsensusState to avoid confusion with module name
    ConsensusState as NodeLocalConsensusState, 
    sequencer_create_block, submit_transaction_payload, validate_and_apply_block,
    get_current_state_summary, Block, TransactionPayload, ConcordanceTransaction
};
use triad_web::{NetworkMessage, P2PService, initialize_tcp_p2p_service, IncomingMessage};
use serde_json;
use clap::Parser;
use std::fs::{OpenOptions, File};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::net::SocketAddr;
use tokio::sync::Mutex; // Use tokio's Mutex for async state
use std::sync::Arc;


const SEQUENCER_ID_PREFIX: &str = "sequencer-";

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(long, help = "Unique Node ID for this instance")]
    node_id: String,
    #[clap(long, help = "Listen address and port for P2P communication, e.g., 127.0.0.1:8001")]
    listen_addr: String,
    #[clap(long, value_delimiter = ',', help = "Comma-separated list of peer addresses to connect to")]
    peers: Option<String>,
    #[clap(long, help = "Path to the data directory for this node")]
    data_dir: PathBuf,
}

fn load_consensus_state_from_disk(node_id: &str, blockchain_file_path: &Path) -> NodeLocalConsensusState {
    let mut state = NodeLocalConsensusState::new(node_id.to_string()); // Pass node_id
    if blockchain_file_path.exists() {
        if let Ok(file) = File::open(blockchain_file_path) {
            let reader = io::BufReader::new(file);
            // Iterate from the end or just take the last valid block
            if let Some(Ok(last_line)) = reader.lines().last() {
                if !last_line.trim().is_empty() { // Ensure line is not empty
                    if let Ok(last_block) = serde_json::from_str::<Block>(&last_line) {
                        state.current_height = last_block.height;
                        state.last_block_hash = last_block.block_hash;
                        println!("[Node:{}] Restored consensus state: Height {}, LastHash {}", node_id, state.current_height, state.last_block_hash);
                    } else {
                        eprintln!("[Node:{}] Failed to parse last block from chain file: {}", node_id, last_line);
                    }
                }
            }
        } else {
             eprintln!("[Node:{}] Could not open blockchain file {:?} for reading.", node_id, blockchain_file_path);
        }
    }
    state
}


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if !args.data_dir.exists() {
        std::fs::create_dir_all(&args.data_dir)?;
    }
    let blockchain_file_path = args.data_dir.join(format!("{}_blockchain.jsonl", args.node_id));
    
    let is_sequencer = args.node_id.starts_with(SEQUENCER_ID_PREFIX);
    println!("[Node:{}] Starting. Listen: {}. Sequencer: {}. ChainFile: {:?}", 
        args.node_id, args.listen_addr, is_sequencer, blockchain_file_path);

    // Each node has its own Arc<Mutex<NodeLocalConsensusState>>
    let consensus_state_arc = Arc::new(Mutex::new(
        load_consensus_state_from_disk(&args.node_id, &blockchain_file_path)
    ));

    let (p2p_service, mut p2p_message_rx) = initialize_tcp_p2p_service(args.node_id.clone(), &args.listen_addr)?;
    
    if let Some(peers_str) = args.peers.clone() { // Clone peers_str
        let peer_socket_addrs: Vec<SocketAddr> = peers_str.split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        // This connect_to_initial_peers is conceptual for TcpP2PManager;
        // Actual connection attempts would happen within the P2P service logic when it starts.
        // For now, we assume the listener on other nodes will accept our connection when we broadcast.
        if let Some(p2p_manager) = Arc::downcast::<triad_web::tcp_p2p::TcpP2PManager>(p2p_service.clone()).ok() {
             p2p_manager.connect_to_initial_peers(peer_socket_addrs, p2p_message_rx.sender_handle_for_direct_messages_if_needed()).await;
             // The above line for sender_handle is pseudo-code. The TcpP2PManager needs to manage how it gets messages to send to specific peers.
             // Let's simplify: the current design of TcpP2PManager might not support this call directly,
             // it implicitly connects when `send_direct` is called or through a background peer management task.
             // For now, we'll let connections happen organically or when broadcast/send_direct is used.
             println!("[Node:{}] Initial peer connection attempts would be managed by P2P service for: {:?}", args.node_id, peers_str);
        }
    }


    if is_sequencer {
        let node_id_clone = args.node_id.clone();
        let p2p_broadcast_clone = p2p_service.clone();
        let blockchain_file_path_clone = blockchain_file_path.clone();
        let consensus_state_sequencer_arc = consensus_state_arc.clone();

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;
                
                let mut local_consensus_state = consensus_state_sequencer_arc.lock().await;
                // Sequencer creates block using ITS OWN local consensus state
                match sequencer_create_block(&mut local_consensus_state, &node_id_clone) {
                    Ok(new_block) => {
                        println!("[Node:{}:Sequencer] Created Block Height: {}. Persisting and Broadcasting...", node_id_clone, new_block.height);
                        // Persist to its own file
                        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&blockchain_file_path_clone) {
                            if writeln!(file, "{}", serde_json::to_string(&new_block).unwrap()).is_err() {
                                eprintln!("[Node:{}:Sequencer] Error writing block to file.", node_id_clone);
                            }
                        } else {
                             eprintln!("[Node:{}:Sequencer] Error opening blockchain file for writing.", node_id_clone);
                        }
                        // Broadcast
                        match serde_json::to_vec(&new_block) {
                            Ok(serialized_block) => {
                                p2p_broadcast_clone.broadcast(NetworkMessage::BlockProposal(serialized_block));
                            }
                            Err(e) => eprintln!("[Node:{}:Sequencer] Error serializing block: {}", node_id_clone, e),
                        }
                    }
                    Err(e) if e == "No pending transactions to create a block." => {}
                    Err(e) => eprintln!("[Node:{}:Sequencer] Error creating block: {}", node_id_clone, e),
                }
                // Explicitly drop lock before sleep
                drop(local_consensus_state);
            }
        });
    }

    println!("[Node:{}] Listening for P2P messages...", args.node_id);
    loop {
        tokio::select! {
            Some(incoming_message) = p2p_message_rx.recv() => {
                println!("[Node:{}] Received from {:?}: {:?}", args.node_id, incoming_message.from, incoming_message.message);
                match incoming_message.message {
                    NetworkMessage::BlockProposal(serialized_block) => {
                        match serde_json::from_slice::<Block>(&serialized_block) {
                            Ok(block) => {
                                if block.proposer_id == args.node_id { continue; }

                                let mut local_consensus_state = consensus_state_arc.lock().await;
                                match validate_and_apply_block(&mut local_consensus_state, &block) {
                                    Ok(()) => {
                                        println!("[Node:{}] Validated and applied Block Height: {} from {}", args.node_id, block.height, block.proposer_id);
                                        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&blockchain_file_path){
                                            if writeln!(file, "{}", serde_json::to_string(&block).unwrap()).is_err(){
                                                 eprintln!("[Node:{}] Error writing received block to file.", args.node_id);
                                            }
                                        } else {
                                            eprintln!("[Node:{}] Error opening own blockchain file for writing.", args.node_id);
                                        }
                                    }
                                    Err(e) => eprintln!("[Node:{}] Invalid block (Height: {}): {}", args.node_id, block.height, e),
                                }
                                drop(local_consensus_state);
                            }
                            Err(e) => eprintln!("[Node:{}] Error deserializing block: {}", args.node_id, e),
                        }
                    }
                    NetworkMessage::Transaction(serialized_tx_payload) => {
                        if is_sequencer {
                             match serde_json::from_slice::<TransactionPayload>(&serialized_tx_payload) {
                                Ok(tx_payload) => {
                                    let mut local_consensus_state = consensus_state_arc.lock().await;
                                    match submit_transaction_payload(&mut local_consensus_state, tx_payload.data) {
                                        Ok(tx_id) => println!("[Node:{}:Sequencer] Added payload to mempool via P2P, TxID: {}", args.node_id, tx_id),
                                        Err(e) => eprintln!("[Node:{}:Sequencer] Error submitting P2P payload: {}", args.node_id, e),
                                    }
                                    drop(local_consensus_state);
                                }
                                Err(e) => eprintln!("[Node:{}:Sequencer] Error deserializing P2P tx payload: {}", args.node_id, e),
                             }
                        } else {
                            println!("[Node:{}:Follower] Received transaction, but not sequencer. Forwarding to known sequencer (mock).", args.node_id);
                            // A follower might try to forward to a known sequencer.
                            // For now, just log. It could also try to send_direct to SEQUENCER_ID if its address is known.
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
