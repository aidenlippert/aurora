// nodes/graviton_edge_node/src/main.rs

use ecliptic_concordance::{
    ConsensusState as NodeLocalConsensusState,
    sequencer_create_block, submit_aurora_transaction, validate_and_apply_block,
    create_attestation, process_incoming_attestation,
    Block, AuroraTransaction, TransferAucPayload,
};
use triad_web::{
    NetworkMessage, AppP2PEvent, P2PService,
    initialize_p2p_service, 
    network_behaviour::AuroraTopic,
};
use novavault_flux_finance::{process_public_auc_transfer, get_account_balance as novavault_get_balance, ensure_account_exists_with_initial_funds};
use aethercore_runtime::ExecutionRequest as AetherExecutionRequest; // For the struct
use wasmi::Value as WasmiValue; // Use wasmi::Value directly

use serde::{Deserialize, Serialize};
use serde_json;
use serde_json::json;
use clap::Parser;
use std::fs::{File, OpenOptions, read_to_string as fs_read_to_string, write as fs_write};
use std::io::{self, BufRead, Write, BufReader as StdBufReader};
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::sync::Arc;
use std::collections::{HashMap, HashSet};

use tokio::sync::Mutex as TokioMutex;
use tokio::net::TcpListener;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};

use libp2p::{Multiaddr, PeerId};
use log::{info, warn, error, debug, trace};

use ed25519_dalek::{SigningKey, SecretKey as DalekSecretKey};
use rand::rngs::OsRng;
use hex;
use std::error::Error as StdError;
use once_cell::sync::Lazy;
use wasmi::core::F32 as WasmiF32; 


const SEQUENCER_ID_PREFIX: &str = "sequencer-";
const MAX_BLOCKS_PER_BATCH_RESPONSE: u32 = 20;
const MAX_BLOCKS_TO_REQUEST_IN_SYNC: u32 = 50;
const SYNC_RETRY_TIMEOUT_SECS: u64 = 30;
const SYNC_STUCK_THRESHOLD_SECS: u64 = SYNC_RETRY_TIMEOUT_SECS * 2 / 3;
const NODE_APP_KEY_FILENAME: &str = "app_signing_key.skhex";

const MOCK_VALIDATOR_APP_PUBKEYS_HEX: [&str; 2] = [
    "16b3295223e05522224d752825606001212ad2269eb169e5f6c3b57d767ed29b", 
    "b46946e89c9e07bd52768d231cd77013da4b9bcf727843a71455bd478c3db1bb", 
];
const ATTESTATION_THRESHOLD: usize = 1;

static TEMP_NONCE_TRACKER: Lazy<TokioMutex<HashMap<String, u64>>> = Lazy::new(|| TokioMutex::new(HashMap::new()));


fn load_or_generate_app_signing_key(key_path: &Path) -> Result<SigningKey, Box<dyn StdError + Send + Sync>> {
    if key_path.exists() {
        let sk_hex = fs_read_to_string(key_path)
            .map_err(|e| Box::new(e) as Box<dyn StdError + Send + Sync>)?;
        let sk_seed_bytes = hex::decode(sk_hex.trim())
            .map_err(|e| Box::new(e) as Box<dyn StdError + Send + Sync>)?;
        
        if sk_seed_bytes.len() != ed25519_dalek::SECRET_KEY_LENGTH {
            let err_msg = format!(
                "Invalid secret key seed length from {:?}: Expected {}, Got {}",
                key_path, ed25519_dalek::SECRET_KEY_LENGTH, sk_seed_bytes.len()
            );
            return Err(Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, err_msg)) as Box<dyn StdError + Send + Sync>);
        }
        
        let mut seed_array = [0u8; ed25519_dalek::SECRET_KEY_LENGTH];
        seed_array.copy_from_slice(&sk_seed_bytes);
        let dalek_secret_key = DalekSecretKey::from(seed_array);
        let signing_key = SigningKey::from(&dalek_secret_key);

        info!("[KeyMgmt] Loaded Ed25519 app signing key from {:?}", key_path);
        Ok(signing_key)
    } else {
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let sk_seed_bytes = signing_key.as_bytes();
        let sk_hex_to_save = hex::encode(sk_seed_bytes);
        
        if let Some(parent_dir) = key_path.parent() {
            std::fs::create_dir_all(parent_dir)
                .map_err(|e| Box::new(e) as Box<dyn StdError + Send + Sync>)?;
        }
        fs_write(key_path, sk_hex_to_save)
            .map_err(|e| Box::new(e) as Box<dyn StdError + Send + Sync>)?;
        info!("[KeyMgmt] Generated and saved new Ed25519 app signing key to {:?}", key_path);
        Ok(signing_key)
    }
}

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
    #[clap(long, help = "Unique Node ID")]
    node_name: String,
    #[clap(long, value_delimiter = ',', help = "Libp2p listen multiaddresses")]
    listen_addrs: String,
    #[clap(long, value_delimiter = ',', help = "Libp2p bootstrap peer multiaddresses")]
    bootstrap_peers: Option<String>,
    #[clap(long, help = "Data directory path")]
    data_dir: PathBuf,
}

