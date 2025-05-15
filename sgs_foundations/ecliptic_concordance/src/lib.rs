// sgs_foundations/ecliptic_concordance/src/lib.rs
#![allow(unused_variables, dead_code, unused_imports)]

use serde::{Serialize, Deserialize};
use uuid::Uuid;
use std::collections::{VecDeque, HashMap, HashSet};
use log::{info, debug, trace, warn, error};
use sha2::{Sha256, Digest};
use hex; 
use ed25519_dalek::{
    Signature, Signer, Verifier, SigningKey, VerifyingKey, 
    PUBLIC_KEY_LENGTH, SIGNATURE_LENGTH
};

// NEW: Define structured transaction types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct TransferAucPayload {
    pub sender_pk_hex: String, // Sender's app layer public key hex
    pub recipient_pk_hex: String,
    pub amount: u64,
    pub nonce: u64, // Simple replay protection
    // signature will be on the ConcordanceTransaction itself later
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum AuroraTransaction {
    TransferAUC(TransferAucPayload),
    // Future types: CallWasm { module_id: String, function: String, args: Vec<u8> },
    //               RegisterDID { did_doc: String },
}

// UPDATED: ConcordanceTransaction now holds an AuroraTransaction
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConcordanceTransaction {
    pub id: String, // Unique ID for this consensus-level transaction wrapper
    pub payload: AuroraTransaction, // The actual application-level transaction
    pub timestamp: u64,
    // TODO: Add overall transaction signature by the originator of AuroraTransaction
    // For TransferAUC, this would be from sender_pk_hex's private key.
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Block {
    pub id: String,
    pub height: u64,
    pub prev_block_hash: String,
    pub transactions: Vec<ConcordanceTransaction>, // Now holds Vec<AuroraTransaction>
    pub timestamp: u64,
    #[serde(with = "hex::serde")] 
    pub proposer_pk_bytes: Vec<u8>, 
    pub block_hash: String,
    #[serde(with = "hex::serde")] 
    pub proposer_signature: [u8; SIGNATURE_LENGTH],
    #[serde(default)]
    pub is_confirmed_by_attestations: bool,
}

impl Block {
    pub fn proposer_pk_hex(&self) -> String { 
        hex::encode(&self.proposer_pk_bytes)
    }

    pub fn calculate_content_hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.id.as_bytes());
        hasher.update(&self.height.to_le_bytes());
        hasher.update(self.prev_block_hash.as_bytes());
        for tx in &self.transactions {
            hasher.update(tx.id.as_bytes());
            // For more robust hashing, serialize tx.payload and hash that
            // For now, tx.id representing the whole wrapped tx is okay for structure
        }
        hasher.update(&self.timestamp.to_le_bytes());
        hasher.update(&self.proposer_pk_bytes); 
        hex::encode(hasher.finalize())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Attestation {
    pub block_hash: String,
    pub block_height: u64,
    #[serde(with = "hex::serde")] 
    pub attestor_pk_bytes: Vec<u8>, 
    #[serde(with = "hex::serde")] 
    pub signature: [u8; SIGNATURE_LENGTH],
}

impl Attestation {
    pub fn attestor_pk_hex(&self) -> String { 
        hex::encode(&self.attestor_pk_bytes)
    }
    pub fn message_to_sign(block_height: u64, block_hash: &str) -> Vec<u8> {
        format!("ATT:{}:{}", block_height, block_hash).into_bytes()
    }
}

#[derive(Debug, Clone)]
pub struct ConsensusState {
    pub node_log_id: String,
    pub current_height: u64,
    pub last_block_hash: String,
    pub pending_transactions: VecDeque<ConcordanceTransaction>, // UPDATED Type
    pub block_attestations: HashMap<String, HashMap<String, Attestation>>,
    pub known_validator_pk_hexes: HashSet<String>, 
    pub attestation_threshold: usize,
}

impl ConsensusState {
    pub fn new(node_log_id: String) -> Self {
        ConsensusState {
            node_log_id,
            current_height: 0,
            last_block_hash: "GENESIS_HASH_0.0.1".to_string(),
            pending_transactions: VecDeque::new(),
            block_attestations: HashMap::new(),
            known_validator_pk_hexes: HashSet::new(),
            attestation_threshold: 1,
        }
    }
    pub fn set_validators(&mut self, validator_pk_hexes: HashSet<String>, threshold: usize) {
        self.known_validator_pk_hexes = validator_pk_hexes;
        if !self.known_validator_pk_hexes.is_empty() {
            self.attestation_threshold = threshold.min(self.known_validator_pk_hexes.len()).max(1);
        } else { self.attestation_threshold = 0; }
        info!("[EclipticConcordance:{}] Validators set. Count: {}, Threshold: {}", 
            self.node_log_id, self.known_validator_pk_hexes.len(), self.attestation_threshold);
    }
}

pub fn sequencer_create_block(
    state: &mut ConsensusState,
    proposer_signing_key: &SigningKey,
) -> Result<Block, String> {
    if state.pending_transactions.is_empty() && !(state.current_height == 0 && state.last_block_hash == "GENESIS_HASH_0.0.1") {
        trace!("[EclipticConcordance:{}] No pending transactions for block.", state.node_log_id);
    }
    let new_height = if state.current_height == 0 && state.last_block_hash == "GENESIS_HASH_0.0.1" { 0 } else { state.current_height + 1 };
    let block_id = format!("blk_{}", Uuid::new_v4());
    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let transactions_for_block: Vec<ConcordanceTransaction> = state.pending_transactions.drain(..).collect();
    let proposer_pk_bytes = proposer_signing_key.verifying_key().to_bytes().to_vec();

    let mut partial_block = Block {
        id: block_id, height: new_height, prev_block_hash: state.last_block_hash.clone(),
        transactions: transactions_for_block, timestamp, proposer_pk_bytes,
        block_hash: String::new(), proposer_signature: [0u8; SIGNATURE_LENGTH],
        is_confirmed_by_attestations: false,
    };
    let content_hash = partial_block.calculate_content_hash();
    partial_block.block_hash = content_hash.clone();
    let signature = proposer_signing_key.sign(content_hash.as_bytes());
    partial_block.proposer_signature.copy_from_slice(signature.to_bytes().as_slice());

    state.current_height = partial_block.height;
    state.last_block_hash = partial_block.block_hash.clone();
    info!("[EclipticConcordance:{}] Proposer (PK_hex:{:.8}) created Block H:{}, Hash:{:.8}, Txs:{}",
        state.node_log_id, partial_block.proposer_pk_hex(), partial_block.height, partial_block.block_hash, partial_block.transactions.len());
    Ok(partial_block)
}

// UPDATED: submit_transaction_payload now takes an AuroraTransaction
pub fn submit_aurora_transaction(state: &mut ConsensusState, aurora_tx: AuroraTransaction) -> Result<String, String> {
    let tx_id = format!("ctx_{}", Uuid::new_v4()); // Consensus Transaction ID
    let concordance_tx = ConcordanceTransaction {
        id: tx_id.clone(),
        payload: aurora_tx,
        timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
    };
    state.pending_transactions.push_back(concordance_tx);
    info!("[EclipticConcordance:{}] Submitted Aurora Tx. Wrapped in CTxID: {}. Pending: {}",
        state.node_log_id, tx_id, state.pending_transactions.len());
    Ok(tx_id)
}

pub fn validate_and_apply_block(state: &mut ConsensusState, received_block: &Block) -> Result<(), String> {
    // ... (validation logic from previous version, no major changes here for this step)
    // ... ensure it iterates through received_block.transactions and potentially validates them
    // ... or calls out to HyperEngines (like NovaVault) for validation if needed.
    // For now, we assume transactions in a signed block are valid if the block signature is valid.
    // The actual execution/state change from transactions will happen on the node applying the block.
    debug!("[EclipticConcordance:{}] Validating Block H:{}, PropPK_hex:{:.8}, PrevH:{:.8}, OwnH:{:.8}. Current H:{}, LastH:{:.8}",
        state.node_log_id, received_block.height, received_block.proposer_pk_hex(), 
        received_block.prev_block_hash, received_block.block_hash, 
        state.current_height, state.last_block_hash);

    if received_block.height == 0 && state.current_height == 0 && state.last_block_hash == "GENESIS_HASH_0.0.1" {
        if received_block.prev_block_hash != "GENESIS_HASH_0.0.1" {
             return Err(format!("Genesis block H:0 has incorrect prev_block_hash: {}", received_block.prev_block_hash));
        }
        debug!("[EclipticConcordance:{}] Applying Block H:0 as first block.", state.node_log_id);
    } else if received_block.height != state.current_height + 1 {
        return Err(format!("Invalid block height. Expected:{}, Got:{}", state.current_height + 1, received_block.height));
    } else if received_block.prev_block_hash != state.last_block_hash {
        return Err(format!("Invalid prev_block_hash. Expected:{:.8}, Got:{:.8}", state.last_block_hash, received_block.prev_block_hash));
    }
    
    let calculated_content_hash = received_block.calculate_content_hash();
    if calculated_content_hash != received_block.block_hash {
        return Err(format!("Block content hash mismatch. Calculated:{}, Provided:{}", calculated_content_hash, received_block.block_hash));
    }
    
    if received_block.proposer_pk_bytes.len() != PUBLIC_KEY_LENGTH {
        return Err(format!("Proposer PK bytes have wrong length: expected {}, got {}", PUBLIC_KEY_LENGTH, received_block.proposer_pk_bytes.len()));
    }
    let mut proposer_pk_array = [0u8; PUBLIC_KEY_LENGTH];
    proposer_pk_array.copy_from_slice(&received_block.proposer_pk_bytes);

    let verifying_key = VerifyingKey::from_bytes(&proposer_pk_array)
        .map_err(|e| format!("Failed to create VerifyingKey from proposer_pk_bytes: {}", e))?;
    
    let signature = Signature::from_bytes(&received_block.proposer_signature);

    if verifying_key.verify(received_block.block_hash.as_bytes(), &signature).is_err() {
        return Err(format!("Invalid block signature for H:{}", received_block.height));
    }
    debug!("[EclipticConcordance:{}] Block H:{} signature VERIFIED from PK_hex:{:.8}",
        state.node_log_id, received_block.height, received_block.proposer_pk_hex());

    // If block is valid, now "execute" its transactions against local state (e.g. NovaVault)
    // This is where each node independently processes the transactions in the block.
    // For this step, we'll assume this happens in the node's main loop *after* calling this.
    // Here, we just update the consensus state.
    state.current_height = received_block.height;
    state.last_block_hash = received_block.block_hash.clone();
    state.pending_transactions.retain(|ptx| !received_block.transactions.iter().any(|btx| btx.id == ptx.id));
    state.block_attestations.entry(received_block.block_hash.clone()).or_default();

    info!("[EclipticConcordance:{}] Applied Block H:{}. New last hash: {:.8}. Tx count: {}",
        state.node_log_id, state.current_height, state.last_block_hash, received_block.transactions.len());
    Ok(())
}

pub fn create_attestation(
    block_height: u64,
    block_hash: &str,
    attestor_signing_key: &SigningKey,
) -> Result<Attestation, String> {
    // ... (no changes from previous correct version)
    let attestor_pk_bytes = attestor_signing_key.verifying_key().to_bytes().to_vec();
    let message_to_sign = Attestation::message_to_sign(block_height, block_hash);
    let signature = attestor_signing_key.sign(&message_to_sign);
    Ok(Attestation { block_hash: block_hash.to_string(), block_height, attestor_pk_bytes, signature: signature.to_bytes() })
}

pub fn process_incoming_attestation(
    state: &mut ConsensusState,
    attestation: &Attestation,
) -> bool {
    // ... (no changes from previous correct version)
    let attestor_pk_hex_for_check = attestation.attestor_pk_hex();
    if state.known_validator_pk_hexes.is_empty() && state.attestation_threshold == 0 { return false; }
    if !state.known_validator_pk_hexes.contains(&attestor_pk_hex_for_check) { return false; }
    if attestation.attestor_pk_bytes.len() != PUBLIC_KEY_LENGTH { return false; }
    let mut attestor_pk_array = [0u8; PUBLIC_KEY_LENGTH];
    attestor_pk_array.copy_from_slice(&attestation.attestor_pk_bytes);
    let verifying_key = match VerifyingKey::from_bytes(&attestor_pk_array) { Ok(k) => k, Err(_) => return false };
    let message_signed = Attestation::message_to_sign(attestation.block_height, &attestation.block_hash);
    let signature = Signature::from_bytes(&attestation.signature);
    if verifying_key.verify(&message_signed, &signature).is_err() { return false; }
    debug!("[EclipticConcordance:{}] Attestation sig VERIFIED for H:{} Hash:{:.8} from PK_hex:{:.8}",
         state.node_log_id, attestation.block_height, attestation.block_hash, attestor_pk_hex_for_check);
    let attestations_for_this_block = state.block_attestations.entry(attestation.block_hash.clone()).or_default();
    if attestations_for_this_block.contains_key(&attestor_pk_hex_for_check) { return false; }
    attestations_for_this_block.insert(attestor_pk_hex_for_check.clone(), attestation.clone());
    let current_att_count = attestations_for_this_block.len();
    info!("[EclipticConcordance:{}] Attestation for H:{} Hash:{:.8} from PK_hex:{:.8} added. Count: {}/{}",
        state.node_log_id, attestation.block_height, attestation.block_hash, attestor_pk_hex_for_check,
        current_att_count, state.attestation_threshold);
    if current_att_count >= state.attestation_threshold {
        info!("[EclipticConcordance:{}] Block H:{} Hash:{:.8} CONFIRMED by {} attestations (threshold: {}).",
            state.node_log_id, attestation.block_height, attestation.block_hash, current_att_count, state.attestation_threshold);
        return true;
    }
    false
}

pub fn get_current_state_summary(state: &ConsensusState) -> (u64, String) {
    (state.current_height, state.last_block_hash.clone())
}

pub fn status() -> &'static str {
    "EclipticConcordance: Structured AuroraTransactions, Signed Blocks/Attestations"
}