use ecliptic_concordance::{
    ConsensusState as NodeLocalConsensusState, 
    sequencer_create_block, submit_transaction_payload, validate_and_apply_block,
    Block, TransactionPayload, 
};
use triad_web::{
    NetworkMessage, AppP2PEvent, P2PService, 
    initialize_p2p_service, message_summary 
};
use triad_web::network_behaviour::AuroraTopic; 

use serde::{Deserialize, Serialize}; 
use serde_json;
use clap::Parser;
use std::fs::{File, OpenOptions}; 
use std::io::{self, BufRead, Write, BufReader as StdBufReader}; 
use std::path::{Path, PathBuf}; 
use std::time::Duration; 
use std::sync::Arc; 
use std::collections::HashMap; 

use tokio::sync::Mutex as TokioMutex; 
use tokio::net::TcpListener; 
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader}; 

use libp2p::{Multiaddr, PeerId}; 
use log::{info, warn, error, debug};


const SEQUENCER_ID_PREFIX: &str = "sequencer-"; 
const MAX_BLOCKS_PER_BATCH_RESPONSE: u32 = 20; 
const MAX_BLOCKS_TO_REQUEST_IN_SYNC: u32 = 50; 

#[derive(Debug, Clone, PartialEq, Eq)] 
enum NodeSyncState {
    Synced,
    AttemptingSync { 
        target_peer: Option<PeerId>, 
        target_height: u64,          
        next_expected_batch_start_height: u64,
    }, 
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args { 
    #[clap(long, help = "Unique Node ID (for logging, libp2p generates its own PeerId)")]
    node_name: String, 
    #[clap(long, value_delimiter = ',', help = "Comma-separated listen multiaddresses, e.g., /ip4/0.0.0.0/tcp/8001")]
    listen_addrs: String,
    #[clap(long, value_delimiter = ',', help = "Comma-separated bootstrap peer multiaddresses")]
    bootstrap_peers: Option<String>,
    #[clap(long, help = "Path to the data directory for this node (used for blockchain and node key)")]
    data_dir: PathBuf,
}

fn load_consensus_state_from_disk(_node_name: &str, blockchain_file_path: &Path) -> NodeLocalConsensusState { // Added _ to node_name
    let mut state = NodeLocalConsensusState::new(_node_name.to_string()); 
    if blockchain_file_path.exists() {
        if let Ok(file) = File::open(blockchain_file_path) {
            let reader = StdBufReader::new(file); 
            for line in reader.lines() {
                if let Ok(line_content) = line {
                    if !line_content.trim().is_empty() {
                        if let Ok(block) = serde_json::from_str::<Block>(&line_content) {
                            // More robust loading: only advance if blocks are sequential
                            if block.height == state.current_height + 1 && block.prev_block_hash == state.last_block_hash {
                                state.current_height = block.height;
                                state.last_block_hash = block.block_hash.clone();
                            } else if block.height == 0 && state.current_height == 0 && state.last_block_hash == "GENESIS_HASH_0.0.1" { // Allow first block if genesis
                                state.current_height = block.height;
                                state.last_block_hash = block.block_hash.clone();
                            }
                            else if block.height > state.current_height { // Jump if there's a gap, assuming file is mostly ordered
                                info!("[Node:{}] Blockchain file load jumped from H:{} to H:{}", _node_name, state.current_height, block.height);
                                state.current_height = block.height;
                                state.last_block_hash = block.block_hash.clone();
                            }
                        }
                    }
                }
            }
            info!("[Node:{}] Restored consensus: Height {}, LastHash {}", _node_name, state.current_height, state.last_block_hash);
        }
    }
    state
}

fn read_blocks_from_file(
    blockchain_file_path: &Path, 
    start_height: u64, 
    max_count: u32
) -> Result<Vec<Block>, io::Error> {
    let mut blocks = Vec::new();
    if !blockchain_file_path.exists() {
        return Ok(blocks);
    }
    let file = File::open(blockchain_file_path)?;
    let reader = StdBufReader::new(file);
    let mut count = 0;

    for line_result in reader.lines() {
        let line = line_result?;
        if line.trim().is_empty() { continue; }
        match serde_json::from_str::<Block>(&line) {
            Ok(block) => {
                if block.height >= start_height {
                    blocks.push(block);
                    count += 1;
                    if count >= max_count {
                        break;
                    }
                }
            }
            Err(e) => {
                error!("Failed to parse block from chain file: {}", e);
            }
        }
    }
    Ok(blocks)
}


#[derive(Debug, Serialize, Deserialize)] 
struct NodeStateSummary { 
    node_id: String, 
    name: String, 
    height: u64,
    last_block_hash: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct RpcRequest {
    id: String, 
    method: String,
    params: serde_json::Value, 
}

#[derive(Serialize, Deserialize, Debug)]
struct RpcResponse {
    id: String,
    result: Option<serde_json::Value>,
    error: Option<RpcError>,
}

#[derive(Serialize, Deserialize, Debug)]
struct RpcError {
    code: i32,
    message: String,
}


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init(); 
    