fn load_consensus_state_from_disk(node_log_id: &str, blockchain_file_path: &Path) -> NodeLocalConsensusState {
    let mut state = NodeLocalConsensusState::new(node_log_id.to_string());
    let validator_app_pk_hexes_set: HashSet<String> = MOCK_VALIDATOR_APP_PUBKEYS_HEX.iter().map(|s| s.to_string()).collect();
    state.set_validators(validator_app_pk_hexes_set, ATTESTATION_THRESHOLD);

    if blockchain_file_path.exists() {
        if let Ok(file) = File::open(blockchain_file_path) {
            let reader = StdBufReader::new(file);
            for line in reader.lines() {
                if let Ok(line_content) = line {
                    if !line_content.trim().is_empty() {
                        if let Ok(block) = serde_json::from_str::<Block>(&line_content) {
                            if block.height >= state.current_height || (block.height == 0 && state.current_height == 0 && state.last_block_hash == "GENESIS_HASH_0.0.1") {
                                if (block.height == 0 && block.prev_block_hash == "GENESIS_HASH_0.0.1") || (block.height > 0 && block.prev_block_hash == state.last_block_hash) {
                                   state.current_height = block.height;
                                   state.last_block_hash = block.block_hash.clone();
                                   for tx_wrapper in &block.transactions {
                                       if let AuroraTransaction::TransferAUC(transfer_payload) = &tx_wrapper.payload {
                                           if let Err(e) = process_public_auc_transfer(transfer_payload, block.height) {
                                               error!("[Node:{}] Error re-applying loaded TransferAUC tx {} from block H:{}: {}", node_log_id, tx_wrapper.id, block.height, e);
                                           }
                                       }
                                   }
                                } else if block.height > state.current_height {
                                    warn!("[Node:{}] Discontinuity loading chain. Jumping to H:{}. Local NovaVault state may be inconsistent until full resync.", node_log_id, block.height);
                                    state.current_height = block.height;
                                    state.last_block_hash = block.block_hash.clone();
                                }
                            }
                        } else {
                             error!("[Node:{}] Failed to parse block from chain file line: '{}'", node_log_id, line_content);
                        }
                    }
                }
            }
            info!("[Node:{}] Restored consensus: Height {}, LastHash {:.8}", node_log_id, state.current_height, state.last_block_hash);
        } else {
            warn!("[Node:{}] Could not open blockchain file {:?}. Starting from genesis.", node_log_id, blockchain_file_path);
        }
    } else {
        info!("[Node:{}] No blockchain file found at {:?}. Starting from genesis.", node_log_id, blockchain_file_path);
    }
    state
}

fn read_blocks_from_file(
    blockchain_file_path: &Path,
    start_height: u64,
    max_count: u32,
) -> Result<Vec<Block>, io::Error> {
    let mut blocks = Vec::new();
    if !blockchain_file_path.exists() { return Ok(blocks); }
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
                    if count >= max_count { break; }
                }
            }
            Err(e) => { error!("Failed to parse block from file: {}", e); }
        }
    }
    Ok(blocks)
}

#[derive(Debug, Serialize, Deserialize)]
struct NodeStateSummary {
    node_name: String,
    libp2p_peer_id: String,
    app_layer_pk_hex: String,
    current_height: u64,
    last_block_hash: String,
    sync_state: String,
    is_sequencer: bool,
    known_validators_count: usize,
    attestation_threshold: usize,
}

#[derive(Serialize, Deserialize, Debug)]
struct RpcRequest { id: String, method: String, params: serde_json::Value }
#[derive(Serialize, Deserialize, Debug)]
struct RpcResponse { id: String, result: Option<serde_json::Value>, error: Option<RpcError> }
#[derive(Serialize, Deserialize, Debug)]
struct RpcError { code: i32, message: String }

