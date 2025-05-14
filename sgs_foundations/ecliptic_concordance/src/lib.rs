// sgs_foundations/ecliptic_concordance/src/lib.rs
#![allow(unused_variables, dead_code, unused_imports)]

use serde::{Serialize, Deserialize};
use uuid::Uuid;
use std::collections::{VecDeque, HashMap, HashSet};
use log::{info, debug, trace, warn, error};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct TransactionPayload {
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConcordanceTransaction {
    pub id: String,
    pub payload: TransactionPayload,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Block {
    pub id: String,
    pub height: u64,
    pub prev_block_hash: String,
    pub transactions: Vec<ConcordanceTransaction>,
    pub timestamp: u64,
    pub proposer_id: String,
    pub block_hash: String,
    #[serde(default)] 
    pub is_confirmed_by_attestations: bool,
}

impl Block {
    pub fn calculate_hash(&self) -> String {
        let mut content = format!("{}{}{}{}", self.id, self.height, self.prev_block_hash, self.timestamp);
        for tx in &self.transactions {
            content.push_str(&tx.id);
        }
        content.push_str(&self.proposer_id);
        format!("mock_hash_{}", content.len() % 1000)
    }
}

#[derive(Debug, Clone)]
pub struct ConsensusState {
    pub node_id: String,
    pub current_height: u64,
    pub last_block_hash: String,
    pub pending_transactions: VecDeque<ConcordanceTransaction>,
    pub block_attestations: HashMap<String /* block_hash */, HashSet<String /* attestor_peer_id_str */>>,
    pub known_validator_peer_ids: Vec<String>,
    pub attestation_threshold: usize,
}

impl ConsensusState {
    pub fn new(node_id: String) -> Self {
        ConsensusState {
            node_id,
            current_height: 0,
            last_block_hash: "GENESIS_HASH_0.0.1".to_string(),
            pending_transactions: VecDeque::new(),
            block_attestations: HashMap::new(),
            known_validator_peer_ids: vec![],
            attestation_threshold: 1,
        }
    }

    pub fn set_validators(&mut self, validator_ids: Vec<String>, threshold: usize) {
        self.known_validator_peer_ids = validator_ids;
        if !self.known_validator_peer_ids.is_empty() {
            self.attestation_threshold = threshold.min(self.known_validator_peer_ids.len()).max(1);
        } else {
            self.attestation_threshold = 0;
        }
        info!("[EclipticConcordance:{}] Validators set: {:?}, Threshold: {}", self.node_id, self.known_validator_peer_ids, self.attestation_threshold);
    }
}

pub fn sequencer_create_block(state: &mut ConsensusState, proposer_id: &str) -> Result<Block, String> {
    if state.pending_transactions.is_empty() {
        trace!("[EclipticConcordance:{}] No pending transactions, creating an empty block.", state.node_id);
    }

    let new_height = if state.current_height == 0 && state.last_block_hash == "GENESIS_HASH_0.0.1" {
        0
    } else {
        state.current_height + 1
    };

    let block_id = format!("blk_{}", Uuid::new_v4());
    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    
    let transactions_for_block: Vec<ConcordanceTransaction> = state.pending_transactions.drain(..).collect();

    let mut block = Block {
        id: block_id,
        height: new_height,
        prev_block_hash: state.last_block_hash.clone(),
        transactions: transactions_for_block,
        timestamp,
        proposer_id: proposer_id.to_string(),
        block_hash: String::new(),
        is_confirmed_by_attestations: false,
    };
    block.block_hash = block.calculate_hash();

    state.current_height = block.height;
    state.last_block_hash = block.block_hash.clone();
    state.block_attestations.retain(|bh, _| bh == &block.block_hash);
    
    info!("[EclipticConcordance:{}] Sequencer '{}' created Block Height: {}, ID: {}, Hash: {}",
        state.node_id, proposer_id, block.height, block.id, block.block_hash);
    Ok(block)
}

pub fn submit_transaction_payload(state: &mut ConsensusState, payload_data: Vec<u8>) -> Result<String, String> {
    let tx_id = format!("tx_{}", Uuid::new_v4());
    let transaction = ConcordanceTransaction {
        id: tx_id.clone(),
        payload: TransactionPayload { data: payload_data },
        timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
    };
    state.pending_transactions.push_back(transaction);
    info!("[EclipticConcordance:{}] Submitted transaction payload. Assigned TxID: {}. Pending txs: {}",
        state.node_id, tx_id, state.pending_transactions.len());
    Ok(tx_id)
}

pub fn validate_and_apply_block(state: &mut ConsensusState, received_block: &Block) -> Result<(), String> {
    debug!("[EclipticConcordance:{}] Validating received Block H:{}, ID:{}, PrevHash:{:.8}, OwnHash:{:.8}. Current state H:{}, LastHash:{:.8}",
        state.node_id, received_block.height, received_block.id, received_block.prev_block_hash, received_block.block_hash, state.current_height, state.last_block_hash);

    if received_block.height == 0 && state.current_height == 0 && state.last_block_hash == "GENESIS_HASH_0.0.1" {
        debug!("[EclipticConcordance:{}] Applying Block H:0 as first block after genesis.", state.node_id);
    } else if received_block.height != state.current_height + 1 {
        let err_msg = format!("Invalid block height. Node_Current_H: {}, Expected_Next: {}, Got_Block_H: {}", state.current_height, state.current_height + 1, received_block.height);
        error!("[EclipticConcordance:{}] {}", state.node_id, err_msg);
        return Err(err_msg);
    } else if received_block.prev_block_hash != state.last_block_hash {
        let err_msg = format!("Invalid previous block hash for H:{}. Node_LastHash: {}, Block_PrevHash: {}",  received_block.height, state.last_block_hash, received_block.prev_block_hash);
        error!("[EclipticConcordance:{}] {}", state.node_id, err_msg);
        return Err(err_msg);
    }
    
    let calculated_hash = received_block.calculate_hash();
    if calculated_hash != received_block.block_hash {
        let err_msg = format!("Block hash mismatch for H:{}. Calculated: {}, Block_OwnHash: {}", received_block.height, calculated_hash, received_block.block_hash);
        error!("[EclipticConcordance:{}] {}", state.node_id, err_msg);
        return Err(err_msg);
    }

    state.current_height = received_block.height;
    state.last_block_hash = received_block.block_hash.clone();
    
    state.pending_transactions.retain(|ptx| !received_block.transactions.iter().any(|btx| btx.id == ptx.id));

    info!("[EclipticConcordance:{}] Successfully validated and applied Block Height: {}. New last hash: {:.8}",
        state.node_id, state.current_height, state.last_block_hash);
    Ok(())
}

pub fn process_incoming_attestation(
    state: &mut ConsensusState,
    block_height: u64,
    block_hash: &str,
    attestor_peer_id_str: &str,
) -> bool {
    if state.known_validator_peer_ids.is_empty() && state.attestation_threshold == 0 {
        trace!("[EclipticConcordance:{}] No validators configured, attestations have no effect for H:{}, Hash:{:.8}", state.node_id, block_height, block_hash);
        return false; 
    }
    if !state.known_validator_peer_ids.contains(&attestor_peer_id_str.to_string()) {
        trace!("[EclipticConcordance:{}] Attestation from unknown peer {} (not in known_validator_peer_ids: {:?}) for H:{}, Hash:{:.8}, ignoring.",
            state.node_id, attestor_peer_id_str, state.known_validator_peer_ids, block_height, block_hash);
        return false;
    }
    
    debug!("[EclipticConcordance:{}] Received attestation for H:{} Hash:{:.8} from Peer:{}",
        state.node_id, block_height, block_hash, attestor_peer_id_str);

    let attestors = state.block_attestations.entry(block_hash.to_string()).or_insert_with(HashSet::new);
    let new_attestation = attestors.insert(attestor_peer_id_str.to_string());

    if new_attestation {
        info!("[EclipticConcordance:{}] Attestation for H:{} Hash:{:.8} added. Total for this block: {}/{} (Known Validators: {})",
            state.node_id, block_height, block_hash, attestors.len(), state.attestation_threshold, state.known_validator_peer_ids.len());
        if attestors.len() >= state.attestation_threshold {
            info!("[EclipticConcordance:{}] Block H:{} Hash:{:.8} CONFIRMED by {} attestations (threshold: {}).",
                state.node_id, block_height, block_hash, attestors.len(), state.attestation_threshold);
            return true;
        }
    } else {
        trace!("[EclipticConcordance:{}] Duplicate attestation for H:{} Hash:{:.8} from Peer:{}, ignoring.",
            state.node_id, block_height, block_hash, attestor_peer_id_str);
    }
    false
}

pub fn get_current_state_summary(state: &ConsensusState) -> (u64, String) {
    (state.current_height, state.last_block_hash.clone())
}

pub fn status() -> &'static str {
    "EclipticConcordance Logic Module (with basic attestation tracking)"
}