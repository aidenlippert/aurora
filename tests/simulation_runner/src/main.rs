use aethercore_runtime::ExecutionRequest;
use ecliptic_concordance::{Transaction as ConsensusTransaction, Block};
use cosmic_data_constellation::IsnNode; // Used for type annotation

// Import from new HyperEngine and CSN
use novavault_flux_finance::{FinancialOperationType, FinancialOperation};
use celestial_synapse_network_csn as csn; // Alias for brevity

use std::collections::HashMap;
use sha2::{Sha256, Digest};
use hex;

fn mock_hash_struct<T: std::fmt::Debug>(data: &T) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{:?}", data).as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

fn main() {
    println!("=== Aurora Enhanced Lifecycle Simulation (with NovaVault & CSN) ===");

    // --- Phase 1: User Initiates Operation via NebulaPulse ---
    let originator = "user_punk_123";
    let operation_data_payload = b"{\"recipient\":\"user_cosmic_456\", \"amount\":100, \"asset\":\"AUC\"}".to_vec();
    let initiated_op = match nebula_pulse_swarm::initiate_operation(originator, "TransferAUC_HyperEngine", operation_data_payload.clone()) {
        Ok(op) => op,
        Err(e) => { eprintln!("Error initiating operation: {}", e); return; }
    };
    println!("  -> NebulaPulse: Initiated op: Type '{}', Originator '{}'", initiated_op.operation_type, initiated_op.originator_id);
    let _ = nebula_pulse_swarm::send_data_to_edge(&initiated_op); // Ignoring edge_proc_id for now

    // --- Phase 2: CSN provides dynamic fee suggestion (conceptual) ---
    let csn_suggested_fee = match csn::get_dynamic_fee_for_novavault("TransferAUC") {
        Ok(fee) => fee,
        Err(e) => { eprintln!("Error getting CSN fee: {}", e); 0 } // Default if CSN fails
    };
    println!("  -> CSN: Suggested fee for TransferAUC: {} micro-AUC", csn_suggested_fee);


    // --- Phase 3: NovaVault Flux (HyperEngine) processes the financial operation ---
    // This would typically happen after consensus on the intent, or NovaVault itself
    // would use AetherCore for its internal Wasm contracts.
    // For this simulation, we'll assume NovaVault is invoked and will later get its state confirmed.

    let mut nv_payload = HashMap::new();
    nv_payload.insert("to_address".to_string(), "user_cosmic_456".to_string());
    nv_payload.insert("amount".to_string(), "100".to_string());
    nv_payload.insert("asset".to_string(), "AUC".to_string());
    nv_payload.insert("fee_paid".to_string(), csn_suggested_fee.to_string()); // Include CSN fee

    // We need a mock block height for ISN recording. Let's assume a block is about to be formed.
    // In a real flow, consensus would happen *after* execution for state changes.
    // Here, we'll simulate consensus on the *result* of NovaVault's processing.
    let mock_current_block_height_for_nv = 1; // Simulating this for ISN recording within NovaVault

    let financial_op_result: FinancialOperation = match novavault_flux_finance::process_financial_operation(
        originator,
        FinancialOperationType::TransferAUC,
        nv_payload,
        mock_current_block_height_for_nv // This implies ISN recording happens post-consensus usually
    ) {
        Ok(op_res) => op_res,
        Err(e) => { eprintln!("NovaVault processing error: {}", e); return; }
    };
    println!("  -> NovaVault: Processed financial op: ID '{}', Type: {:?}, ISN Node: {:?}",
        financial_op_result.id,
        financial_op_result.op_type,
        financial_op_result.associated_isn_node_id
    );


    // --- Phase 4: AetherCore "executes" a conceptual part of NovaVault or a related contract ---
    // This could be a logging contract, or a sub-operation.
    let exec_request = ExecutionRequest {
        module_id: "mock_contract_v1".to_string(), // Could be a NovaVault specific module
        function_name: "process_payment".to_string(), // Or a more generic function
        arguments: operation_data_payload, // Using original payload for this mock AetherCore call
    };
    let execution_result = match aethercore_runtime::execute_module(exec_request) {
        Ok(res) => res,
        Err(e) => { eprintln!("AetherCore execution error: {}", e); return; }
    };
     println!("  -> AetherCore (Post-NovaVault concept): Executed. Success: {}, Output: {:?}",
        execution_result.success, String::from_utf8_lossy(&execution_result.output)
    );


    // --- Phase 5: Ecliptic Concordance for the overall confirmed state change ---
    // Hash of the NovaVault operation result (or its ISN node ID) for consensus
    let nv_op_result_hash = mock_hash_struct(&financial_op_result);

    let consensus_tx: ConsensusTransaction = match ecliptic_concordance::submit_for_consensus(nv_op_result_hash.clone()) {
        Ok(tx) => tx,
        Err(e) => { eprintln!("Error submitting for consensus: {}", e); return; }
    };
    println!("  -> EclipticConcordance: Submitted for consensus. TxID: '{}', PayloadHash: '{}'", consensus_tx.id, consensus_tx.payload_hash);

    let finalized_block: Block = match ecliptic_concordance::form_and_finalize_block(vec![consensus_tx.clone()]) {
        Ok(block) => block,
        Err(e) => { eprintln!("Error forming/finalizing block: {}", e); return; }
    };
    println!("  -> EclipticConcordance: Block finalized. ID: '{}', Height: {}", finalized_block.id, finalized_block.height);

    // --- Phase 6: ISN records the consensus-confirmed NovaVault operation ---
    // This might be redundant if NovaVault already recorded and that record is part of what's confirmed,
    // or it could be a separate higher-level confirmation record.
    // For simulation, we'll use the ISN node ID from NovaVault's own recording.
    if let Some(ref isn_node_id_from_nv) = financial_op_result.associated_isn_node_id {
        println!("[ISN_CDC] NovaVault operation (ID: {}) was recorded under ISN Node ID: {}. This is now confirmed by block {}.",
            financial_op_result.id, isn_node_id_from_nv, finalized_block.height);

        if let Some(retrieved_node) = cosmic_data_constellation::get_isn_node(isn_node_id_from_nv) {
            println!("  -> ISN_CDC: Successfully re-retrieved confirmed NovaVault op record: {:?}", retrieved_node);
        }
    } else {
        println!("[ISN_CDC] NovaVault operation did not return an ISN node ID for confirmation.");
    }

    // --- Phase 7: CSN "observes" NovaVault activity (conceptual) ---
    csn::monitor_novavault_activity_patterns();
    println!("  -> CSN: Monitoring of NovaVault patterns initiated.");


    // --- Phase 8: NovaVault queries ISN for a balance (example of ISN read) ---
    match novavault_flux_finance::get_account_balance(originator, "AUC") {
        Ok(balance) => println!("  -> NovaVault: Retrieved balance for {} (AUC): {} (mock from ISN)", originator, balance),
        Err(e) => eprintln!("  -> NovaVault: Error getting balance: {}", e),
    }


    println!("\n=== Enhanced Simulation Complete ===");
}
