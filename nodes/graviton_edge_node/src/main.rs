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
use std::io::{self, BufRead, Write}; 
use std::path::{Path, PathBuf}; 
use std::time::Duration; 
use std::sync::Arc; 
use std::collections::HashMap; 

use tokio::sync::Mutex as TokioMutex; 
use tokio::net::TcpListener; 
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader}; 

use libp2p::{Multiaddr, PeerId}; 
use log::{info, warn, error, debug}; // Removed trace, Uuid as they are not used in this final version

// Removed: use uuid::Uuid; // Not directly used in this file anymore for RPC IDs, CLI handles it.


const SEQUENCER_ID_PREFIX: &str = "sequencer-"; 

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
            let reader = io::BufReader::new(file); 
            if let Some(Ok(last_line)) = reader.lines().last() {
                if !last_line.trim().is_empty() {
                    if let Ok(last_block) = serde_json::from_str::<Block>(&last_line) {
                        state.current_height = last_block.height;
                        state.last_block_hash = last_block.block_hash;
                        info!("[Node:{}] Restored consensus: Height {}, LastHash {}", node_name, state.current_height, state.last_block_hash);
                    }
                }
            }
        }
    }
    state
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
    let rpc_node_name_for_server_task = args.node_name.clone(); // Clone for the RPC server task
    let rpc_local_peer_id_for_server_task = local_peer_id.clone(); // Clone for the RPC server task

    tokio::spawn(async move { // rpc_node_name_for_server_task, rpc_local_peer_id_for_server_task moved here
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
                    // Clone for each connection handler task
                    let rpc_handler_consensus_state = rpc_consensus_state_arc.clone();
                    let rpc_handler_p2p_service = rpc_p2p_service.clone();
                    let rpc_handler_node_name_conn = rpc_node_name_for_server_task.clone(); 
                    let rpc_handler_local_peer_id_conn = rpc_local_peer_id_for_server_task.clone();

                    debug!("[Node:{}:RPC] Accepted RPC connection from: {}", rpc_handler_node_name_conn, addr);
                    tokio::spawn(async move { // rpc_handler_node_name_conn, rpc_handler_local_peer_id_conn moved here
                        handle_rpc_connection(
                            stream, 
                            rpc_handler_consensus_state, 
                            rpc_handler_p2p_service,
                            rpc_handler_node_name_conn, // Use the clone
                            rpc_handler_local_peer_id_conn, // Use the clone
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
    loop {
        let node_name_for_loop_logging = main_loop_node_name.clone();
        let p2p_service_for_spawn = p2p_service.clone();
        
        tokio::select! {
            Some(app_event) = app_event_rx.recv() => { 
                match app_event {
                    AppP2PEvent::GossipsubMessage { source, topic_hash, message } => {
                        debug!("[Node:{}] Received Gossip from PeerId {:?}, Topic {:?}: {}", node_name_for_loop_logging, source, topic_hash.to_string(), message_summary(&message)); 
                        match message {
                            NetworkMessage::BlockProposal(serialized_block) => {
                                match bincode::deserialize::<Block>(&serialized_block) { 
                                    Ok(block) => {
                                        if block.proposer_id == node_name_for_loop_logging { continue; } 
                                        
                                        let mut local_consensus_state = consensus_state_arc.lock().await;
                                        match validate_and_apply_block(&mut local_consensus_state, &block) { 
                                            Ok(()) => {
                                                info!("[Node:{}] Validated Block H:{} from {} (PeerId: {:?})", node_name_for_loop_logging, block.height, block.proposer_id, source);
                                                if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&blockchain_file_path){
                                                    if writeln!(file, "{}", serde_json::to_string(&block).unwrap()).is_err(){/* log */}
                                                }
                                            }
                                            Err(e) => warn!("[Node:{}] Invalid block (H:{} from PeerId {:?}): {}", node_name_for_loop_logging, block.height, source, e),
                                        }
                                        drop(local_consensus_state);
                                    }
                                    Err(e) => error!("[Node:{}] Error deserializing block from PeerId {:?}: {}", node_name_for_loop_logging, source, e),
                                }
                            }
                            NetworkMessage::Transaction(serialized_tx_payload_data) => {
                                if is_sequencer { 
                                    match serde_json::from_slice::<TransactionPayload>(&serialized_tx_payload_data) { 
                                        Ok(tx_payload) => {
                                            let mut local_consensus_state = consensus_state_arc.lock().await;
                                            match submit_transaction_payload(&mut local_consensus_state, tx_payload.data) { 
                                                Ok(tx_id) => info!("[Node:{}:Seq] Mempooled TxID: {} from Gossip (PeerId {:?})", node_name_for_loop_logging, tx_id, source),
                                                Err(e) => error!("[Node:{}:Seq] Error submitting Gossip payload from PeerId {:?}: {}", node_name_for_loop_logging, source, e),
                                            }
                                            drop(local_consensus_state);
                                        }
                                        Err(e) => error!("[Node:{}:Seq] Error deserializing Gossip tx payload from JSON from PeerId {:?}: {}", node_name_for_loop_logging, source, e),
                                    }
                                } else {
                                    debug!("[Node:{}] Received transaction from {:?}, not a sequencer. Re-gossiping.", node_name_for_loop_logging, source);
                                    let node_name_for_regossip = node_name_for_loop_logging.clone();
                                    let p2p_service_for_regossip = p2p_service_for_spawn.clone(); 
                                    let msg_to_regossip = NetworkMessage::Transaction(serialized_tx_payload_data); 
                                    
                                    tokio::spawn(async move { 
                                       if let Err(e) = p2p_service_for_regossip.publish(AuroraTopic::Transactions, msg_to_regossip).await {
                                           warn!("[Node:{}] Failed to re-gossip transaction: {}", node_name_for_regossip, e);
                                       } else {
                                           debug!("[Node:{}] Re-gossiped transaction from PeerId {:?}", node_name_for_regossip, source);
                                       }
                                    });
                                }
                            }
                            NetworkMessage::NodeStateQuery { responder_peer_id } => {
                                let should_respond = responder_peer_id.as_ref().map_or(true, |id_str| id_str == &local_peer_id.to_string());
                                if should_respond {
                                    info!("[Node:{}] Received NodeStateQuery via Gossip from {}", node_name_for_loop_logging, source);
                                    let local_consensus_state_lock = consensus_state_arc.lock().await; // Renamed to avoid conflict
                                    let summary = NodeStateSummary { 
                                        node_id: local_peer_id.to_string(),
                                        name: node_name_for_loop_logging.clone(), 
                                        height: local_consensus_state_lock.current_height,
                                        last_block_hash: local_consensus_state_lock.last_block_hash.clone(),
                                    };
                                    drop(local_consensus_state_lock); // Drop lock
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
    mut stream: tokio::net::TcpStream, 
    consensus_state_arc: Arc<TokioMutex<NodeLocalConsensusState>>, // Corrected type name
    p2p_service: Arc<impl P2PService>, 
    node_name_handler: String, // Renamed to avoid conflict with outer scope if it were an issue
    local_peer_id_handler: PeerId, // Renamed for clarity
) {
    let (raw_reader, mut writer) = stream.split(); 
    let mut reader = TokioBufReader::new(raw_reader);

    let mut line = String::new();
    loop {
        // Clone values needed *inside* this loop iteration if they are moved into spawned tasks
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

                // Clone for potential spawn, if original rpc_req.id is needed later in this iteration
                let response_id_clone = rpc_req.id.clone(); 

                let response: RpcResponse = match rpc_req.method.as_str() {
                    "submit_transaction" => {
                        match serde_json::from_value::<HashMap<String, String>>(rpc_req.params) {
                            Ok(params) => {
                                if let Some(data_str) = params.get("data") {
                                    let tx_data_bytes = data_str.as_bytes().to_vec();
                                    let concord_tx_payload = TransactionPayload {data: tx_data_bytes.clone()};
                                    
                                    // Lock consensus state to submit transaction
                                    let tx_id_result = { // New scope for consensus_state lock
                                        let mut local_consensus_state = consensus_state_arc.lock().await;
                                        submit_transaction_payload(&mut local_consensus_state, tx_data_bytes)
                                    };

                                    match tx_id_result {
                                        Ok(tx_id) => {
                                            info!("[Node:{}:RPC] Transaction submitted via RPC. TxID: {}, Gossiping...", node_name_for_this_rpc_iter, tx_id);
                                            
                                            // Clone for the spawned task
                                            let p2p_service_for_gossip = p2p_service_for_this_rpc_iter.clone();
                                            let node_name_for_gossip = node_name_for_this_rpc_iter.clone();
                                            let tx_id_for_gossip = tx_id.clone(); // Clone tx_id for the spawn

                                            let network_tx_payload = serde_json::to_vec(&concord_tx_payload).expect("RPC: Failed to serialize TransactionPayload for network");
                                            let tx_gossip_msg = NetworkMessage::Transaction(network_tx_payload);
                                            
                                            tokio::spawn(async move { // tx_id_for_gossip, node_name_for_gossip, p2p_service_for_gossip moved
                                                if let Err(e) = p2p_service_for_gossip.publish(AuroraTopic::Transactions, tx_gossip_msg).await {
                                                    error!("[Node:{}:RPC] Failed to gossip transaction {} via P2P: {}", node_name_for_gossip, tx_id_for_gossip, e);
                                                } else {
                                                    info!("[Node:{}:RPC] Successfully gossiped transaction {}", node_name_for_gossip, tx_id_for_gossip);
                                                }
                                            });
                                            // Use the original tx_id for the response
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
                        let local_consensus_state_lock = consensus_state_arc.lock().await; // Renamed
                        let summary = NodeStateSummary {
                            node_id: local_peer_id_handler.to_string(), // Use local_peer_id_handler
                            name: node_name_handler.clone(), // Use node_name_handler (original from args)
                            height: local_consensus_state_lock.current_height,
                            last_block_hash: local_consensus_state_lock.last_block_hash.clone(),
                        };
                        drop(local_consensus_state_lock); // Drop lock
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
                    error!("[Node:{}:RPC] Error reading RPC line: {}", node_name_handler, e); // Use original node_name_handler for final error
                }
                break; 
            }
        }
    }
    debug!("[Node:{}:RPC] RPC connection handler finished for an address.", node_name_handler); // Use original
}