    let args = Args::parse(); 
    let main_loop_node_name = args.node_name.clone(); 

    if !args.data_dir.exists() { std::fs::create_dir_all(&args.data_dir)?; }
    let blockchain_file_path = args.data_dir.join(format!("{}_blockchain.jsonl", args.node_name));
    
    let is_sequencer = args.node_name.starts_with(SEQUENCER_ID_PREFIX); 
    info!("[Node:{}] Starting. Sequencer: {}. ChainFile: {:?}. DataDir: {:?}", 
        args.node_name, is_sequencer, blockchain_file_path, args.data_dir);

    let rpc_port_str = args.listen_addrs
        .split(',')
        .next() 
        .and_then(|addr_str| addr_str.split('/').nth(4)) 
        .unwrap_or("0"); 
    let rpc_port = rpc_port_str.parse::<u16>().unwrap_or(0) + 1000;
    let rpc_listen_addr_str = format!("127.0.0.1:{}", rpc_port);
    info!("[Node:{}] RPC server will listen on: {}", args.node_name, rpc_listen_addr_str);

    let consensus_state_arc = Arc::new(TokioMutex::new( 
        load_consensus_state_from_disk(&args.node_name, &blockchain_file_path) 
    ));
    
    let current_sync_state_arc = Arc::new(TokioMutex::new(NodeSyncState::Synced));

    let listen_multiaddrs: Vec<Multiaddr> = args.listen_addrs.split(',') 
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if listen_multiaddrs.is_empty() {
        error!("[Node:{}] No valid listen multiaddresses provided. Exiting.", args.node_name);
        return Err("No listen addresses".into());
    }

    let bootstrap_peers: Vec<Multiaddr> = args.bootstrap_peers.map_or_else(Vec::new, |peers_str| {
        peers_str.split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect()
    });
    
    let (p2p_service, mut app_event_rx) = initialize_p2p_service( 
        args.node_name.clone(), 
        args.data_dir.clone(), 
        listen_multiaddrs, 
        bootstrap_peers
    ).await?;
    let local_peer_id = p2p_service.get_peer_id(); 
    info!("[Node:{}] P2P service initialized. Local PeerId: {}", args.node_name, local_peer_id);

    let rpc_consensus_state_arc = consensus_state_arc.clone();
    let rpc_p2p_service = p2p_service.clone();
    let rpc_node_name_for_server_task = args.node_name.clone(); 
    let rpc_local_peer_id_for_server_task = local_peer_id.clone(); 

