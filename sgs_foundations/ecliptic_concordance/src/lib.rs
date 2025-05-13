#![allow(unused_variables, dead_code, unused_imports)]
//! Ecliptic Concordance: Quantum-Resilient Consensus Hyperledger.
use std::sync::Mutex;
use once_cell::sync::Lazy;
use serde::{Serialize, Deserialize};
use uuid::Uuid;
// ZKP parts are not critical for v0.0.1 consensus, can be re-added later
// use voidproof_engine_zkp::{ZkProof, verify_privacy_proof};

// Generic payload for initial testnet transactions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct TransactionPayload {
    pub data: Vec<u8>, // Arbitrary data for now
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConcordanceTransaction { // Renamed to avoid conflict with other Transaction types
    pub id: String,
    pub payload: TransactionPayload, // Using the generic payload
    pub timestamp: u64,
    // pub zk_proof_id: Option<String>, // Can be added back later
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Block {
    pub id: String,
    pub height: u64,
    pub prev_block_hash: String,
    pub transactions: Vec<ConcordanceTransaction>,
    pub timestamp: u64,
    pub proposer_id: String, // DID or Node ID of the block proposer
    // Merkle root can be added later for more robust validation
    // pub merkle_root_hash: String,
    // For now, block_hash will be a simple hash of relevant fields
    pub block_hash: String,
}

impl Block {
    // Simple hash for the block (excluding the block_hash field itself during calculation)
    // In a real system, use a cryptographic hash like SHA256 and serialize fields deterministically.
    pub fn calculate_hash(&self) -> String {
        let mut content = format!("{}{}{}{}", self.id, self.height, self.prev_block_hash, self.timestamp);
        for tx in &self.transactions {
            content.push_str(&tx.id); // Simple concatenation for mock hash
        }
        // In a real system:
        // use sha2::{Sha256, Digest};
        // let mut hasher = Sha256::new();
        // hasher.update(content.as_bytes());
        // hex::encode(hasher.finalize())
        format!("mock_hash_{}", content.len() % 1000) // Very simple mock hash
    }
}


// --- Mock Consensus State & Logic (Single Sequencer for v0.0.1) ---
static CONSENSUS_STATE: Lazy<Mutex<ConsensusState>> = Lazy::new(|| {
    Mutex::new(ConsensusState {
        current_height: 0,
        last_block_hash: "GENESIS_HASH_0.0.1".to_string(),
        pending_transactions: Vec::new(),
    })
});

pub struct ConsensusState {
    pub current_height: u64,
    pub last_block_hash: String,
    pub pending_transactions: Vec<ConcordanceTransaction>,
}

// Function for the designated sequencer node to create a block
pub fn sequencer_create_block(proposer_id: &str) -> Result<Block, String> {
    let mut state = CONSENSUS_STATE.lock().unwrap();
    if state.pending_transactions.is_empty() {
        // Optionally, create empty blocks or wait
        // return Err("No pending transactions to create a block.".to_string());
        println!("[EclipticConcordance] No pending transactions, creating an empty block.");
    }

    state.current_height += 1;
    let block_id = format!("blk_{}", Uuid::new_v4());
    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    
    let mut block = Block {
        id: block_id,
        height: state.current_height,
        prev_block_hash: state.last_block_hash.clone(),
        transactions: std::mem::take(&mut state.pending_transactions), // Takes all pending txs
        timestamp,
        proposer_id: proposer_id.to_string(),
        block_hash: String::new(), // Will be calculated
    };
    block.block_hash = block.calculate_hash(); // Calculate and set the hash

    state.last_block_hash = block.block_hash.clone();
    
    println!("[EclipticConcordance] Sequencer '{}' created Block Height: {}, ID: {}, Hash: {}",
        proposer_id, block.height, block.id, block.block_hash);
    Ok(block)
}

// Function for any node to submit a transaction payload to the (conceptual) mempool
pub fn submit_transaction_payload(payload_data: Vec<u8>) -> Result<String, String> {
    let mut state = CONSENSUS_STATE.lock().unwrap();
    let tx_id = format!("tx_{}", Uuid::new_v4());
    let transaction = ConcordanceTransaction {
        id: tx_id.clone(),
        payload: TransactionPayload { data: payload_data },
        timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
    };
    state.pending_transactions.push(transaction);
    println!("[EclipticConcordance] Submitted transaction payload. Assigned TxID: {}. Pending txs: {}",
        tx_id, state.pending_transactions.len());
    Ok(tx_id)
}

// Function for follower nodes to validate and apply a received block
pub fn validate_and_apply_block(received_block: &Block) -> Result<(), String> {
    let mut state = CONSENSUS_STATE.lock().unwrap();
    println!("[EclipticConcordance] Validating received Block Height: {}, ID: {}",
        received_block.height, received_block.id);

    // Basic validations
    if received_block.height != state.current_height + 1 {
        return Err(format!("Invalid block height. Expected: {}, Got: {}", state.current_height + 1, received_block.height));
    }
    if received_block.prev_block_hash != state.last_block_hash {
        return Err(format!("Invalid previous block hash. Expected: {}, Got: {}", state.last_block_hash, received_block.prev_block_hash));
    }
    // Re-calculate hash to verify (important!)
    let calculated_hash = received_block.calculate_hash();
    if calculated_hash != received_block.block_hash {
         return Err(format!("Block hash mismatch. Calculated: {}, Got: {}", calculated_hash, received_block.block_hash));
    }

    // If valid, update local state
    state.current_height = received_block.height;
    state.last_block_hash = received_block.block_hash.clone();
    // In a real system, you'd also process transactions here and update application state
    // For now, just log them.
    for tx in &received_block.transactions {
        println!("[EclipticConcordance] Applying Tx ID: {} from received block.", tx.id);
    }
    // Remove these transactions if they were in our local pending_transactions (simple approach)
    state.pending_transactions.retain(|ptx| !received_block.transactions.iter().any(|btx| btx.id == ptx.id));

    println!("[EclipticConcordance] Successfully validated and applied Block Height: {}. New last hash: {}",
        state.current_height, state.last_block_hash);
    Ok(())
}

// Helper to get current consensus state (for querying)
pub fn get_current_state_summary() -> (u64, String) {
    let state = CONSENSUS_STATE.lock().unwrap();
    (state.current_height, state.last_block_hash.clone())
}

pub fn status() -> &'static str { /* ... same ... */
    let crate_name = "ecliptic_concordance";
    println!("[{}] placeholder_function called (mock status)", crate_name);
    "skeleton operational (mock)"
}
