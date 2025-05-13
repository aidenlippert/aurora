// nodes/graviton_edge_node/src/main.rs

use ecliptic_concordance::{
    ConsensusState as NodeLocalConsensusState,
    sequencer_create_block, submit_transaction_payload, validate_and_apply_block,
    Block, TransactionPayload,
};
use triad_web::{
    NetworkMessage, AppP2PEvent, P2PService,
    initialize_p2p_service, message_summary,
    network_behaviour::AuroraTopic,
};

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
use log::{info, warn, error, debug, trace};

const SEQUENCER_ID_PREFIX: &str = "sequencer-";
const MAX_BLOCKS_PER_BATCH_RESPONSE: u32 = 20;
const MAX_BLOCKS_TO_REQUEST_IN_SYNC: u32 = 50;
const SYNC_RETRY_TIMEOUT_SECS: u64 = 30;
const SYNC_STUCK_THRESHOLD_SECS: u64 = SYNC_RETRY_TIMEOUT_SECS * 2 / 3;

#[derive(Debug, Clone, PartialEq, Eq)]
enum NodeSyncState {
    Synced,
    AttemptingSync {
        target_peer: Option<PeerId>,
        highest_known_height: u64,
        next_expected_batch_start_height: u64,
        last_request_time: tokio::time::Instant,
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

fn load_consensus_state_from_disk(node_name: &str, blockchain_file_path: &Path) -> NodeLocalConsensusState {
    let mut state = NodeLocalConsensusState::new(node_name.to_string());
    if blockchain_file_path.exists() {
        if let Ok(file) = File::open(blockchain_file_path) {
            let reader = StdBufReader::new(file);
            let mut genesis_block_processed_from_file = false;

            for line in reader.lines() {
                if let Ok(line_content) = line {
                    if !line_content.trim().is_empty() {
                        if let Ok(block) = serde_json::from_str::<Block>(&line_content) {
                            if block.height == 0 && state.current_height == 0 && state.last_block_hash == "GENESIS_HASH_0.0.1" && !genesis_block_processed_from_file {
                                state.current_height = block.height;
                                state.last_block_hash = block.block_hash.clone();
                                genesis_block_processed_from_file = true;
                                trace!("[Node:{}] Loaded genesis block H:0 from file.", node_name);
                            } else if block.height == state.current_height + 1 && block.prev_block_hash == state.last_block_hash {
                                state.current_height = block.height;
                                state.last_block_hash = block.block_hash.clone();
                            } else if block.height > state.current_height + 1 {
                                warn!("[Node:{}] Discontinuity in blockchain file. Current H:{}, Block H:{}. Attempting to jump.",
                                       node_name, state.current_height, block.height);
                                state.current_height = block.height;
                                state.last_block_hash = block.block_hash.clone();
                            } else if block.height <= state.current_height && !(block.height == 0 && genesis_block_processed_from_file) {
                                trace!("[Node:{}] Found older or duplicate block H:{} in chain file during load, skipping.", node_name, block.height);
                            }
                        } else {
                             error!("[Node:{}] Failed to parse block from chain file line: '{}'", node_name, line_content);
                        }
                    }
                }
            }
            info!("[Node:{}] Restored consensus: Height {}, LastHash {}", node_name, state.current_height, state.last_block_hash);
        } else {
            warn!("[Node:{}] Could not open blockchain file {:?}, starting from genesis.", node_name, blockchain_file_path);
        }
    } else {
        info!("[Node:{}] No blockchain file found at {:?}, starting from genesis.", node_name, blockchain_file_path);
    }
    state
}


fn read_blocks_from_file(
    blockchain_file_path: &Path,
    start_height: u64,
    max_count: u32,
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
                error!("Failed to parse block from chain file during read_blocks_from_file: {}", e);
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
    sync_state: String,
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

async fn send_block_request_range_to_peer(
    p2p_service_clone: Arc<impl P2PService>,
    node_name_clone_for_spawn: String,
    local_peer_id_str: String,
    target_sync_peer: PeerId,
    start_h: u64,
    end_h: u64, // This is the highest_known_height from the peer for this sync cycle
) {
    let sync_req_msg = NetworkMessage::BlockRequestRange {
        start_height: start_h,
        end_height: Some(end_h),
        max_blocks_to_send: Some(MAX_BLOCKS_TO_REQUEST_IN_SYNC),
        requesting_peer_id: local_peer_id_str,
    };
    info!("[Node:{}] Sending BlockRequestRange (start_h:{}, end_h(target):{}) to peer {:?}",
          node_name_clone_for_spawn, start_h, end_h, target_sync_peer);

    if let Err(e) = p2p_service_clone.publish(AuroraTopic::Consensus, sync_req_msg).await {
        error!("[Node:{}] Failed to publish BlockRequestRange to peer {:?}: {}", node_name_clone_for_spawn, target_sync_peer, e);
    }
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
    let buffered_future_blocks_arc = Arc::new(TokioMutex::new(HashMap::<u64, Block>::new()));


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
    let rpc_current_sync_state_arc = current_sync_state_arc.clone();

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
                    let rpc_handler_sync_state = rpc_current_sync_state_arc.clone();

                    debug!("[Node:{}:RPC] Accepted RPC connection from: {}", rpc_handler_node_name_conn, addr);
                    tokio::spawn(async move {
                        handle_rpc_connection(
                            stream,
                            rpc_handler_consensus_state,
                            rpc_handler_p2p_service,
                            rpc_handler_node_name_conn,
                            rpc_handler_local_peer_id_conn,
                            rpc_handler_sync_state,
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
        let sync_state_sequencer_check = current_sync_state_arc.clone();

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;
                
                let is_synced = {
                    let sync_state = sync_state_sequencer_check.lock().await;
                    matches!(*sync_state, NodeSyncState::Synced)
                };

                if !is_synced {
                    trace!("[Node:{}:Seq] Sequencer is not synced, skipping block proposal.", seq_node_name);
                    continue;
                }

                let mut local_consensus_state = consensus_state_sequencer_arc.lock().await;
                match sequencer_create_block(&mut local_consensus_state, &seq_node_name) {
                    Ok(new_block) => {
                        info!("[Node:{}:Seq] Created Block H:{}. Publishing...", seq_node_name, new_block.height);
                        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&blockchain_file_path_clone) {
                            if writeln!(file, "{}", serde_json::to_string(&new_block).unwrap_or_default()).is_err() {
                                error!("[Node:{}:Seq] Failed to write block to file.", seq_node_name);
                            }
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
                    Err(e) if e == "No pending transactions to create a block." => { /* Normal, do nothing */ }
                    Err(e) => error!("[Node:{}:Seq] Error creating block: {}", seq_node_name, e),
                }
            }
        });
    }

    info!("[Node:{}] Listening for P2P application events...", main_loop_node_name);

    let periodic_sync_check_p2p_service = p2p_service.clone();
    let periodic_sync_check_node_name = main_loop_node_name.clone();
    let periodic_sync_check_local_peer_id_str = local_peer_id.to_string();
    let periodic_sync_check_consensus_state = consensus_state_arc.clone();
    let periodic_sync_check_sync_state = current_sync_state_arc.clone();
    let periodic_blockchain_file_path = blockchain_file_path.clone();
    let periodic_buffered_blocks_arc = buffered_future_blocks_arc.clone();

    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(SYNC_RETRY_TIMEOUT_SECS)).await;
            let mut sync_state_guard = periodic_sync_check_sync_state.lock().await;
            match *sync_state_guard {
                NodeSyncState::AttemptingSync { target_peer, highest_known_height, next_expected_batch_start_height, last_request_time } => {
                    if last_request_time.elapsed() > Duration::from_secs(SYNC_STUCK_THRESHOLD_SECS) {
                        warn!("[Node:{}:SyncCheck] Sync seems stuck (no response for >{}s). Current next_expected_batch_start_height: {}. Re-requesting or finding new peer.",
                               periodic_sync_check_node_name, SYNC_STUCK_THRESHOLD_SECS, next_expected_batch_start_height);
                        
                        if let Some(peer_to_retry_with) = target_peer {
                            let current_cs_height = periodic_sync_check_consensus_state.lock().await.current_height;
                            if next_expected_batch_start_height <= highest_known_height && next_expected_batch_start_height > current_cs_height { // Corrected logic here
                                // Drop guard before await, then re-acquire to update
                                drop(sync_state_guard); 
                                send_block_request_range_to_peer(
                                    periodic_sync_check_p2p_service.clone(),
                                    periodic_sync_check_node_name.clone(),
                                    periodic_sync_check_local_peer_id_str.clone(),
                                    peer_to_retry_with,
                                    next_expected_batch_start_height,
                                    highest_known_height
                                ).await;
                                let mut sync_state_guard_update = periodic_sync_check_sync_state.lock().await;
                                if let NodeSyncState::AttemptingSync { last_request_time: ref mut time_ref, .. } = *sync_state_guard_update {
                                    *time_ref = tokio::time::Instant::now();
                                }
                                // sync_state_guard_update is dropped here when it goes out of scope
                            } else {
                                 info!("[Node:{}:SyncCheck] Sync stuck but conditions for re-request not met (next_expected: {}, highest_known: {}, current: {}).",
                                       periodic_sync_check_node_name, next_expected_batch_start_height, highest_known_height, current_cs_height);
                                 if target_peer.is_none() || (highest_known_height <= current_cs_height && next_expected_batch_start_height > current_cs_height) {
                                     *sync_state_guard = NodeSyncState::Synced;
                                 }
                            }
                        } else {
                            warn!("[Node:{}:SyncCheck] Sync stuck, but no target_peer. Resetting to Synced to allow new peer discovery on next block event.", periodic_sync_check_node_name);
                            *sync_state_guard = NodeSyncState::Synced;
                        }
                    }
                }
                NodeSyncState::Synced => {
                    let mut buffered_map_guard = periodic_buffered_blocks_arc.lock().await;
                    if !buffered_map_guard.is_empty() {
                        let mut cs_lock = periodic_sync_check_consensus_state.lock().await;
                        let mut next_to_apply_height = cs_lock.current_height + 1;
                        let mut applied_from_buffer_count = 0;
                        while let Some(block) = buffered_map_guard.remove(&next_to_apply_height) {
                            if validate_and_apply_block(&mut cs_lock, &block).is_ok() {
                                info!("[Node:{}:SyncCheckBuffer] Applied buffered block H:{}", periodic_sync_check_node_name, block.height);
                                if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&periodic_blockchain_file_path){
                                    let _ = writeln!(file, "{}", serde_json::to_string(&block).unwrap_or_default());
                                }
                                next_to_apply_height = cs_lock.current_height + 1;
                                applied_from_buffer_count += 1;
                            } else {
                                warn!("[Node:{}:SyncCheckBuffer] Failed to apply buffered block H:{}. Re-inserting.", periodic_sync_check_node_name, block.height);
                                buffered_map_guard.insert(block.height, block);
                                break;
                            }
                        }
                        if applied_from_buffer_count > 0 {
                            info!("[Node:{}:SyncCheckBuffer] Applied {} blocks from buffer. New height: {}", periodic_sync_check_node_name, applied_from_buffer_count, cs_lock.current_height);
                        }
                    }
                }
            }
        }
    });


    loop {
        let node_name_for_loop_logging = main_loop_node_name.clone();
        let p2p_service_for_spawn = p2p_service.clone();
        let blockchain_file_path_for_handler = blockchain_file_path.clone();
        let local_peer_id_str_for_req = local_peer_id.to_string();
        let buffered_blocks_main_loop_clone = buffered_future_blocks_arc.clone();

        tokio::select! {
            Some(app_event) = app_event_rx.recv() => {
                let mut current_sync_state_guard = current_sync_state_arc.lock().await;

                match app_event {
                    AppP2PEvent::GossipsubMessage { source, topic_hash: _, message } => {
                        trace!("[Node:{}] Received Gossip from PeerId {:?}: {}", node_name_for_loop_logging, source, message_summary(&message));
                        match message {
                            NetworkMessage::BlockProposal(serialized_block) => {
                                match bincode::deserialize::<Block>(&serialized_block) {
                                    Ok(block) => {
                                        if block.proposer_id == node_name_for_loop_logging { continue; }

                                        let mut local_consensus_state_lock = consensus_state_arc.lock().await;
                                        let current_node_height = local_consensus_state_lock.current_height;

                                        if block.height > current_node_height + 1 {
                                            debug!("[Node:{}] Received advanced block H:{} from {:?} (current H:{}). Buffering.",
                                                node_name_for_loop_logging, block.height, source, current_node_height);
                                            buffered_blocks_main_loop_clone.lock().await.insert(block.height, block.clone());

                                            let mut should_initiate_new_sync_cycle = false;
                                            let mut new_target_peer_for_cycle = source; 
                                            let mut new_highest_known_for_cycle = block.height;

                                            match *current_sync_state_guard {
                                                NodeSyncState::Synced => {
                                                    info!("[Node:{}] Synced, but saw advanced block H:{}. Initiating sync.", node_name_for_loop_logging, block.height);
                                                    should_initiate_new_sync_cycle = true;
                                                }
                                                NodeSyncState::AttemptingSync { target_peer: Some(current_target_p), highest_known_height: current_highest, last_request_time, .. } => {
                                                    if block.height > current_highest {
                                                        if let NodeSyncState::AttemptingSync { ref mut highest_known_height, ..} = *current_sync_state_guard {
                                                            *highest_known_height = block.height; // Update target height
                                                            debug!("[Node:{}] Sync in progress with {:?}. Updated target_height to {} due to new block from {:?}.",
                                                                   node_name_for_loop_logging, current_target_p, block.height, source);
                                                        }
                                                        new_highest_known_for_cycle = block.height; // Use new block's height for potential new cycle
                                                    } else {
                                                        new_highest_known_for_cycle = current_highest; // Keep current highest if new block isn't higher
                                                    }

                                                    if source != current_target_p && block.height > current_highest {
                                                        info!("[Node:{}] Switching sync target from {:?} to {:?} due to significantly newer block H:{}.",
                                                              node_name_for_loop_logging, current_target_p, source, block.height);
                                                        should_initiate_new_sync_cycle = true;
                                                        // new_target_peer_for_cycle is already 'source'
                                                    } else if last_request_time.elapsed() > Duration::from_secs(SYNC_STUCK_THRESHOLD_SECS) {
                                                        info!("[Node:{}] Current sync target {:?} seems stuck (elapsed {:?}). Re-initiating sync cycle, possibly with same peer for new height {}.",
                                                              node_name_for_loop_logging, current_target_p, last_request_time.elapsed(), new_highest_known_for_cycle);
                                                        should_initiate_new_sync_cycle = true;
                                                        new_target_peer_for_cycle = current_target_p; // Try same peer first for the (potentially updated) height
                                                    } else {
                                                        trace!("[Node:{}] Already syncing with {:?}. New block from {:?} (H:{}) noted. Current highest target is {}.",
                                                               node_name_for_loop_logging, current_target_p, source, block.height, new_highest_known_for_cycle);
                                                    }
                                                }
                                                NodeSyncState::AttemptingSync { target_peer: None, .. } => { 
                                                    info!("[Node:{}] Was AttemptingSync but no target_peer. Initiating sync with {:?}.", node_name_for_loop_logging, source);
                                                    should_initiate_new_sync_cycle = true;
                                                }
                                            }

                                            if should_initiate_new_sync_cycle {
                                                let next_expected_sync_start = current_node_height + 1;
                                                if next_expected_sync_start <= new_highest_known_for_cycle { 
                                                    info!("[Node:{}] Setting new sync cycle: TargetPeer:{:?}, HighestKnown:{}, NextExpectedBatchStart:{}",
                                                        node_name_for_loop_logging, new_target_peer_for_cycle, new_highest_known_for_cycle, next_expected_sync_start);
                                                    *current_sync_state_guard = NodeSyncState::AttemptingSync {
                                                        target_peer: Some(new_target_peer_for_cycle),
                                                        highest_known_height: new_highest_known_for_cycle,
                                                        next_expected_batch_start_height: next_expected_sync_start,
                                                        last_request_time: tokio::time::Instant::now(),
                                                    };
                                                    drop(local_consensus_state_lock);

                                                    send_block_request_range_to_peer(
                                                        p2p_service_for_spawn.clone(),
                                                        node_name_for_loop_logging.clone(),
                                                        local_peer_id_str_for_req.clone(),
                                                        new_target_peer_for_cycle,
                                                        next_expected_sync_start,
                                                        new_highest_known_for_cycle
                                                    ).await;
                                                } else {
                                                    debug!("[Node:{}] Advanced block seen (H:{}), but no range to request (current H:{}, next_expected_sync_start:{}). Relying on gossip/buffer.",
                                                           node_name_for_loop_logging, block.height, current_node_height, next_expected_sync_start);
                                                }
                                            }
                                        } else if matches!(*current_sync_state_guard, NodeSyncState::Synced) && block.height == current_node_height + 1 {
                                            match validate_and_apply_block(&mut local_consensus_state_lock, &block) {
                                                Ok(()) => {
                                                    info!("[Node:{}] Applied gossiped Block H:{} from {} (PeerId: {:?})", node_name_for_loop_logging, block.height, block.proposer_id, source);
                                                    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&blockchain_file_path_for_handler){
                                                        if writeln!(file, "{}", serde_json::to_string(&block).unwrap_or_default()).is_err(){
                                                            error!("[Node:{}] Failed to write applied gossiped block H:{} to file", node_name_for_loop_logging, block.height);
                                                        }
                                                    }
                                                }
                                                Err(e) => warn!("[Node:{}] Invalid gossiped block (H:{} from PeerId {:?}): {}", node_name_for_loop_logging, block.height, source, e),
                                            }
                                        } else {
                                            trace!("[Node:{}] Ignoring gossiped block H:{} from {:?}. Current H:{}, SyncState: {:?}",
                                                   node_name_for_loop_logging, block.height, source, current_node_height, *current_sync_state_guard);
                                        }
                                    }
                                    Err(e) => error!("[Node:{}] Error deserializing block from PeerId {:?}: {}", node_name_for_loop_logging, source, e),
                                }
                            }
                            NetworkMessage::Transaction(serialized_tx_payload_data) => {
                                if !matches!(*current_sync_state_guard, NodeSyncState::Synced) {
                                    trace!("[Node:{}] Not synced, ignoring transaction from gossip.", node_name_for_loop_logging);
                                    continue;
                                }
                                match serde_json::from_slice::<TransactionPayload>(&serialized_tx_payload_data) {
                                    Ok(tx_payload) => {
                                        let mut local_consensus_state_lock = consensus_state_arc.lock().await;
                                        match submit_transaction_payload(&mut local_consensus_state_lock, tx_payload.data) {
                                            Ok(tx_id) => info!("[Node:{}] Mempooled TxID: {} from Gossip (PeerId {:?})", node_name_for_loop_logging, tx_id, source),
                                            Err(e) => error!("[Node:{}] Error submitting Gossip payload from PeerId {:?}: {}", node_name_for_loop_logging, source, e),
                                        }
                                    }
                                    Err(e) => error!("[Node:{}] Error deserializing Gossip tx payload from JSON from PeerId {:?}: {}", node_name_for_loop_logging, source, e),
                                }
                            }
                            NetworkMessage::BlockRequestRange { start_height, end_height, max_blocks_to_send, requesting_peer_id } => {
                                info!("[Node:{}] Received BlockRequestRange from PeerId {} (Actual Source: {:?}) for height {} to {:?}",
                                    node_name_for_loop_logging, requesting_peer_id, source, start_height, end_height);

                                let max_to_send_val = max_blocks_to_send.unwrap_or(MAX_BLOCKS_PER_BATCH_RESPONSE).min(MAX_BLOCKS_PER_BATCH_RESPONSE);
                                match read_blocks_from_file(&blockchain_file_path_for_handler, start_height, max_to_send_val) {
                                    Ok(found_blocks) => {
                                        let response_msg: NetworkMessage;
                                        if found_blocks.is_empty() {
                                            warn!("[Node:{}] No blocks found in range for {}. Start: {}, Max: {}",
                                                node_name_for_loop_logging, requesting_peer_id, start_height, max_to_send_val);
                                            response_msg = NetworkMessage::NoBlocksInRange {
                                                requested_start: start_height,
                                                requested_end: end_height,
                                                responder_peer_id: local_peer_id.to_string(),
                                            };
                                        } else {
                                            let response_from_height = found_blocks.first().map_or(0, |b| b.height);
                                            let response_to_height = found_blocks.last().map_or(0, |b| b.height);
                                            info!("[Node:{}] Sending BlockResponseBatch ({} blocks, H:{} to H:{}) to {} (Actual Source: {:?})",
                                                node_name_for_loop_logging, found_blocks.len(), response_from_height, response_to_height, requesting_peer_id, source);
                                            let blocks_data: Vec<Vec<u8>> = found_blocks.into_iter()
                                                .filter_map(|b| bincode::serialize(&b).ok())
                                                .collect();
                                            response_msg = NetworkMessage::BlockResponseBatch { blocks_data, from_height: response_from_height, to_height: response_to_height };
                                        }
                                        let p2p_clone = p2p_service_for_spawn.clone();
                                        let node_name_clone_for_spawn = node_name_for_loop_logging.clone();
                                        // Drop guard before await
                                        drop(current_sync_state_guard);
                                        tokio::spawn(async move {
                                            if let Err(e) = p2p_clone.publish(AuroraTopic::Consensus, response_msg).await {
                                                error!("[Node:{}] Failed to publish BlockResponse/NoBlocks: {}", node_name_clone_for_spawn, e);
                                            }
                                        });
                                        continue; // Avoid re-locking and dropping guard again
                                    }
                                    Err(e) => {
                                        error!("[Node:{}] Error reading blocks from file for request from {}: {}", node_name_for_loop_logging, requesting_peer_id, e);
                                    }
                                }
                            }
                            NetworkMessage::BlockResponseBatch { blocks_data, from_height, to_height } => {
                                info!("[Node:{}] Received BlockResponseBatch from {:?} ({} blocks, H:{} to H:{})",
                                    node_name_for_loop_logging, source, blocks_data.len(), from_height, to_height);

                                if let NodeSyncState::AttemptingSync { target_peer, highest_known_height, next_expected_batch_start_height, .. } = *current_sync_state_guard {
                                    if Some(source) != target_peer {
                                        debug!("[Node:{}] Received BlockResponseBatch from non-target peer {:?}. Ignoring.", node_name_for_loop_logging, source);
                                        continue;
                                    }
                                    
                                    let mut local_consensus_state_lock = consensus_state_arc.lock().await;

                                    if from_height < next_expected_batch_start_height && from_height == local_consensus_state_lock.current_height + 1 {
                                        warn!("[Node:{}:Sync] Received batch starting at H:{} (older than expected H:{}, but fits current H:{}). Attempting to apply this unexpected but useful batch.",
                                            node_name_for_loop_logging, from_height, next_expected_batch_start_height, local_consensus_state_lock.current_height);
                                    } else if from_height != next_expected_batch_start_height {
                                        warn!("[Node:{}:Sync] Received batch starting at H:{} but expected H:{}. Current node H:{}. Ignoring this batch and resetting sync.",
                                            node_name_for_loop_logging, from_height, next_expected_batch_start_height, local_consensus_state_lock.current_height);
                                        *current_sync_state_guard = NodeSyncState::Synced;
                                        continue;
                                    }

                                    let mut all_applied_successfully_in_batch = true;
                                    let mut last_applied_height_this_batch = local_consensus_state_lock.current_height;

                                    for block_bytes in blocks_data {
                                        match bincode::deserialize::<Block>(&block_bytes) {
                                            Ok(block_to_apply) => {
                                                if block_to_apply.height == local_consensus_state_lock.current_height + 1 {
                                                    if validate_and_apply_block(&mut local_consensus_state_lock, &block_to_apply).is_ok() {
                                                        trace!("[Node:{}:Sync] Applied synced block H:{}", node_name_for_loop_logging, block_to_apply.height);
                                                        last_applied_height_this_batch = block_to_apply.height;
                                                        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&blockchain_file_path_for_handler){
                                                            let _ = writeln!(file, "{}", serde_json::to_string(&block_to_apply).unwrap_or_default());
                                                        }
                                                    } else {
                                                        warn!("[Node:{}:Sync] Failed to validate/apply synced block H:{} from batch. Stopping batch application.", node_name_for_loop_logging, block_to_apply.height);
                                                        all_applied_successfully_in_batch = false;
                                                        break;
                                                    }
                                                } else {
                                                    warn!("[Node:{}:Sync] Received out-of-order block H:{} in batch (expected H:{}). Stopping batch.",
                                                        node_name_for_loop_logging, block_to_apply.height, local_consensus_state_lock.current_height + 1);
                                                    all_applied_successfully_in_batch = false;
                                                    break;
                                                }
                                            }
                                            Err(e) => {
                                                error!("[Node:{}:Sync] Failed to deserialize block in batch: {}. Stopping batch application.", node_name_for_loop_logging, e);
                                                all_applied_successfully_in_batch = false;
                                                break;
                                            }
                                        }
                                    }

                                    let current_height_after_batch = local_consensus_state_lock.current_height;
                                    drop(local_consensus_state_lock);


                                    if current_height_after_batch >= highest_known_height {
                                        info!("[Node:{}] Sync complete. Reached/passed target height {}. Now Synced.", node_name_for_loop_logging, highest_known_height);
                                        *current_sync_state_guard = NodeSyncState::Synced;
                                        
                                        let mut buffered_blocks_map_guard = buffered_blocks_main_loop_clone.lock().await;
                                        let mut next_buffered_to_apply_height = current_height_after_batch + 1;
                                        let mut applied_from_buffer_in_sync_finish = 0;
                                        while let Some(block) = buffered_blocks_map_guard.remove(&next_buffered_to_apply_height) {
                                            let mut cs_lock_for_buffer = consensus_state_arc.lock().await;
                                            if validate_and_apply_block(&mut cs_lock_for_buffer, &block).is_ok() {
                                                info!("[Node:{}:SyncFinishBuffer] Applied buffered block H:{}", node_name_for_loop_logging, block.height);
                                                if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&blockchain_file_path_for_handler){
                                                    let _ = writeln!(file, "{}", serde_json::to_string(&block).unwrap_or_default());
                                                }
                                                next_buffered_to_apply_height = cs_lock_for_buffer.current_height + 1;
                                                applied_from_buffer_in_sync_finish +=1;
                                            } else {
                                                warn!("[Node:{}:SyncFinishBuffer] Failed to apply buffered block H:{}. Re-inserting.", node_name_for_loop_logging, block.height);
                                                buffered_blocks_map_guard.insert(block.height, block);
                                                break;
                                            }
                                        }
                                        if applied_from_buffer_in_sync_finish > 0 {
                                            info!("[Node:{}:SyncFinishBuffer] Applied {} blocks from buffer.", node_name_for_loop_logging, applied_from_buffer_in_sync_finish);
                                        }
                                    } else if all_applied_successfully_in_batch && last_applied_height_this_batch == to_height {
                                        let next_req_start = last_applied_height_this_batch + 1;
                                        if next_req_start <= highest_known_height {
                                            info!("[Node:{}:Sync] Batch H:{} to H:{} applied. Requesting next from H:{}",
                                                  node_name_for_loop_logging, from_height, to_height, next_req_start);
                                            *current_sync_state_guard = NodeSyncState::AttemptingSync {
                                                target_peer,
                                                highest_known_height,
                                                next_expected_batch_start_height: next_req_start,
                                                last_request_time: tokio::time::Instant::now(),
                                            };
                                            if let Some(peer_to_request_from) = target_peer {
                                                // Drop guard before await
                                                drop(current_sync_state_guard);
                                                send_block_request_range_to_peer(
                                                    p2p_service_for_spawn.clone(),
                                                    node_name_for_loop_logging.clone(),
                                                    local_peer_id_str_for_req.clone(),
                                                    peer_to_request_from,
                                                    next_req_start,
                                                    highest_known_height
                                                ).await;
                                                continue; // Avoid dropping guard again
                                            } else {
                                                 *current_sync_state_guard = NodeSyncState::Synced;
                                            }
                                        } else {
                                             info!("[Node:{}] Applied batch up to H:{}, which meets or exceeds highest_known_height {}. Setting to Synced.", node_name_for_loop_logging, last_applied_height_this_batch, highest_known_height);
                                             *current_sync_state_guard = NodeSyncState::Synced;
                                        }
                                    } else {
                                        warn!("[Node:{}:Sync] Sync batch H:{} to H:{} from {:?} incomplete or failed. Resetting to Synced. Will retry on next advanced block.",
                                              node_name_for_loop_logging, from_height, to_height, source);
                                        *current_sync_state_guard = NodeSyncState::Synced;
                                    }
                                } else {
                                     debug!("[Node:{}] Received BlockResponseBatch but not in AttemptingSync state or from wrong peer. SyncState: {:?}, Source: {:?}",
                                           node_name_for_loop_logging, *current_sync_state_guard, source);
                                }
                            }
                            NetworkMessage::NoBlocksInRange { requested_start, requested_end, responder_peer_id } => {
                                if let NodeSyncState::AttemptingSync { target_peer: Some(sync_target_id), next_expected_batch_start_height, .. } = *current_sync_state_guard {
                                    if source == sync_target_id && requested_start == next_expected_batch_start_height {
                                        warn!("[Node:{}:Sync] Sync target peer {:?} (reported by {}) reported no blocks for range starting at {} to {:?}. Resetting sync state.",
                                            node_name_for_loop_logging, source, responder_peer_id, requested_start, requested_end);
                                        *current_sync_state_guard = NodeSyncState::Synced;
                                    } else {
                                        debug!("[Node:{}] Received NoBlocksInRange from {:?} but not relevant to current sync op. Ignoring.", node_name_for_loop_logging, source);
                                    }
                                }
                            }
                            _ => {
                                trace!("[Node:{}] Unhandled Gossipsub message type from {:?}.", node_name_for_loop_logging, source);
                            }
                        }
                    }
                    AppP2PEvent::DirectMessage { source, message } => {
                         trace!("[Node:{}] Received unhandled DirectMessage from {:?}: {:?}", node_name_for_loop_logging, source, message_summary(&message));
                    }
                    AppP2PEvent::PeerConnected(peer_id) => {
                        info!("[Node:{}] Peer connected: {}", node_name_for_loop_logging, peer_id);
                    }
                    AppP2PEvent::PeerDisconnected(peer_id) => {
                        info!("[Node:{}] Peer disconnected: {}", node_name_for_loop_logging, peer_id);
                        if let NodeSyncState::AttemptingSync { target_peer: Some(sync_target_id), .. } = *current_sync_state_guard {
                            if sync_target_id == peer_id {
                                warn!("[Node:{}:Sync] Sync target peer {:?} disconnected. Resetting sync state. Will find new peer on next trigger.",
                                      node_name_for_loop_logging, peer_id);
                                *current_sync_state_guard = NodeSyncState::Synced;
                            }
                        }
                    }
                }
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
    sync_state_arc: Arc<TokioMutex<NodeSyncState>>,
) {
    let (raw_reader, mut writer) = stream.into_split();
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
                        let current_sync_state_val = sync_state_arc.lock().await;
                        if !matches!(*current_sync_state_val, NodeSyncState::Synced) {
                            warn!("[Node:{}:RPC] Node not synced, rejecting submit_transaction. SyncState: {:?}", node_name_for_this_rpc_iter, *current_sync_state_val);
                             RpcResponse { id: response_id_clone, result: None, error: Some(RpcError { code: -100, message: format!("Node is not synced. Current state: {:?}", *current_sync_state_val) }) }
                        } else {
                            drop(current_sync_state_val);
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
                                                
                                                let mut gossip_status = "gossip_queued".to_string();
                                                match serde_json::to_vec(&concord_tx_payload) {
                                                    Ok(network_tx_payload_bytes) => {
                                                        let tx_gossip_msg = NetworkMessage::Transaction(network_tx_payload_bytes);
                                                        let p2p_service_for_gossip = p2p_service_for_this_rpc_iter.clone();
                                                        let node_name_for_gossip = node_name_for_this_rpc_iter.clone();
                                                        let tx_id_for_gossip = tx_id.clone();
                                                        
                                                        tokio::spawn(async move {
                                                            if let Err(e) = p2p_service_for_gossip.publish(AuroraTopic::Transactions, tx_gossip_msg).await {
                                                                error!("[Node:{}:RPC:GossipSpawn] Failed to gossip transaction {} via P2P: {}", node_name_for_gossip, tx_id_for_gossip, e);
                                                            } else {
                                                                info!("[Node:{}:RPC:GossipSpawn] Successfully gossiped transaction {}", node_name_for_gossip, tx_id_for_gossip);
                                                            }
                                                        });
                                                    }
                                                    Err(e) => {
                                                        error!("[Node:{}:RPC] Failed to serialize TransactionPayload for network, cannot gossip: {}", node_name_for_this_rpc_iter, e);
                                                        gossip_status = "gossip_serialization_failed".to_string();
                                                    }
                                                };
                                                RpcResponse {
                                                    id: response_id_clone,
                                                    result: Some(serde_json::json!({
                                                        "transaction_id": tx_id,
                                                        "gossip_status": gossip_status
                                                    })),
                                                    error: None
                                                }
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
                    }
                    "get_node_state" => {
                        let local_consensus_state_lock = consensus_state_arc.lock().await;
                        let sync_state_lock = sync_state_arc.lock().await;
                        let summary = NodeStateSummary {
                            node_id: local_peer_id_handler.to_string(),
                            name: node_name_handler.clone(),
                            height: local_consensus_state_lock.current_height,
                            last_block_hash: local_consensus_state_lock.last_block_hash.clone(),
                            sync_state: format!("{:?}", *sync_state_lock),
                        };
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