    tokio::spawn(async move { 
        let listener = match TcpListener::bind(&rpc_listen_addr_str).await { 
            Ok(l) => l,
            Err(e) => {
                error!("[Node:{}:RPC] Failed to bind RPC listener on {}: {}", rpc_node_name_for_server_task, rpc_listen_addr_str, e);
                return;
            }
        };
        info!("[Node:{}:RPC] Listening for RPC commands on {}", rpc_node_name_for_server_task, rpc_listen_addr_str);

        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    let rpc_handler_consensus_state = rpc_consensus_state_arc.clone();
                    let rpc_handler_p2p_service = rpc_p2p_service.clone();
                    let rpc_handler_node_name_conn = rpc_node_name_for_server_task.clone(); 
                    let rpc_handler_local_peer_id_conn = rpc_local_peer_id_for_server_task.clone();

                    debug!("[Node:{}:RPC] Accepted RPC connection from: {}", rpc_handler_node_name_conn, addr);
                    tokio::spawn(async move { 
                        handle_rpc_connection(
                            stream, 
                            rpc_handler_consensus_state, 
                            rpc_handler_p2p_service,
                            rpc_handler_node_name_conn, 
                            rpc_handler_local_peer_id_conn, 
                        ).await;
                    });
                }
                Err(e) => error!("[Node:{}:RPC] Error accepting RPC connection: {}", rpc_node_name_for_server_task, e),
            }
        }
    });

    if is_sequencer {
        let seq_node_name = args.node_name.clone();
        let p2p_publish_clone = p2p_service.clone(); 
        let blockchain_file_path_clone = blockchain_file_path.clone();
        let consensus_state_sequencer_arc = consensus_state_arc.clone();

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await; 
                let mut local_consensus_state = consensus_state_sequencer_arc.lock().await;
                match sequencer_create_block(&mut local_consensus_state, &seq_node_name) { 
                    Ok(new_block) => {
                        info!("[Node:{}:Seq] Created Block H:{}. Publishing...", seq_node_name, new_block.height);
                        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&blockchain_file_path_clone) { 
                            if writeln!(file, "{}", serde_json::to_string(&new_block).unwrap()).is_err() { /* log error */ }
                        }
                        match bincode::serialize(&new_block) {
                            Ok(serialized_block) => {
                                if let Err(e) = p2p_publish_clone.publish(AuroraTopic::Blocks, NetworkMessage::BlockProposal(serialized_block)).await { 
                                    warn!("[Node:{}:Seq] Failed to publish block: {}", seq_node_name, e);
                                }
                            }
                            Err(e) => error!("[Node:{}:Seq] Error serializing block with bincode: {}", seq_node_name, e),
                        }
                    }
                    Err(e) if e == "No pending transactions to create a block." => { }
                    Err(e) => error!("[Node:{}:Seq] Error creating block: {}", seq_node_name, e),
                }
                drop(local_consensus_state);
            }
        });
    }

    info!("[Node:{}] Listening for P2P application events...", main_loop_node_name);
    let mut _buffered_future_blocks: HashMap<u64, Block> = HashMap::new(); // Renamed with _ as it's not fully used yet

    loop {
        let node_name_for_loop_logging = main_loop_node_name.clone();
        let p2p_service_for_spawn = p2p_service.clone();
        // let current_sync_state_clone = current_sync_state_arc.clone(); // This clone isn't used
        let blockchain_file_path_for_handler = blockchain_file_path.clone();
        
        tokio::select! {
            Some(app_event) = app_event_rx.recv() => { 
                // Lock sync state at the beginning of event processing
                let mut current_sync_state_guard = current_sync_state_arc.lock().await;

                match app_event {
                    AppP2PEvent::GossipsubMessage { source, topic_hash, message } => {
                        debug!("[Node:{}] Received Gossip from PeerId {:?}, Topic {:?}: {}", node_name_for_loop_logging, source, topic_hash.to_string(), message_summary(&message));
                        match message {
                            NetworkMessage::BlockProposal(serialized_block) => {
                                match bincode::deserialize::<Block>(&serialized_block) { 
                                    Ok(block) => {
                                        if block.proposer_id == node_name_for_loop_logging { drop(current_sync_state_guard); continue; } 
                                        
                                        let mut local_consensus_state_lock = consensus_state_arc.lock().await; // Renamed lock
                                        if block.height > local_consensus_state_lock.current_height + 1 {
                                            warn!("[Node:{}] Received advanced block H:{} from {:?} (current H:{}). Buffering & initiating sync if not already.", 
                                                node_name_for_loop_logging, block.height, source, local_consensus_state_lock.current_height);
                                            _buffered_future_blocks.insert(block.height, block.clone());

                                            let mut needs_to_request_sync = false;
                                            if *current_sync_state_guard == NodeSyncState::Synced {
                                                needs_to_request_sync = true;
                                            } else if let NodeSyncState::AttemptingSync { target_height, .. } = *current_sync_state_guard {
                                                if block.height > target_height { // A new, even further peer appeared
                                                    needs_to_request_sync = true;
                                                }
                                            }
                                            
                                            if needs_to_request_sync {
                                                info!("[Node:{}] Entering/Updating sync mode. Target height from peer: {}", node_name_for_loop_logging, block.height);
                                                let next_expected = local_consensus_state_lock.current_height + 1;
                                                *current_sync_state_guard = NodeSyncState::AttemptingSync {
                                                    target_peer: Some(source),
                                                    target_height: block.height, 
                                                    next_expected_batch_start_height: next_expected,
                                                };
                                                
                                                let request_start_height = next_expected;
                                                let request_end_height = block.height.saturating_sub(1); 
                                                
                                                if request_start_height <= request_end_height {
                                                    let sync_req_msg = NetworkMessage::BlockRequestRange {
                                                        start_height: request_start_height,
                                                        end_height: Some(request_end_height),
                                                        max_blocks_to_send: Some(MAX_BLOCKS_TO_REQUEST_IN_SYNC),
                                                        requesting_peer_id: local_peer_id.to_string(),
                                                    };
                                                    info!("[Node:{}] Sending BlockRequestRange (start:{}, end:{:?}) to peer {:?}", 
                                                          node_name_for_loop_logging, request_start_height, Some(request_end_height), source);

                                                    let p2p_clone = p2p_service_for_spawn.clone();
                                                    let node_name_clone_for_spawn = node_name_for_loop_logging.clone();
                                                    tokio::spawn(async move {
                                                        if let Err(e) = p2p_clone.publish(AuroraTopic::Consensus, sync_req_msg).await {
                                                            error!("[Node:{}] Failed to publish BlockRequestRange: {}", node_name_clone_for_spawn, e);
                                                        }
                                                    });
                                                }
                                            }
                                            drop(local_consensus_state_lock);
                                            // current_sync_state_guard is still held if mutated, or dropped if not
                                            // If needs_to_request_sync was false, guard is still held.
                                            // If it was true, guard was mutated.
                                            // This continue is fine as we don't process this block now.
                                            drop(current_sync_state_guard); // Explicitly drop before continue
                                            continue; 
                                        }

                                        if *current_sync_state_guard != NodeSyncState::Synced && block.height != local_consensus_state_lock.current_height + 1 {
                                             debug!("[Node:{}] In sync mode, ignoring gossiped block H:{} not directly part of sync batch.", node_name_for_loop_logging, block.height);
                                             drop(local_consensus_state_lock);
                                             drop(current_sync_state_guard);
                                             continue;
                                        }

                                        match validate_and_apply_block(&mut local_consensus_state_lock, &block) { 
                                            Ok(()) => {
                                                info!("[Node:{}] Validated Block H:{} from {} (PeerId: {:?})", node_name_for_loop_logging, block.height, block.proposer_id, source);
                                                if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&blockchain_file_path){
                                                    if writeln!(file, "{}", serde_json::to_string(&block).unwrap()).is_err(){/* log */}
                                                }
                                                
                                                // Check if sync target reached after applying a regular gossiped block
                                                let current_height_after_apply = local_consensus_state_lock.current_height;
                                                let mut became_synced_this_block = false;
                                                if let NodeSyncState::AttemptingSync { target_height, .. } = *current_sync_state_guard {
                                                    if current_height_after_apply >= target_height {
                                                        info!("[Node:{}] Sync complete (via gossiped block). Reached/passed target height {}. Now Synced.", node_name_for_loop_logging, target_height);
                                                        *current_sync_state_guard = NodeSyncState::Synced;
                                                        became_synced_this_block = true;
                                                    }
                                                }
                                                // If became synced, try to process buffered blocks
                                                if became_synced_this_block {
                                                    let temp_current_h = current_height_after_apply;
                                                    // This loop needs to release and re-acquire consensus_state_lock or pass it
                                                    // For simplicity, we'll just log intent for now or do it less efficiently.
                                                    // This part is complex with async mutexes.
                                                    debug!("[Node:{}] Became synced. TODO: Process buffered blocks if any.", node_name_for_loop_logging);
                                                }
                                            }
                                            Err(e) => warn!("[Node:{}] Invalid block (H:{} from PeerId {:?}): {}", node_name_for_loop_logging, block.height, source, e),
                                        }
                                        drop(local_consensus_state_lock);
                                    }
                                    Err(e) => error!("[Node:{}] Error deserializing block from PeerId {:?}: {}", node_name_for_loop_logging, source, e),
                                }
                            }
                            NetworkMessage::Transaction(serialized_tx_payload_data) => {
                                match serde_json::from_slice::<TransactionPayload>(&serialized_tx_payload_data) { 
                                    Ok(tx_payload) => {
                                        let mut local_consensus_state_lock = consensus_state_arc.lock().await; // Renamed
                                        match submit_transaction_payload(&mut local_consensus_state_lock, tx_payload.data) { 
                                            Ok(tx_id) => info!("[Node:{}] Mempooled TxID: {} from Gossip (PeerId {:?})", node_name_for_loop_logging, tx_id, source),
                                            Err(e) => error!("[Node:{}] Error submitting Gossip payload from PeerId {:?}: {}", node_name_for_loop_logging, source, e),
                                        }
                                        drop(local_consensus_state_lock); // Drop lock
                                    }
                                    Err(e) => error!("[Node:{}] Error deserializing Gossip tx payload from JSON from PeerId {:?}: {}", node_name_for_loop_logging, source, e),
                                }
                            }
                            NetworkMessage::BlockRequestRange { start_height, end_height, max_blocks_to_send, requesting_peer_id } => {
                                info!("[Node:{}] Received BlockRequestRange from PeerId {} (Source: {:?}) for height {} to {:?}", 
                                    node_name_for_loop_logging, requesting_peer_id, source, start_height, end_height);
                                
                                let max_to_send = max_blocks_to_send.unwrap_or(MAX_BLOCKS_PER_BATCH_RESPONSE).min(MAX_BLOCKS_PER_BATCH_RESPONSE); 
                                
                                match read_blocks_from_file(&blockchain_file_path_for_handler, start_height, max_to_send) {
                                    Ok(found_blocks) => {
                                        if found_blocks.is_empty() {
                                            warn!("[Node:{}] No blocks found in range for PeerId {}. Start: {}, Max: {}", 
                                                node_name_for_loop_logging, requesting_peer_id, start_height, max_to_send);
                                            let no_blocks_msg = NetworkMessage::NoBlocksInRange { 
                                                requested_start: start_height, 
                                                requested_end: end_height,
                                                responder_peer_id: local_peer_id.to_string(),
                                            };
                                            let p2p_clone = p2p_service_for_spawn.clone();
                                            let node_name_clone_for_spawn = node_name_for_loop_logging.clone();
                                            tokio::spawn(async move {
                                                if let Err(e) = p2p_clone.publish(AuroraTopic::Consensus, no_blocks_msg).await { // Still gossiping
                                                    error!("[Node:{}] Failed to publish NoBlocksInRange: {}", node_name_clone_for_spawn, e);
                                                }
                                            });
                                        } else {
                                            let response_from_height = found_blocks.first().map_or(0, |b| b.height);
                                            let response_to_height = found_blocks.last().map_or(0, |b| b.height);
                                            info!("[Node:{}] Sending BlockResponseBatch ({} blocks, H:{} to H:{}) to PeerId {} (Source: {:?})", 
                                                node_name_for_loop_logging, found_blocks.len(), response_from_height, response_to_height, requesting_peer_id, source);
                                            
                                            let blocks_data: Vec<Vec<u8>> = found_blocks.into_iter()
                                                .filter_map(|b| bincode::serialize(&b).ok())
                                                .collect();

                                            let response_msg = NetworkMessage::BlockResponseBatch { blocks_data, from_height: response_from_height, to_height: response_to_height };
                                            let p2p_clone = p2p_service_for_spawn.clone();
                                            let node_name_clone_for_spawn = node_name_for_loop_logging.clone();
                                            tokio::spawn(async move {
                                                 if let Err(e) = p2p_clone.publish(AuroraTopic::Consensus, response_msg).await { 
                                                    error!("[Node:{}] Failed to publish BlockResponseBatch: {}", node_name_clone_for_spawn, e);
                                                }
                                            });
                                        }
                                    }
                                    Err(e) => {
                                        error!("[Node:{}] Error reading blocks from file for request from {}: {}", node_name_for_loop_logging, requesting_peer_id, e);
                                    }
                                }
                            }
                            NetworkMessage::BlockResponseBatch { blocks_data, from_height, to_height } => {
                                info!("[Node:{}] Received BlockResponseBatch from {:?} ({} blocks, H:{} to H:{})", 
                                    node_name_for_loop_logging, source, blocks_data.len(), from_height, to_height);

                                // Extract sync state info BEFORE locking consensus_state
                                let (current_target_peer, current_target_height, next_expected_start) = 
                                    if let NodeSyncState::AttemptingSync { target_peer, target_height, next_expected_batch_start_height } = *current_sync_state_guard {
                                        (target_peer, target_height, next_expected_batch_start_height)
                                    } else {
                                        debug!("[Node:{}] Received BlockResponseBatch but not in sync mode. Discarding.", node_name_for_loop_logging);
                                        drop(current_sync_state_guard); // Release sync state lock
                                        continue;
                                    };
                                
                                if from_height != next_expected_start {
                                    warn!("[Node:{}:Sync] Received batch starting at H:{} but expected H:{}. Discarding.", 
                                        node_name_for_loop_logging, from_height, next_expected_start);
                                    drop(current_sync_state_guard);
                                    continue;
                                }

                                let mut local_consensus_state_lock = consensus_state_arc.lock().await; // Renamed
                                let mut all_applied_successfully = true;
                                let mut last_applied_height_in_batch = local_consensus_state_lock.current_height;

                                for block_bytes in blocks_data {
                                    match bincode::deserialize::<Block>(&block_bytes) {
                                        Ok(block_to_apply) => {
                                            if block_to_apply.height == local_consensus_state_lock.current_height + 1 {
                                                if validate_and_apply_block(&mut local_consensus_state_lock, &block_to_apply).is_ok() {
                                                    info!("[Node:{}:Sync] Applied synced block H:{}", node_name_for_loop_logging, block_to_apply.height);
                                                    last_applied_height_in_batch = block_to_apply.height;
                                                     if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&blockchain_file_path){
                                                        let _ = writeln!(file, "{}", serde_json::to_string(&block_to_apply).unwrap());
                                                    }
                                                } else {
                                                    warn!("[Node:{}:Sync] Failed to validate/apply synced block H:{}", node_name_for_loop_logging, block_to_apply.height);
                                                    all_applied_successfully = false;
                                                    break;
                                                }
                                            } else {
                                                warn!("[Node:{}:Sync] Received out-of-order block H:{} in batch (expected H:{}). Stopping batch.", 
                                                    node_name_for_loop_logging, block_to_apply.height, local_consensus_state_lock.current_height + 1);
                                                all_applied_successfully = false; 
                                                break;
                                            }
                                        }
                                        Err(e) => {
                                            error!("[Node:{}:Sync] Failed to deserialize block in batch: {}", node_name_for_loop_logging, e);
                                            all_applied_successfully = false;
                                            break;
                                        }
                                    }
                                }
                                
                                // After processing batch, decide next step
                                // current_target_height and current_target_peer are from the initial state read
                                if local_consensus_state_lock.current_height >= current_target_height {
                                    info!("[Node:{}] Sync complete. Reached/passed target height {}. Now Synced.", node_name_for_loop_logging, current_target_height);
                                    *current_sync_state_guard = NodeSyncState::Synced;
                                    // TODO: Process buffered_future_blocks after releasing locks
                                    debug!("[Node:{}] TODO: Process buffered future blocks if any.", node_name_for_loop_logging);
                                } else if all_applied_successfully && last_applied_height_in_batch == to_height { 
                                    info!("[Node:{}:Sync] Batch applied up to H:{}. Requesting next batch from H:{} to target H:{}", 
                                        node_name_for_loop_logging, last_applied_height_in_batch, last_applied_height_in_batch + 1, current_target_height);
                                    
                                    *current_sync_state_guard = NodeSyncState::AttemptingSync {
                                        target_peer: current_target_peer, // Keep original target peer
                                        target_height: current_target_height, // Keep original target height
                                        next_expected_batch_start_height: last_applied_height_in_batch + 1,
                                    };

                                    let sync_req_msg = NetworkMessage::BlockRequestRange {
                                        start_height: last_applied_height_in_batch + 1,
                                        end_height: Some(current_target_height), 
                                        max_blocks_to_send: Some(MAX_BLOCKS_TO_REQUEST_IN_SYNC),
                                        requesting_peer_id: local_peer_id.to_string(),
                                    };
                                    let p2p_clone = p2p_service_for_spawn.clone();
                                    let node_name_clone_for_spawn = node_name_for_loop_logging.clone();
                                    if let Some(sync_target_peer_id) = current_target_peer { 
                                        tokio::spawn(async move {
                                             if let Err(e) = p2p_clone.publish(AuroraTopic::Consensus, sync_req_msg).await {
                                                error!("[Node:{}:Sync] Failed to publish next BlockRequestRange: {}", node_name_clone_for_spawn, e);
                                            }
                                        });
                                    } else {
                                        warn!("[Node:{}:Sync] No target peer to request next batch from.", node_name_for_loop_logging);
                                        *current_sync_state_guard = NodeSyncState::Synced; 
                                    }
                                } else if !all_applied_successfully {
                                    warn!("[Node:{}:Sync] Batch application incomplete or failed. Resetting sync state.", node_name_for_loop_logging);
                                    *current_sync_state_guard = NodeSyncState::Synced;
                                }
                                drop(local_consensus_state_lock); // Release consensus lock
                            }
                            NetworkMessage::NoBlocksInRange { requested_start, requested_end, responder_peer_id } => {
                                if let NodeSyncState::AttemptingSync { target_peer: Some(sync_target_id), .. } = *current_sync_state_guard {
                                    if source == sync_target_id || responder_peer_id == source.to_string() { // Check if response is from our sync target
                                        warn!("[Node:{}:Sync] Peer {:?} (or {}) reported no blocks in range {} to {:?}. Resetting sync state.", 
                                            node_name_for_loop_logging, source, responder_peer_id, requested_start, requested_end);
                                        *current_sync_state_guard = NodeSyncState::Synced; 
                                    }
                                }
                            }
                            NetworkMessage::NodeStateQuery { responder_peer_id: query_responder_peer_id } => { 
                                let should_respond = query_responder_peer_id.as_ref().map_or(true, |id_str| id_str == &local_peer_id.to_string());
                                if should_respond {
                                    info!("[Node:{}] Received NodeStateQuery via Gossip from {}", node_name_for_loop_logging, source);
                                    let local_consensus_state_lock = consensus_state_arc.lock().await;
                                    let summary = NodeStateSummary { 
                                        node_id: local_peer_id.to_string(),
                                        name: node_name_for_loop_logging.clone(), 
                                        height: local_consensus_state_lock.current_height,
                                        last_block_hash: local_consensus_state_lock.last_block_hash.clone(),
                                    };
                                    drop(local_consensus_state_lock); 
                                    if let Ok(serialized_summary) = bincode::serialize(&summary) {
                                        let node_name_for_response = node_name_for_loop_logging.clone();
                                        let p2p_service_for_response = p2p_service_for_spawn.clone();
                                        let response_msg = NetworkMessage::NodeStateResponse(serialized_summary);
                                        
                                        tokio::spawn(async move { 
                                            if let Err(e) = p2p_service_for_response.publish(AuroraTopic::Consensus, response_msg).await { 
                                                warn!("[Node:{}] Error publishing state response: {}", node_name_for_response, e);
                                            }
                                        });
                                    }
                                }
                            }
                            _ => {
                                debug!("[Node:{}] Unhandled Gossipsub message type from {:?}.", node_name_for_loop_logging, source);
                            }
                        }
                    }
                    AppP2PEvent::DirectMessage { source, message } => {
                         debug!("[Node:{}] Received unhandled DirectMessage from {:?}: {:?}", node_name_for_loop_logging, source, message_summary(&message));
                    }
                    AppP2PEvent::PeerConnected(peer_id) => {
                        info!("[Node:{}] Peer connected: {}", node_name_for_loop_logging, peer_id);
                    }
                    AppP2PEvent::PeerDisconnected(peer_id) => {
                        info!("[Node:{}] Peer disconnected: {}", node_name_for_loop_logging, peer_id);
                        if let NodeSyncState::AttemptingSync { target_peer: Some(sync_target), .. } = &*current_sync_state_guard {
                            if *sync_target == peer_id {
                                warn!("[Node:{}:Sync] Sync target peer {:?} disconnected. Resetting sync state.", node_name_for_loop_logging, peer_id);
                                *current_sync_state_guard = NodeSyncState::Synced;
                            }
                        }
                    }
                }
                // Explicitly drop the guard after the match if it wasn't already dropped
                // This ensures it's released before the next select! iteration or await point.
                // However, in most branches above, it's either used and dropped, or the branch continues.
                // Adding it here defensively.
                drop(current_sync_state_guard); 
            }
            else => {
                error!("[Node:{}] P2P application event channel closed. Shutting down.", main_loop_node_name); 
                break;
            }
        }
    }
    Ok(())
}