async fn send_block_request_range_to_peer(
    p2p_service_clone: Arc<impl P2PService>,
    node_name_log_sender: String,
    local_libp2p_peer_id_str: String,
    target_sync_libp2p_peer: PeerId,
    start_h: u64,
    end_h: u64,
) {
    let sync_req_msg = NetworkMessage::BlockRequestRange {
        start_height: start_h, end_height: Some(end_h),
        max_blocks_to_send: Some(MAX_BLOCKS_TO_REQUEST_IN_SYNC),
        requesting_peer_id: local_libp2p_peer_id_str,
    };
    info!("[Node:{}] Sending BlockRequestRange (start_h:{}, target_end_h:{}) to peer {:?}",
          node_name_log_sender, start_h, end_h, target_sync_libp2p_peer);
    if let Err(e) = p2p_service_clone.publish(AuroraTopic::Consensus, sync_req_msg).await {
        error!("[Node:{}] Failed to publish BlockRequestRange to peer {:?}: {}", node_name_log_sender, target_sync_libp2p_peer, e);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn StdError + Send + Sync>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,graviton_edge_node=debug,ecliptic_concordance=trace,triad_web=debug,novavault_flux_finance=debug")).init();
    let args = Args::parse();
    let main_loop_node_name = args.node_name.clone();

    if !args.data_dir.exists() { std::fs::create_dir_all(&args.data_dir)?; }
    let blockchain_file_path = args.data_dir.join(format!("{}_blockchain.jsonl", args.node_name));
    let app_signing_key_path = args.data_dir.join(NODE_APP_KEY_FILENAME);

    let app_signing_key = load_or_generate_app_signing_key(&app_signing_key_path)?;
    let app_verifying_key_hex = hex::encode(app_signing_key.verifying_key().as_bytes());
    info!("[Node:{}] App Layer Ed25519 PK (Hex): {}", args.node_name, app_verifying_key_hex);

    let is_sequencer = args.node_name.starts_with(SEQUENCER_ID_PREFIX);
    info!("[Node:{}] Starting. Sequencer: {}. ChainFile: {:?}", args.node_name, is_sequencer, blockchain_file_path);
    
    ensure_account_exists_with_initial_funds(&app_verifying_key_hex);

    let rpc_port_str = args.listen_addrs.split(',').next().and_then(|addr| addr.split('/').nth(4)).unwrap_or("0");
    let rpc_port = rpc_port_str.parse::<u16>().unwrap_or(8000) + 10000;
    let rpc_listen_addr_str = format!("127.0.0.1:{}", rpc_port);
    info!("[Node:{}] RPC server will listen on: {}", args.node_name, rpc_listen_addr_str);

    let mut initial_consensus_state = load_consensus_state_from_disk(&args.node_name, &blockchain_file_path);
    let validator_app_pk_hexes: HashSet<String> = MOCK_VALIDATOR_APP_PUBKEYS_HEX.iter().map(|s| s.to_string()).collect();
    initial_consensus_state.set_validators(validator_app_pk_hexes, ATTESTATION_THRESHOLD);
    let consensus_state_arc = Arc::new(TokioMutex::new(initial_consensus_state));

    let current_sync_state_arc = Arc::new(TokioMutex::new(NodeSyncState::Synced));
    let buffered_future_blocks_arc = Arc::new(TokioMutex::new(HashMap::<u64, Block>::new()));
    let locally_confirmed_blocks_cache_arc = Arc::new(TokioMutex::new(HashSet::<String>::new()));

    let listen_multiaddrs: Vec<Multiaddr> = args.listen_addrs.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if listen_multiaddrs.is_empty() { return Err("No valid libp2p listen multiaddresses".into()); }
    let bootstrap_peers: Vec<Multiaddr> = args.bootstrap_peers.map_or_else(Vec::new, |s| s.split(',').filter_map(|p| p.trim().parse().ok()).collect());

    let (p2p_service, mut app_event_rx) = initialize_p2p_service(
        args.node_name.clone(), args.data_dir.join("libp2p_data"), listen_multiaddrs, bootstrap_peers
    ).await?;
    let local_libp2p_peer_id = p2p_service.get_peer_id();
    info!("[Node:{}] P2P service initialized. Local libp2p PeerId: {}", args.node_name, local_libp2p_peer_id);

    let rpc_cs_clone = consensus_state_arc.clone();
    let rpc_p2p_clone = p2p_service.clone();
    let rpc_name_clone = args.node_name.clone();
    let rpc_libp2p_id_clone = local_libp2p_peer_id.clone();
    let rpc_app_pk_clone = app_verifying_key_hex.clone();
    let rpc_app_sk_clone = Arc::new(app_signing_key.clone());
    let rpc_sync_clone = current_sync_state_arc.clone();
    let rpc_is_sequencer = is_sequencer;
    let rpc_consensus_state_for_height_arc = consensus_state_arc.clone();

    tokio::spawn(async move {
        let listener = match TcpListener::bind(&rpc_listen_addr_str).await {
            Ok(l) => l, Err(e) => { error!("[RPC] Bind Err: {}", e); return; }
        };
        info!("[Node:{}:RPC] Listening on {}", rpc_name_clone, rpc_listen_addr_str);
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let cs = rpc_cs_clone.clone();
                    let p2p = rpc_p2p_clone.clone();
                    let name = rpc_name_clone.clone();
                    let libp2p_id = rpc_libp2p_id_clone.clone();
                    let app_pk = rpc_app_pk_clone.clone();
                    let app_sk = rpc_app_sk_clone.clone();
                    let sync_s = rpc_sync_clone.clone();
                    let cs_for_height = rpc_consensus_state_for_height_arc.clone();
                    tokio::spawn(async move {
                        let current_block_height = cs_for_height.lock().await.current_height;
                        handle_rpc_connection(stream, cs, p2p, name, libp2p_id, app_pk, app_sk, sync_s, rpc_is_sequencer, current_block_height).await;
                    });
                }
                Err(e) => error!("[Node:{}:RPC] Accept Error: {}", rpc_name_clone, e),
            }
        }
    });

    if is_sequencer {
        let seq_name = args.node_name.clone();
        let seq_p2p = p2p_service.clone();
        let seq_bchain_path = blockchain_file_path.clone();
        let seq_cs = consensus_state_arc.clone();
        let seq_signing_key_param = app_signing_key.clone();
        let seq_sync_state = current_sync_state_arc.clone();
        let seq_confirmed_cache = locally_confirmed_blocks_cache_arc.clone();

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;
                if !matches!(*seq_sync_state.lock().await, NodeSyncState::Synced) { continue; }
                let mut cs_lock = seq_cs.lock().await;
                let last_h = cs_lock.last_block_hash.clone();
                let curr_h = cs_lock.current_height;
                if curr_h > 0 || (curr_h == 0 && last_h != "GENESIS_HASH_0.0.1") {
                    if !seq_confirmed_cache.lock().await.contains(&last_h) {
                        debug!("[Node:{}:Seq] Last H:{} Hash:{:.8} not confirmed. Waiting.", seq_name, curr_h, last_h);
                        continue;
                    }
                }
                match sequencer_create_block(&mut cs_lock, &seq_signing_key_param) {
                    Ok(new_block) => {
                        info!("[Node:{}:Seq] Created Block H:{}. Publishing...", seq_name, new_block.height);
                        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&seq_bchain_path) {
                            let _ = writeln!(f, "{}", serde_json::to_string(&new_block).unwrap_or_default());
                        }
                        if let Ok(sb) = bincode::serialize(&new_block) {
                            if let Err(e) = seq_p2p.publish(AuroraTopic::Blocks, NetworkMessage::BlockProposal(sb)).await {
                                warn!("[Node:{}:Seq] Publish block error: {}", seq_name, e);
                            }
                        } else { error!("[Node:{}:Seq] Serialize block error.", seq_name); }
                    }
                    Err(e) if e == "No pending transactions to create a block." => {}
                    Err(e) => error!("[Node:{}:Seq] Create block error: {}", seq_name, e),
                }
            }
        });
    }
    
    let periodic_p2p_clone = p2p_service.clone();
    let periodic_name_clone = main_loop_node_name.clone();
    let periodic_libp2p_id_clone = local_libp2p_peer_id.to_string();
    let periodic_cs_clone = consensus_state_arc.clone();
    let periodic_sync_clone = current_sync_state_arc.clone();
    let periodic_bchain_path_clone = blockchain_file_path.clone();
    let periodic_buffered_blocks_clone = buffered_future_blocks_arc.clone();

    tokio::spawn(async move {
         loop {
            tokio::time::sleep(Duration::from_secs(SYNC_RETRY_TIMEOUT_SECS)).await;
            let mut sync_g = periodic_sync_clone.lock().await;
            match *sync_g {
                NodeSyncState::AttemptingSync { target_peer, highest_known_height, next_expected_batch_start_height, last_request_time } => {
                    if last_request_time.elapsed() > Duration::from_secs(SYNC_STUCK_THRESHOLD_SECS) {
                        if let Some(p_retry) = target_peer {
                             let cs_l = periodic_cs_clone.lock().await;
                             let ch = cs_l.current_height; let clh = cs_l.last_block_hash.clone(); drop(cs_l);
                             let r_start_h = if ch == 0 && clh == "GENESIS_HASH_0.0.1" { 0 } else { next_expected_batch_start_height };
                             if r_start_h <= highest_known_height && (r_start_h > ch || (r_start_h == 0 && ch == 0 && clh == "GENESIS_HASH_0.0.1")) {
                                 drop(sync_g);
                                 send_block_request_range_to_peer(periodic_p2p_clone.clone(), periodic_name_clone.clone(), periodic_libp2p_id_clone.clone(), p_retry, r_start_h, highest_known_height).await;
                                 let mut ssg_upd = periodic_sync_clone.lock().await;
                                 if let NodeSyncState::AttemptingSync { last_request_time: ref mut time, .. } = *ssg_upd { *time = tokio::time::Instant::now(); }
                             } else { *sync_g = NodeSyncState::Synced; }
                        } else { *sync_g = NodeSyncState::Synced; }
                    }
                }
                NodeSyncState::Synced => {
                    let mut b_map = periodic_buffered_blocks_clone.lock().await;
                    if !b_map.is_empty() {
                        let mut cs_l = periodic_cs_clone.lock().await;
                        let mut next_a_h = if cs_l.current_height == 0 && cs_l.last_block_hash == "GENESIS_HASH_0.0.1" { 0 } else { cs_l.current_height + 1 };
                        let mut applied_count = 0;
                        while let Some(blk_to_apply) = b_map.remove(&next_a_h) {
                            let mut all_txs_in_block_applied_locally = true;
                            let block_height_for_tx_apply = blk_to_apply.height; 
                            for tx_wrapper in &blk_to_apply.transactions {
                                if let AuroraTransaction::TransferAUC(transfer_payload) = &tx_wrapper.payload {
                                    if let Err(e) = process_public_auc_transfer(transfer_payload, block_height_for_tx_apply) {
                                        error!("[Node:{}:PeriodicSyncBuffer] Error applying buffered TransferAUC tx {} from block H:{}: {}. Re-inserting block.", 
                                            periodic_name_clone, tx_wrapper.id, block_height_for_tx_apply, e);
                                        all_txs_in_block_applied_locally = false;
                                    }
                                }
                            }
                            if !all_txs_in_block_applied_locally { 
                                b_map.insert(blk_to_apply.height, blk_to_apply); 
                                break; 
                            }

                            if validate_and_apply_block(&mut cs_l, &blk_to_apply).is_ok() {
                                info!("[Node:{}:PeriodicSyncBuffer] Applied buffered block H:{} from periodic check.", periodic_name_clone, blk_to_apply.height);
                                if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&periodic_bchain_path_clone) { 
                                    let _ = writeln!(f, "{}", serde_json::to_string(&blk_to_apply).unwrap_or_default());
                                }
                                next_a_h = cs_l.current_height + 1;
                                applied_count += 1;
                            } else { 
                                warn!("[Node:{}:PeriodicSyncBuffer] Failed to validate/apply buffered block H:{} from periodic check. Re-inserting.", periodic_name_clone, blk_to_apply.height);
                                b_map.insert(blk_to_apply.height, blk_to_apply); 
                                break; 
                            }
                        }
                        if applied_count > 0 {
                             info!("[Node:{}:PeriodicSyncBuffer] Applied {} blocks from buffer during periodic check. New height: {}", periodic_name_clone, applied_count, cs_l.current_height);
                        }
                    }
                }
            }
        }
    });

    info!("[Node:{}] Main event loop started.", main_loop_node_name);
    loop {
        let name_log_l = main_loop_node_name.clone();
        let p2p_l_spawn = p2p_service.clone();
        let bchain_f_l_h = blockchain_file_path.clone();
        let local_libp2p_id_l_req = local_libp2p_peer_id.to_string();
        let app_sk_l_att = app_signing_key.clone();
        let app_vk_hex_l_own_check = app_verifying_key_hex.clone();
        let buffered_b_l_clone = buffered_future_blocks_arc.clone();
        let confirmed_c_l = locally_confirmed_blocks_cache_arc.clone();

        tokio::select! {
            Some(app_event) = app_event_rx.recv() => {
                let mut sync_g_main = current_sync_state_arc.lock().await;
                match app_event {
                    AppP2PEvent::GossipsubMessage { source, message, .. } => {
                        match message {
                            NetworkMessage::BlockProposal(ser_blk) => {
                                match bincode::deserialize::<Block>(&ser_blk) {
                                    Ok(blk) => {
                                        if blk.proposer_pk_hex() == app_vk_hex_l_own_check { continue; }
                                        let mut cs_l = consensus_state_arc.lock().await;
                                        let ch = cs_l.current_height; 
                                        let clh = cs_l.last_block_hash.clone();

                                        let is_next_sequential_if_at_genesis = blk.height == 0 && ch == 0 && clh == "GENESIS_HASH_0.0.1";
                                        let is_next_sequential_after_genesis = blk.height == ch + 1 && ch >= 0 && clh != "GENESIS_HASH_0.0.1";

                                        if matches!(*sync_g_main, NodeSyncState::Synced) && (is_next_sequential_if_at_genesis || is_next_sequential_after_genesis) {
                                            let mut all_txs_applied_locally = true;
                                            let block_height_for_apply = blk.height; 
                                            for tx_wrapper in &blk.transactions {
                                                if let AuroraTransaction::TransferAUC(transfer_payload) = &tx_wrapper.payload {
                                                    if let Err(e) = process_public_auc_transfer(transfer_payload, block_height_for_apply) {
                                                        warn!("[Node:{}] Failed to process TransferAUC tx {} from received block H:{}: {}. Invalidating block.", 
                                                            name_log_l, tx_wrapper.id, block_height_for_apply, e);
                                                        all_txs_applied_locally = false;
                                                        break; 
                                                    }
                                                }
                                            }

                                            if all_txs_applied_locally && validate_and_apply_block(&mut cs_l, &blk).is_ok() {
                                                info!("[Node:{}] Applied gossiped Block H:{} from PK:{:.8} (tx count: {})", name_log_l, blk.height, blk.proposer_pk_hex(), blk.transactions.len());
                                                if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&bchain_f_l_h) { let _ = writeln!(f, "{}", serde_json::to_string(&blk).unwrap_or_default()); }
                                                if !is_sequencer {
                                                    match create_attestation(blk.height, &blk.block_hash, &app_sk_l_att) {
                                                        Ok(att) => {
                                                            let att_m = NetworkMessage::BlockAttestation(att);
                                                            let p2p_att_c = p2p_l_spawn.clone(); let name_att_c = name_log_l.clone();
                                                            drop(cs_l); drop(sync_g_main);
                                                            tokio::spawn(async move { if let Err(e) = p2p_att_c.publish(AuroraTopic::Attestations, att_m).await {error!("[{}] Pub Att Err: {}", name_att_c,e);}});
                                                            continue;
                                                        }
                                                        Err(e) => error!("[Node:{}] Create Att Err: {}", name_log_l, e),
                                                    }
                                                }
                                            } else { 
                                                if !all_txs_applied_locally {} else {
                                                    warn!("[Node:{}] Invalid gossiped block H:{} (expected sequential) from PK:{:.8} after local tx processing.", name_log_l, blk.height, blk.proposer_pk_hex()); 
                                                }
                                            }
                                        } else { 
                                            debug!("[Node:{}] Received non-sequential or out-of-sync block H:{} from PK:{:.8} (Current H:{}, SyncState:{:?}). Buffering/Triggering Sync.", 
                                                name_log_l, blk.height, blk.proposer_pk_hex(), ch, *sync_g_main);
                                            buffered_b_l_clone.lock().await.insert(blk.height, blk.clone());
                                            let mut initiate_sync = false; let mut new_target_p = source; let mut new_highest_h = blk.height;
                                            match *sync_g_main {
                                                NodeSyncState::Synced => { initiate_sync = true; }
                                                NodeSyncState::AttemptingSync { target_peer: Some(curr_p), highest_known_height: curr_h, last_request_time, ..} => {
                                                    if blk.height > curr_h { new_highest_h = blk.height; debug!("[Node:{}] Sync in progress. Updated target_H to {} due to new block from {:?}.", name_log_l, blk.height, source); } 
                                                    else { new_highest_h = curr_h; }
                                                    if (source != curr_p && blk.height > curr_h) || last_request_time.elapsed() > Duration::from_secs(SYNC_STUCK_THRESHOLD_SECS) {
                                                        initiate_sync = true;
                                                        if source == curr_p || blk.height <= curr_h { new_target_p = curr_p; } else { new_target_p = source; }
                                                    }
                                                }
                                                NodeSyncState::AttemptingSync { ..} => { initiate_sync = true; }
                                            }
                                            if initiate_sync {
                                                let next_ssh = if ch == 0 && clh == "GENESIS_HASH_0.0.1" { 0 } else { ch + 1 };
                                                if next_ssh <= new_highest_h { 
                                                    info!("[Node:{}] Initiating sync: TargetPeer:{:?}, FromH:{}, ToH:{}", name_log_l, new_target_p, next_ssh, new_highest_h);
                                                    *sync_g_main = NodeSyncState::AttemptingSync { target_peer: Some(new_target_p), highest_known_height: new_highest_h, next_expected_batch_start_height: next_ssh, last_request_time: tokio::time::Instant::now() };
                                                    drop(cs_l); drop(sync_g_main);
                                                    send_block_request_range_to_peer(p2p_l_spawn.clone(), name_log_l.clone(), local_libp2p_id_l_req.clone(), new_target_p, next_ssh, new_highest_h).await;
                                                    continue;
                                                } else { debug!("[Node:{}] Block H:{} buffered. No sync range.", name_log_l, blk.height); }
                                            }
                                        }
                                    }
                                    Err(e) => error!("[Node:{}] Deserialize Block Err from {:?}: {}", name_log_l, source, e),
                                }
                            }
                            NetworkMessage::Transaction(aurora_tx) => {
                                if !matches!(*sync_g_main, NodeSyncState::Synced) { continue; }
                                if is_sequencer {
                                    let mut cs_l = consensus_state_arc.lock().await;
                                    if let Ok(ctx_id) = submit_aurora_transaction(&mut cs_l, aurora_tx.clone()) { 
                                        info!("[Node:{}] Mempooled AuroraTx (type {:?}) via Gossip. CTxID: {}", name_log_l, aurora_tx, ctx_id); 
                                    } else { 
                                        error!("[Node:{}] Error submitting Gossip AuroraTx", name_log_l); 
                                    }
                                } else {
                                    trace!("[Node:{}] Non-sequencer received AuroraTransaction {:?} via gossip. Ignoring for mempool.", name_log_l, aurora_tx);
                                }
                            }
                            NetworkMessage::BlockRequestRange { start_height, end_height, max_blocks_to_send, requesting_peer_id: _ } => {
                                let max_s = max_blocks_to_send.unwrap_or(MAX_BLOCKS_PER_BATCH_RESPONSE).min(MAX_BLOCKS_PER_BATCH_RESPONSE);
                                match read_blocks_from_file(&bchain_f_l_h, start_height, max_s) {
                                    Ok(f_blks) => {
                                        let r_msg = if f_blks.is_empty() {
                                            NetworkMessage::NoBlocksInRange { requested_start: start_height, requested_end: end_height, responder_peer_id: local_libp2p_id_l_req.clone() }
                                        } else {
                                            let fh = f_blks.first().map_or(0, |b|b.height); let th = f_blks.last().map_or(0, |b|b.height);
                                            let sb_data: Vec<Vec<u8>> = f_blks.into_iter().filter_map(|b| bincode::serialize(&b).ok()).collect();
                                            NetworkMessage::BlockResponseBatch { blocks_data: sb_data, from_height: fh, to_height: th }
                                        };
                                        let p2p_r_c = p2p_l_spawn.clone(); let name_r_c = name_log_l.clone();
                                        drop(sync_g_main);
                                        tokio::spawn(async move { if let Err(e) = p2p_r_c.publish(AuroraTopic::Consensus, r_msg).await { error!("[{}] Pub BlkResp Err: {}", name_r_c, e);}});
                                        continue;
                                    }
                                    Err(e) => error!("[Node:{}] Read blocks for req error: {}", name_log_l, e),
                                }
                            }
                            NetworkMessage::BlockResponseBatch { blocks_data, from_height, to_height } => {
                                if let NodeSyncState::AttemptingSync { target_peer, highest_known_height, next_expected_batch_start_height, .. } = *sync_g_main {
                                    if Some(source) != target_peer { continue; }
                                    let mut cs_l = consensus_state_arc.lock().await;
                                    let is_sa = (from_height == 0 && cs_l.current_height == 0 && cs_l.last_block_hash == "GENESIS_HASH_0.0.1") || (from_height > 0 && from_height == cs_l.current_height + 1);
                                    if from_height != next_expected_batch_start_height && !is_sa { *sync_g_main = NodeSyncState::Synced; warn!("[Node:{}:Sync] Batch out of order/unexpected. Resetting.", name_log_l); continue; }
                                    
                                    let mut all_ok = true; let mut last_ah = cs_l.current_height;
                                    for bb in blocks_data {
                                        match bincode::deserialize::<Block>(&bb) {
                                            Ok(blk) => {
                                                let exp_nh = if cs_l.current_height == 0 && cs_l.last_block_hash == "GENESIS_HASH_0.0.1" && blk.height == 0 {0} else {cs_l.current_height + 1};
                                                if blk.height == exp_nh {
                                                    let mut txs_in_synced_block_ok = true;
                                                    let block_height_for_batch_apply = blk.height;
                                                    for tx_wrapper_synced in &blk.transactions {
                                                        if let AuroraTransaction::TransferAUC(transfer_payload_synced) = &tx_wrapper_synced.payload {
                                                            if let Err(e) = process_public_auc_transfer(transfer_payload_synced, block_height_for_batch_apply) {
                                                                error!("[Node:{}:Sync] Error applying synced TransferAUC tx {} from block H:{}: {}. Halting batch.", 
                                                                    name_log_l, tx_wrapper_synced.id, block_height_for_batch_apply, e);
                                                                txs_in_synced_block_ok = false;
                                                                break;
                                                            }
                                                        }
                                                    }
                                                    if !txs_in_synced_block_ok { all_ok = false; break; }

                                                    if validate_and_apply_block(&mut cs_l, &blk).is_ok() {
                                                        last_ah = blk.height;
                                                        info!("[Node:{}:Sync] Applied synced block H:{} from batch.", name_log_l, blk.height);
                                                        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&bchain_f_l_h) {let _ = writeln!(f, "{}", serde_json::to_string(&blk).unwrap_or_default());}
                                                    } else { all_ok = false; warn!("[Node:{}:Sync] Failed to validate_and_apply_block H:{} from batch.", name_log_l, blk.height); break; }
                                                } else { all_ok = false; warn!("[Node:{}:Sync] Out of order block H:{} in batch (expected H:{}).", name_log_l, blk.height, exp_nh); break; }
                                            } Err(e) => { all_ok = false; error!("[Node:{}:Sync] Deserialize block in batch error: {}.", name_log_l, e); break; }
                                        }
                                    }
                                    let ch_after_b = cs_l.current_height; drop(cs_l);
                                    
                                    if ch_after_b >= highest_known_height {
                                        *sync_g_main = NodeSyncState::Synced; info!("[Node:{}:Sync] Sync complete to H:{}.", name_log_l, ch_after_b);
                                        let mut bm_g = buffered_b_l_clone.lock().await;
                                        let mut next_bh = if ch_after_b == 0 && highest_known_height == 0 { 0 } else { ch_after_b + 1};
                                        let mut applied_from_buffer_count = 0;
                                        while let Some(blk_buf) = bm_g.remove(&next_bh) {
                                            let mut cs_b_l = consensus_state_arc.lock().await;
                                            let mut tx_in_buf_blk_ok = true;
                                            let block_height_buf_apply = blk_buf.height;
                                            for tx_wrap_buf in &blk_buf.transactions {
                                                if let AuroraTransaction::TransferAUC(tp_buf) = &tx_wrap_buf.payload {
                                                    if let Err(e) = process_public_auc_transfer(tp_buf, block_height_buf_apply) { error!("[{}:SyncBuf] Error applying buf TransferAUC:{}",name_log_l,e); tx_in_buf_blk_ok = false; break;}
                                                }
                                            }
                                            if !tx_in_buf_blk_ok { bm_g.insert(blk_buf.height, blk_buf); break; }

                                            if validate_and_apply_block(&mut cs_b_l, &blk_buf).is_ok() {
                                                info!("[Node:{}:SyncBuf] Applied buffered H:{}", name_log_l, blk_buf.height);
                                                if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&bchain_f_l_h) { let _ = writeln!(f, "{}", serde_json::to_string(&blk_buf).unwrap_or_default());}
                                                next_bh = cs_b_l.current_height + 1; applied_from_buffer_count += 1;
                                            } else { bm_g.insert(blk_buf.height, blk_buf); break;}
                                        }
                                        if applied_from_buffer_count > 0 { info!("[Node:{}:SyncBuf] Applied {} blocks from buffer post-sync.", name_log_l, applied_from_buffer_count); }

                                    } else if all_ok && last_ah == to_height {
                                        let next_rs = last_ah + 1;
                                        if next_rs <= highest_known_height {
                                            *sync_g_main = NodeSyncState::AttemptingSync { target_peer, highest_known_height, next_expected_batch_start_height: next_rs, last_request_time: tokio::time::Instant::now()};
                                            if let Some(pr) = target_peer {
                                                drop(sync_g_main);
                                                send_block_request_range_to_peer(p2p_l_spawn.clone(), name_log_l.clone(), local_libp2p_id_l_req.clone(), pr, next_rs, highest_known_height).await;
                                                continue;
                                            } else { *sync_g_main = NodeSyncState::Synced; }
                                        } else { *sync_g_main = NodeSyncState::Synced; }
                                    } else { *sync_g_main = NodeSyncState::Synced; warn!("[Node:{}:Sync] Sync batch incomplete/failed. Resetting.", name_log_l); }
                                }
                            }
                            NetworkMessage::NoBlocksInRange { .. } => {
                                if let NodeSyncState::AttemptingSync { target_peer: Some(sti), .. } = *sync_g_main {
                                    if source == sti { *sync_g_main = NodeSyncState::Synced; warn!("[Node:{}:Sync] Target peer reported NoBlocksInRange. Resetting.", name_log_l); }
                                }
                            }
                            NetworkMessage::BlockAttestation(attestation) => {
                                if attestation.attestor_pk_hex() == app_vk_hex_l_own_check { continue; }
                                info!("[Node:{}] Received BlockAttestation for H:{} Hash:{:.8} from PK:{:.8} (via p2p peer {:?})",
                                    name_log_l, attestation.block_height, attestation.block_hash, attestation.attestor_pk_hex(), source);
                                let mut cs_l = consensus_state_arc.lock().await;
                                if process_incoming_attestation(&mut cs_l, &attestation) {
                                    confirmed_c_l.lock().await.insert(attestation.block_hash);
                                }
                            }
                             _ => { trace!("[Node:{}] Unhandled Gossipsub msg type.", name_log_l); }
                        }
                    }
                    AppP2PEvent::DirectMessage { source: direct_source, message: direct_message } => {
                        trace!("[Node:{}] Received unhandled DirectMessage from {:?}: {:?}", name_log_l, direct_source, triad_web::message_summary(&direct_message));
                    }
                    AppP2PEvent::PeerConnected(pid) => { info!("[Node:{}] Peer connected: {}", name_log_l, pid); }
                    AppP2PEvent::PeerDisconnected(pid) => { 
                        info!("[Node:{}] Peer disconnected: {}", name_log_l, pid); 
                        if let NodeSyncState::AttemptingSync { target_peer: Some(sti), .. } = *sync_g_main {
                            if sti == pid { *sync_g_main = NodeSyncState::Synced; warn!("[Node:{}:Sync] Sync target peer disconnected. Resetting.", name_log_l); }
                        }
                    }
                }
            }
            else => { error!("[Node:{}] P2P event channel closed.", main_loop_node_name); break; }
        }
    }
    Ok(())
}

