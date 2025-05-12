use aethercore_runtime::ExecutionRequest;
use ecliptic_concordance::{Transaction as ConsensusTransaction, Block};
use cosmic_data_constellation::IsnNode;

use novavault_flux_finance::{FinancialOperationType, FinancialOperation};
use celestial_synapse_network_csn as csn;
// Import ZKP structs
use voidproof_engine_zkp::{ZkProofRequest, ZkProof};


use std::collections::HashMap;
use sha2::{Sha256, Digest};
use hex;

fn mock_hash_data<T: std::fmt::Debug>(data: &T) -> String { // Renamed for clarity
    let mut hasher = Sha256::new();
    hasher.update(format!("{:?}", data).as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

fn main() {
    println!("=== Aurora Simulation: Lifecycle with NovaVault, CSN, and ZKP ===");

    // --- Phase 1: User Initiates Operation ---
    let originator = "user_zkp_punk_789";
    // Public part of the payload
    let mut public_payload_details = HashMap::new();
    public_payload_details.insert("to_address_public_key_hash".to_string(), "hash_of_cosmic_789_pk".to_string());
    public_payload_details.insert("amount_display".to_string(), "CONFIDENTIAL".to_string()); // Publicly visible placeholder
    public_payload_details.insert("asset".to_string(), "AUC_PRIVATE".to_string());

    // Private part of the payload (would not typically be passed around like this)
    let private_inputs_data = b"{\"actual_recipient_encrypted_id\":\"enc_cosmic_789\", \"actual_amount_encrypted\": \"enc_150AUC\"}".to_vec();

    let initiated_op = match nebula_pulse_swarm::initiate_operation(
        originator,
        "PrivateTransferAUC_HyperEngine", // More specific op type
        format!("{:?}", public_payload_details).into_bytes(), // Send only public part representation
    ) {
        Ok(op) => op,
        Err(e) => { eprintln!("Error initiating operation: {}", e); return; }
    };
    println!("  -> NebulaPulse: Initiated op: Type '{}', Originator '{}'", initiated_op.operation_type, initiated_op.originator_id);
    let _ = nebula_pulse_swarm::send_data_to_edge(&initiated_op);


    // --- Phase 2: CSN provides dynamic fee ---
    let csn_suggested_fee = csn::get_dynamic_fee_for_novavault("PrivateTransferAUC").unwrap_or(15);
    println!("  -> CSN: Suggested fee for PrivateTransferAUC: {} micro-AUC", csn_suggested_fee);
    let mut full_public_payload_for_novavault = public_payload_details.clone();
    full_public_payload_for_novavault.insert("fee_paid".to_string(), csn_suggested_fee.to_string());


    // --- Phase 3: NovaVault Flux processes, including ZKP generation ---
    let mock_current_block_height_for_nv = 1;
    let financial_op_result: FinancialOperation = match novavault_flux_finance::process_financial_operation(
        originator,
        FinancialOperationType::PrivateTransferAUC, // Use the enum variant
        full_public_payload_for_novavault, // This is what's "public" about the op
        private_inputs_data,        // This is the sensitive data for ZKP
        mock_current_block_height_for_nv
    ) {
        Ok(op_res) => op_res,
        Err(e) => { eprintln!("NovaVault processing error: {}", e); return; }
    };
    println!("  -> NovaVault: Processed financial op: ID '{}', Type: {:?}", financial_op_result.id, financial_op_result.op_type);
    if let Some(ref proof) = financial_op_result.zk_proof {
        println!("     NovaVault obtained ZK Proof ID: '{}'", proof.proof_id);
    }


    // --- Phase 4: AetherCore (Conceptual execution, if needed for sub-operations) ---
    // This step might be less relevant if NovaVault's ZKP handles the core logic's privacy.
    // For now, we'll keep it simple.
    let exec_request = ExecutionRequest {
        module_id: "private_auc_handler_v1".to_string(),
        function_name: "log_private_op_intent".to_string(),
        arguments: initiated_op.data.clone(), // Using initial public part for this log
    };
    if let Ok(execution_result) = aethercore_runtime::execute_module(exec_request) {
        println!("  -> AetherCore (Conceptual): Executed. Success: {}, Output: {:?}",
            execution_result.success, String::from_utf8_lossy(&execution_result.output));
    }


    // --- Phase 5: Ecliptic Concordance for consensus, now with ZKP verification ---
    let op_outcome_for_consensus = format!("{:?}", financial_op_result.payload); // What public state change to agree on
    let op_outcome_hash = mock_hash_data(&op_outcome_for_consensus);

    let consensus_tx: ConsensusTransaction = match ecliptic_concordance::submit_for_consensus(
        op_outcome_hash.clone(),
        financial_op_result.zk_proof.clone() // Pass the ZK proof to consensus
    ) {
        Ok(tx) => tx,
        Err(e) => { eprintln!("Error submitting for consensus: {}", e); return; }
    };
    println!("  -> EclipticConcordance: Submitted for consensus. TxID: '{}', PayloadHash: '{}', ZKP ID: {:?}",
        consensus_tx.id, consensus_tx.payload_hash, consensus_tx.zk_proof_id);

    let finalized_block: Block = match ecliptic_concordance::form_and_finalize_block(vec![consensus_tx.clone()]) {
        Ok(block) => block,
        Err(e) => { eprintln!("Error forming/finalizing block: {}", e); return; }
    };
    println!("  -> EclipticConcordance: Block finalized. ID: '{}', Height: {}", finalized_block.id, finalized_block.height);


    // --- Phase 6: ISN records the consensus-confirmed operation ---
    // The ISN record made by NovaVault is now considered "final" due to block confirmation.
    if let Some(ref isn_node_id_from_nv) = financial_op_result.associated_isn_node_id {
        println!("[ISN_CDC] NovaVault operation (ID: {}) recorded in ISN Node ID: {} is now confirmed by block {}.",
            financial_op_result.id, isn_node_id_from_nv, finalized_block.height);

        if let Some(retrieved_node) = cosmic_data_constellation::get_isn_node(isn_node_id_from_nv) {
            println!("  -> ISN_CDC: Successfully re-retrieved confirmed op record: Properties: {:?}", retrieved_node.properties);
        }
    }

    // --- Phase 7: CSN "observes" ---
    csn::monitor_novavault_activity_patterns();
    println!("  -> CSN: Monitoring of NovaVault patterns initiated.");

    // --- Phase 8: NovaVault queries ISN for a balance ---
    match novavault_flux_finance::get_account_balance(originator, "AUC_PRIVATE") { // Querying for the private asset
        Ok(balance) => println!("  -> NovaVault: Retrieved balance for {} (AUC_PRIVATE): {} (mock from ISN)", originator, balance),
        Err(e) => eprintln!("  -> NovaVault: Error getting balance: {}", e),
    }

    println!("\n=== Simulation with ZKP Complete ===");
}
