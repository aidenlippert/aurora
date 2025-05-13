#![allow(unused_variables, dead_code, unused_imports)]
//! Ecliptic Concordance: Quantum-Resilient Consensus Hyperledger.
// Removed: use std::sync::Mutex;
// Removed: use once_cell::sync::Lazy;
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use std::collections::VecDeque; // For a simpler mempool queue

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
}

impl Block {
    pub fn calculate_hash(&self) -> String {
        let mut content = format!("{}{}{}{}", self.id, self.height, self.prev_block_hash, self.timestamp);
        for tx in &self.transactions {
            content.push_str(&tx.id);
        }
        format!("mock_hash_{}", content.len() % 1000)
    }
}

// Node-local consensus state
#[derive(Debug, Clone)] // Clone needed if nodes might fork and want to explore alternatives (advanced)
pub struct ConsensusState {
    pub node_id: String, // For logging/identification
    pub current_height: u64,
    pub last_block_hash: String,
    pub pending_transactions: VecDeque<ConcordanceTransaction>, // Using VecDeque as a simple FIFO queue
}

impl ConsensusState {
    pub fn new(node_id: String) -> Self {
        ConsensusState {
            node_id,
            current_height: 0,
            last_block_hash: "GENESIS_HASH_0.0.1".to_string(),
            pending_transactions: VecDeque::new(),
        }
    }
    // Method to load from disk could be added here, taking file path
}

// Functions now take &mut ConsensusState
pub fn sequencer_create_block(state: &mut ConsensusState, proposer_id: &str) -> Result<Block, String> {
    if state.pending_transactions.is_empty() {
        println!("[EclipticConcordance:{}] No pending transactions, creating an empty block.", state.node_id);
    }

    state.current_height += 1;
    let block_id = format!("blk_{}", Uuid::new_v4());
    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    
    let transactions_for_block: Vec<ConcordanceTransaction> = state.pending_transactions.drain(..).collect();

    let mut block = Block {
        id: block_id,
        height: state.current_height,
        prev_block_hash: state.last_block_hash.clone(),
        transactions: transactions_for_block,
        timestamp,
        proposer_id: proposer_id.to_string(),
        block_hash: String::new(),
    };
    block.block_hash = block.calculate_hash();

    state.last_block_hash = block.block_hash.clone();
    
    println!("[EclipticConcordance:{}] Sequencer '{}' created Block Height: {}, ID: {}, Hash: {}",
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
    state.pending_transactions.push_back(transaction); // Add to VecDeque
    println!("[EclipticConcordance:{}] Submitted transaction payload. Assigned TxID: {}. Pending txs: {}",
        state.node_id, tx_id, state.pending_transactions.len());
    Ok(tx_id)
}

pub fn validate_and_apply_block(state: &mut ConsensusState, received_block: &Block) -> Result<(), String> {
    println!("[EclipticConcordance:{}] Validating received Block Height: {}, ID: {}",
        state.node_id, received_block.height, received_block.id);

    if received_block.height != state.current_height + 1 {
        return Err(format!("Invalid block height. Node_Height: {}, Expected_Next: {}, Got: {}", state.current_height, state.current_height + 1, received_block.height));
    }
    if received_block.prev_block_hash != state.last_block_hash {
        return Err(format!("Invalid previous block hash. Node_LastHash: {}, Expected_Prev: {}, Got: {}", state.last_block_hash, state.last_block_hash, received_block.prev_block_hash));
    }
    let calculated_hash = received_block.calculate_hash();
    if calculated_hash != received_block.block_hash {
         return Err(format!("Block hash mismatch. Calculated: {}, Got: {}", calculated_hash, received_block.block_hash));
    }

    state.current_height = received_block.height;
    state.last_block_hash = received_block.block_hash.clone();
    for tx in &received_block.transactions {
        println!("[EclipticConcordance:{}] Applying Tx ID: {} from received block.", state.node_id, tx.id);
    }
    state.pending_transactions.retain(|ptx| !received_block.transactions.iter().any(|btx| btx.id == ptx.id));

    println!("[EclipticConcordance:{}] Successfully validated and applied Block Height: {}. New last hash: {}",
        state.node_id, state.current_height, state.last_block_hash);
    Ok(())
}

// Helper to get current consensus state summary from a given state object
pub fn get_current_state_summary(state: &ConsensusState) -> (u64, String) {
    (state.current_height, state.last_block_hash.clone())
}

pub fn status() -> &'static str {
    // This static status doesn't reflect per-node state anymore.
    // Could be removed or changed to a generic message.
    "EclipticConcordance Logic Module"
}