async fn handle_rpc_connection(
    stream: tokio::net::TcpStream,
    consensus_state_arc: Arc<TokioMutex<NodeLocalConsensusState>>,
    p2p_service: Arc<impl P2PService>,
    node_name_handler: String,
    local_libp2p_peer_id_handler: PeerId,
    node_app_pk_hex: String, 
    _node_app_signing_key: Arc<SigningKey>, 
    sync_state_arc: Arc<TokioMutex<NodeSyncState>>,
    is_sequencer_rpc: bool,
    current_block_height_for_rpc_ops: u64,
) {
    let (raw_reader, mut writer) = stream.into_split();
    let mut reader = TokioBufReader::new(raw_reader);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {
                let req_json = line.trim();
                if req_json.is_empty() { continue; }
                let rpc_req: RpcRequest = match serde_json::from_str(req_json) { Ok(r) => r, Err(e) => { 
                    error!("[Node:{}:RPC] Parse RPC req error: {}", node_name_handler, e);
                    let err_resp = RpcResponse { id: "unknown".to_string(), result: None, error: Some(RpcError { code: -32700, message: "Parse error".to_string() })};
                    if let Ok(rj) = serde_json::to_string(&err_resp) { let _ = writer.write_all(format!("{}\n", rj).as_bytes()).await; let _ = writer.flush().await;}
                    continue; 
                }};
                let resp_id = rpc_req.id.clone();
                
                let response: RpcResponse = match rpc_req.method.as_str() {
                    "submit_transaction" => {
                        if !matches!(*sync_state_arc.lock().await, NodeSyncState::Synced) {
                             RpcResponse {id: resp_id, result:None, error: Some(RpcError{code:-100, message:"Node not synced".into()})}
                        } else {
                            match serde_json::from_value::<TransferAucPayload>(rpc_req.params.clone()) {
                                Ok(mut transfer_payload) => { 
                                    if transfer_payload.sender_pk_hex.is_empty() || transfer_payload.sender_pk_hex == "self" {
                                        transfer_payload.sender_pk_hex = node_app_pk_hex.clone();
                                        let mut nonce_map = TEMP_NONCE_TRACKER.lock().await;
                                        let next_nonce_val = nonce_map.entry(node_app_pk_hex.clone()).or_insert(0);
                                        transfer_payload.nonce = *next_nonce_val;
                                        *next_nonce_val += 1;
                                        info!("[Node:{}:RPC] Using self as sender for TransferAUC, PK: {:.8}, Nonce: {}", 
                                            node_name_handler, node_app_pk_hex, transfer_payload.nonce);
                                    } else {
                                        info!("[Node:{}:RPC] TransferAUC submitted for sender: {:.8}, Nonce: {}", 
                                            node_name_handler, transfer_payload.sender_pk_hex, transfer_payload.nonce);
                                    }

                                    match process_public_auc_transfer(&transfer_payload, current_block_height_for_rpc_ops) {
                                        Ok(nova_vault_op_id) => {
                                            let aurora_tx = AuroraTransaction::TransferAUC(transfer_payload.clone());
                                            let consensus_tx_id_res = { 
                                                let mut cs_lock = consensus_state_arc.lock().await;
                                                submit_aurora_transaction(&mut cs_lock, aurora_tx.clone())
                                            };

                                            match consensus_tx_id_res {
                                                Ok(ctx_id) => {
                                                    info!("[Node:{}:RPC] TransferAUC submitted to consensus. CTxID: {}, NovaVaultOpID: {}. Gossiping...", 
                                                        node_name_handler, ctx_id, nova_vault_op_id);
                                                    
                                                    let p2p_c = p2p_service.clone(); 
                                                    let name_c = node_name_handler.clone();
                                                    let aurora_tx_to_gossip = aurora_tx.clone();
                                                    let ctx_id_clone_for_gossip = ctx_id.clone(); 
                                                    tokio::spawn(async move { 
                                                        if let Err(e) = p2p_c.publish(AuroraTopic::Transactions, NetworkMessage::Transaction(aurora_tx_to_gossip)).await {
                                                            error!("[{}:RPC:Gossip] Failed to gossip TransferAUC CTxID {}: {}", name_c, ctx_id_clone_for_gossip, e);
                                                        } else {
                                                            debug!("[{}:RPC:Gossip] Gossiped TransferAUC CTxID {}", name_c, ctx_id_clone_for_gossip);
                                                        }
                                                    });
                                                    RpcResponse {id: resp_id, result: Some(json!({
                                                        "consensus_transaction_id": ctx_id,
                                                        "novavault_operation_id": nova_vault_op_id
                                                    })), error: None}
                                                }
                                                Err(e) => RpcResponse {id:resp_id, result:None, error:Some(RpcError{code:-1,message:format!("Consensus submission error: {}",e)})}
                                            }
                                        }
                                        Err(e) => RpcResponse {id:resp_id, result:None, error:Some(RpcError{code:-2, message:format!("NovaVault processing error: {}",e)})}
                                    }
                                }
                                Err(e) => RpcResponse {id:resp_id, result:None, error:Some(RpcError{code:-32602, message:format!("Invalid params for submit_transaction (expected TransferAucPayload): {}",e)})}
                             }
                        }
                    }
                    "get_balance" => {
                        match serde_json::from_value::<HashMap<String, String>>(rpc_req.params) {
                            Ok(params) => {
                                if let Some(account_pk_hex_str) = params.get("account_pk_hex") {
                                    match novavault_get_balance(account_pk_hex_str) {
                                        Ok(balance) => RpcResponse {id: resp_id, result: Some(json!({"account_pk_hex": account_pk_hex_str, "balance_auc": balance })), error: None},
                                        Err(e) => RpcResponse {id:resp_id, result:None, error:Some(RpcError{code:-3, message:format!("Error getting balance: {}",e)})}
                                    }
                                } else { RpcResponse {id:resp_id, result:None, error:Some(RpcError{code:-32602, message:"Missing 'account_pk_hex' param for get_balance".into()})} }
                            }
                            Err(e) => RpcResponse {id:resp_id, result:None, error:Some(RpcError{code:-32602, message:format!("Invalid params for get_balance: {}", e)})}
                        }
                    }
                    "get_node_state" => {
                        let cs_lock = consensus_state_arc.lock().await;
                        let sync_lock = sync_state_arc.lock().await;
                        let summary = NodeStateSummary {
                            node_name: node_name_handler.clone(), libp2p_peer_id: local_libp2p_peer_id_handler.to_string(),
                            app_layer_pk_hex: node_app_pk_hex.clone(), current_height: cs_lock.current_height,
                            last_block_hash: cs_lock.last_block_hash.clone(), sync_state: format!("{:?}", *sync_lock),
                            is_sequencer: is_sequencer_rpc, known_validators_count: cs_lock.known_validator_pk_hexes.len(),
                            attestation_threshold: cs_lock.attestation_threshold,
                        };
                        RpcResponse {id: resp_id, result: Some(serde_json::to_value(summary).unwrap()), error: None}
                    }
                    "execute_module_call" => {
                        if !matches!(*sync_state_arc.lock().await, NodeSyncState::Synced) {
                             RpcResponse {id: resp_id, result:None, error: Some(RpcError{code:-100, message:"Node not synced".into()})}
                        } else {
                            let params_val = rpc_req.params.clone();
                            let module_id = params_val.get("module_id").and_then(|v| v.as_str()).map(String::from);
                            let function_name = params_val.get("function_name").and_then(|v| v.as_str()).map(String::from);
                            let gas_limit = params_val.get("gas_limit").and_then(|v| v.as_u64()).unwrap_or(1_000_000); 
                            let originator_did_from_rpc = params_val.get("originator_did").and_then(|v| v.as_str()).map(String::from);

                            let wasm_args_res: Result<Vec<WasmiValue>, String> = params_val.get("args_json") // Use WasmiValue
                                .ok_or_else(|| "Missing 'args_json'".to_string())
                                .and_then(|json_val| serde_json::from_value::<Vec<serde_json::Number>>(json_val.clone()) 
                                    .map_err(|e| format!("Failed to parse args_json as numbers: {}", e))
                                    .map(|numbers| numbers.into_iter().map(|n| if n.is_i64() { WasmiValue::I32(n.as_i64().unwrap_or(0) as i32) } else { WasmiValue::F32(WasmiF32::from_float(n.as_f64().unwrap_or(0.0) as f32)) }).collect())
                                );
                                
                            match (module_id, function_name, wasm_args_res) {
                                (Some(mid), Some(fname), Ok(args_vec)) => {
                                    let exec_req = AetherExecutionRequest { 
                                        module_id: mid,
                                        function_name: fname,
                                        arguments: args_vec,
                                        gas_limit,
                                        execution_context_did: originator_did_from_rpc.or_else(|| Some(node_app_pk_hex.clone())),
                                    };
                                    match aethercore_runtime::execute_module(exec_req, current_block_height_for_rpc_ops) {
                                        Ok(exec_res) => {
                                            let result_payload = json!({
                                                "success": exec_res.success,
                                                "output": exec_res.output_values.iter().map(|v| format!("{:?}", v)).collect::<Vec<String>>(),
                                                "gas_consumed": exec_res.gas_consumed_total,
                                                "logs": exec_res.logs,
                                                "error": exec_res.error_message,
                                            });
                                            RpcResponse {id: resp_id, result: Some(result_payload), error: None}
                                        }
                                        Err(e) => RpcResponse {id:resp_id, result:None, error:Some(RpcError{code:-10, message:format!("AetherCore execution error: {}",e)})}
                                    }
                                }
                                (_, _, Err(e)) => RpcResponse {id:resp_id, result:None, error:Some(RpcError{code:-32602, message:format!("Invalid 'args_json': {}",e)})},
                                _ => RpcResponse {id:resp_id, result:None, error:Some(RpcError{code:-32602, message:"Missing module_id or function_name for execute_module_call".into()})}
                            }
                        }
                    }
                    _ => RpcResponse {id:resp_id, result:None, error:Some(RpcError{code:-32601, message:"Method not found".into()})}
                };
                if let Ok(resp_j) = serde_json::to_string(&response) { 
                    if writer.write_all(format!("{}\n", resp_j).as_bytes()).await.is_err() || writer.flush().await.is_err() {
                        error!("[Node:{}:RPC] Send/Flush RPC response error.", node_name_handler); break;
                    }
                } else {
                    error!("[Node:{}:RPC] Serialize RPC response error.", node_name_handler);
                }
            }
            Err(e) => { 
                if e.kind() != io::ErrorKind::UnexpectedEof { error!("[Node:{}:RPC] Read RPC line error: {}", node_name_handler, e); }
                break; 
            }
        }
    }
     debug!("[Node:{}:RPC] RPC connection handler finished.", node_name_handler);
}