async fn handle_rpc_connection(
    stream: tokio::net::TcpStream, 
    consensus_state_arc: Arc<TokioMutex<NodeLocalConsensusState>>, 
    p2p_service: Arc<impl P2PService>, 
    node_name_handler: String, 
    local_peer_id_handler: PeerId, 
) {
    let (raw_reader, mut writer) = stream.into_split(); // Use into_split for owned R/W halves
    let mut reader = TokioBufReader::new(raw_reader);

    let mut line = String::new();
    loop {
        let node_name_for_this_rpc_iter = node_name_handler.clone();
        let p2p_service_for_this_rpc_iter = p2p_service.clone();

        line.clear();
        match reader.read_line(&mut line).await { 
            Ok(0) => { 
                debug!("[Node:{}:RPC] Connection closed by client.", node_name_for_this_rpc_iter);
                break;
            }
            Ok(_) => {
                let request_json = line.trim();
                if request_json.is_empty() { continue; }

                debug!("[Node:{}:RPC] Received RPC request: {}", node_name_for_this_rpc_iter, request_json);
                let rpc_req: RpcRequest = match serde_json::from_str(request_json) { 
                    Ok(req) => req,
                    Err(e) => {
                        error!("[Node:{}:RPC] Failed to parse RPC request: {}. Raw: '{}'", node_name_for_this_rpc_iter, e, request_json);
                        let err_resp = RpcResponse { 
                            id: "unknown".to_string(),
                            result: None,
                            error: Some(RpcError { code: -32700, message: "Parse error".to_string() }),
                        };
                        if let Ok(resp_json) = serde_json::to_string(&err_resp) {
                            let _ = writer.write_all(format!("{}\n", resp_json).as_bytes()).await; 
                            let _ = writer.flush().await; 
                        }
                        continue; 
                    }
                };

                let response_id_clone = rpc_req.id.clone(); 

                let response: RpcResponse = match rpc_req.method.as_str() {
                    "submit_transaction" => {
                        match serde_json::from_value::<HashMap<String, String>>(rpc_req.params) {
                            Ok(params) => {
                                if let Some(data_str) = params.get("data") {
                                    let tx_data_bytes = data_str.as_bytes().to_vec();
                                    let concord_tx_payload = TransactionPayload {data: tx_data_bytes.clone()};
                                    
                                    let tx_id_result = { 
                                        let mut local_consensus_state = consensus_state_arc.lock().await;
                                        submit_transaction_payload(&mut local_consensus_state, tx_data_bytes)
                                    };

                                    match tx_id_result {
                                        Ok(tx_id) => {
                                            info!("[Node:{}:RPC] Transaction submitted via RPC. TxID: {}, Gossiping...", node_name_for_this_rpc_iter, tx_id);
                                            
                                            let p2p_service_for_gossip = p2p_service_for_this_rpc_iter.clone();
                                            let node_name_for_gossip = node_name_for_this_rpc_iter.clone();
                                            let tx_id_for_gossip = tx_id.clone(); 

                                            let network_tx_payload = serde_json::to_vec(&concord_tx_payload).expect("RPC: Failed to serialize TransactionPayload for network");
                                            let tx_gossip_msg = NetworkMessage::Transaction(network_tx_payload);
                                            
                                            tokio::spawn(async move { 
                                                if let Err(e) = p2p_service_for_gossip.publish(AuroraTopic::Transactions, tx_gossip_msg).await {
                                                    error!("[Node:{}:RPC] Failed to gossip transaction {} via P2P: {}", node_name_for_gossip, tx_id_for_gossip, e);
                                                } else {
                                                    info!("[Node:{}:RPC] Successfully gossiped transaction {}", node_name_for_gossip, tx_id_for_gossip);
                                                }
                                            });
                                            RpcResponse { id: response_id_clone, result: Some(serde_json::json!({"transaction_id": tx_id})), error: None }
                                        }
                                        Err(e) => RpcResponse { id: response_id_clone, result: None, error: Some(RpcError { code: -1, message: format!("Consensus submission error: {}", e) }) },
                                    }
                                } else {
                                    RpcResponse { id: response_id_clone, result: None, error: Some(RpcError { code: -32602, message: "Missing 'data' param".to_string() }) }
                                }
                            }
                            Err(e) => RpcResponse { id: response_id_clone, result: None, error: Some(RpcError { code: -32602, message: format!("Invalid params for submit_transaction: {}", e) }) }
                        }
                    }
                    "get_node_state" => {
                        let local_consensus_state_lock = consensus_state_arc.lock().await; 
                        let summary = NodeStateSummary {
                            node_id: local_peer_id_handler.to_string(), 
                            name: node_name_handler.clone(), 
                            height: local_consensus_state_lock.current_height,
                            last_block_hash: local_consensus_state_lock.last_block_hash.clone(),
                        };
                        drop(local_consensus_state_lock); 
                        RpcResponse { id: response_id_clone, result: Some(serde_json::to_value(summary).unwrap()), error: None }
                    }
                    _ => RpcResponse { id: response_id_clone, result: None, error: Some(RpcError { code: -32601, message: "Method not found".to_string() }) },
                };

                if let Ok(resp_json) = serde_json::to_string(&response) {
                    if writer.write_all(format!("{}\n", resp_json).as_bytes()).await.is_err() {
                        error!("[Node:{}:RPC] Failed to send RPC response.", node_name_for_this_rpc_iter);
                        break; 
                    }
                    if writer.flush().await.is_err() {
                        error!("[Node:{}:RPC] Failed to flush RPC response.", node_name_for_this_rpc_iter);
                        break;
                    }
                } else {
                    error!("[Node:{}:RPC] Failed to serialize RPC response.", node_name_for_this_rpc_iter);
                }
            }
            Err(e) => {
                if e.kind() != std::io::ErrorKind::UnexpectedEof { 
                    error!("[Node:{}:RPC] Error reading RPC line: {}", node_name_handler, e); 
                }
                break; 
            }
        }
    }
    debug!("[Node:{}:RPC] RPC connection handler finished for an address.", node_name_handler); 
}