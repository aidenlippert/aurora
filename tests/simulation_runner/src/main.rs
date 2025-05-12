use aethercore_runtime::ExecutionRequest; // Keep this as it's used
use ecliptic_concordance::{Transaction as ConsensusTransaction, Block};
// cosmic_data_constellation::IsnNode is not directly used by name in main, can be removed if desired
// use cosmic_data_constellation::IsnNode;

use novavault_flux_finance::{FinancialOperationType as NovaVaultOpType, FinancialOperation}; // Aliased to avoid naming clash
use celestial_synapse_network_csn as csn;
use voidproof_engine_zkp::ZkProof; // ZkProof is used for Option<ZkProof> type hint

// Governance imports
use starsenate_collectives_governance::{Proposal, ProposalStatus, submit_proposal, cast_vote_on_proposal, tally_votes_and_decide};
use oraclesync_futarchy; // We call functions on this module directly

use std::collections::HashMap;
use sha2::{Sha256, Digest};
use hex;

fn mock_hash_data<T: std::fmt::Debug>(data: &T) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{:?}", data).as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

fn run_financial_simulation_phase() {
    println!("\n--- Running Financial Simulation Phase ---");
    let originator = "user_zkp_punk_789";
    let public_payload_details = {
        let mut map = HashMap::new();
        map.insert("to_address_public_key_hash".to_string(), "hash_of_cosmic_789_pk".to_string());
        map.insert("amount_display".to_string(), "CONFIDENTIAL".to_string());
        map.insert("asset".to_string(), "AUC_PRIVATE".to_string());
        map
    };
    let private_inputs_data = b"{\"actual_recipient_encrypted_id\":\"enc_cosmic_789\", \"actual_amount_encrypted\": \"enc_150AUC\"}".to_vec();

    let initiated_op = nebula_pulse_swarm::initiate_operation(
        originator,
        "PrivateTransferAUC_HyperEngine",
        format!("{:?}", public_payload_details).into_bytes(),
    ).expect("Failed to initiate op");
    println!("  -> NebulaPulse: Initiated op: Type '{}', Originator '{}'", initiated_op.operation_type, initiated_op.originator_id);
    nebula_pulse_swarm::send_data_to_edge(&initiated_op).expect("Failed to send to edge");

    let csn_suggested_fee = csn::get_dynamic_fee_for_novavault("PrivateTransferAUC").unwrap_or(15);
    println!("  -> CSN: Suggested fee for PrivateTransferAUC: {} micro-AUC", csn_suggested_fee);
    let mut full_public_payload_for_novavault = public_payload_details.clone();
    full_public_payload_for_novavault.insert("fee_paid".to_string(), csn_suggested_fee.to_string());

    let mock_current_block_height_for_nv = ecliptic_concordance::status().split_whitespace().last().unwrap_or("1").parse().unwrap_or(1) +1 ; // Mock next block

    let financial_op_result: FinancialOperation = novavault_flux_finance::process_financial_operation(
        originator,
        NovaVaultOpType::PrivateTransferAUC,
        full_public_payload_for_novavault,
        private_inputs_data.clone(), // Clone private_inputs_data
        mock_current_block_height_for_nv
    ).expect("NovaVault processing failed");
    println!("  -> NovaVault: Processed financial op: ID '{}', Type: {:?}", financial_op_result.id, financial_op_result.op_type);
    if let Some(ref proof) = financial_op_result.zk_proof {
        println!("     NovaVault obtained ZK Proof ID: '{}'", proof.proof_id);
    }

    let exec_request = ExecutionRequest {
        module_id: "private_auc_handler_v1".to_string(),
        function_name: "log_private_op_intent".to_string(),
        arguments: initiated_op.data.clone(),
    };
    if let Ok(execution_result) = aethercore_runtime::execute_module(exec_request) {
         println!("  -> AetherCore (Conceptual): Executed. Success: {}, Output: {:?}",
            execution_result.success, String::from_utf8_lossy(&execution_result.output));
    }

    let op_outcome_for_consensus = format!("{:?}", financial_op_result.payload);
    let op_outcome_hash = mock_hash_data(&op_outcome_for_consensus);
    let consensus_tx: ConsensusTransaction = ecliptic_concordance::submit_for_consensus(
        op_outcome_hash.clone(),
        financial_op_result.zk_proof.clone()
    ).expect("Consensus submission failed");
    println!("  -> EclipticConcordance: Submitted for consensus. TxID: '{}', ZKP ID: {:?}", consensus_tx.id, consensus_tx.zk_proof_id);

    let finalized_block: Block = ecliptic_concordance::form_and_finalize_block(vec![consensus_tx.clone()])
        .expect("Block finalization failed");
    println!("  -> EclipticConcordance: Block finalized. ID: '{}', Height: {}", finalized_block.id, finalized_block.height);

    if let Some(ref isn_node_id_from_nv) = financial_op_result.associated_isn_node_id {
        println!("[ISN_CDC] NovaVault op (ID: {}) in ISN Node ID: {} confirmed by block {}.",
            financial_op_result.id, isn_node_id_from_nv, finalized_block.height);
        if let Some(retrieved_node) = cosmic_data_constellation::get_isn_node(isn_node_id_from_nv) {
            println!("  -> ISN_CDC: Re-retrieved confirmed op record: Properties: {:?}", retrieved_node.properties);
        }
    }
    csn::monitor_novavault_activity_patterns();
    println!("  -> CSN: Monitoring of NovaVault patterns initiated.");
    match novavault_flux_finance::get_account_balance(originator, "AUC_PRIVATE") {
        Ok(balance) => println!("  -> NovaVault: Balance for {} (AUC_PRIVATE): {} (mock ISN)", originator, balance),
        Err(e) => eprintln!("  -> NovaVault: Error getting balance: {}", e),
    }
}

fn run_governance_simulation_phase() {
    println!("\n--- Running Governance Simulation Phase ---");
    let proposer_id = "developer_aurora_core_001";
    let proposal_title = "Upgrade AetherCore mock_contract_v1 to v1.1.0";
    let proposal_desc = "This is a critical_fix to improve payment processing efficiency in mock_contract_v1.";
    let target_module_id = "mock_contract_v1".to_string();
    let new_code_hash = mock_hash_data(&"new_wasm_code_for_v1_1_0"); // Hash of the conceptual new code

    // 1. Submit Proposal to StarSenate
    let proposal = match submit_proposal(
        proposer_id,
        proposal_title,
        proposal_desc,
        Some(target_module_id.clone()),
        &new_code_hash,
    ) {
        Ok(p) => p,
        Err(e) => { eprintln!("[GovSim] Error submitting proposal: {}", e); return; }
    };
    println!("  -> StarSenate: Proposal '{}' submitted. ID: {}, Futarchy Score: {:?}",
        proposal.title, proposal.id, proposal.futarchy_prediction_score);

    // 2. Simulate Voting (mock votes)
    cast_vote_on_proposal(&proposal.id, "voter_alpha", true, 100).expect("Vote failed");
    cast_vote_on_proposal(&proposal.id, "voter_beta", true, 150).expect("Vote failed");
    cast_vote_on_proposal(&proposal.id, "voter_gamma", false, 75).expect("Vote failed");

    // 3. Tally Votes and Decide
    let mock_gov_block_height = ecliptic_concordance::status().split_whitespace().last().unwrap_or("2").parse().unwrap_or(2) + 1; // Mock next block for gov action

    match tally_votes_and_decide(&proposal.id, mock_gov_block_height) {
        Ok(ProposalStatus::Approved) => {
            println!("  -> StarSenate: Proposal ID '{}' APPROVED.", proposal.id);
            // 4. If approved, AetherCore acknowledges the upgrade (conceptual)
            match aethercore_runtime::acknowledge_module_upgrade(&target_module_id, "version_1.1.0", &new_code_hash) {
                Ok(()) => println!("  -> AetherCore: Successfully acknowledged upgrade for module '{}'.", target_module_id),
                Err(e) => eprintln!("[GovSim] Error acknowledging module upgrade: {}", e),
            }
        }
        Ok(ProposalStatus::Rejected) => {
            println!("  -> StarSenate: Proposal ID '{}' REJECTED.", proposal.id);
        }
        Ok(status) => { // Other statuses like Pending, VotingOpen, Executed
            println!("  -> StarSenate: Proposal ID '{}' has status: {:?}", proposal.id, status);
        }
        Err(e) => {
            eprintln!("[GovSim] Error tallying votes for proposal '{}': {}", proposal.id, e);
        }
    }

    // Check ISN for governance action record
    // This would require a more complex query or iterating through ISN mock DB.
    // For now, we rely on the println! from record_governance_action.
}


fn main() {
    println!("=== Aurora Full Lifecycle Simulation (Financial + Governance) ===");
    run_financial_simulation_phase();
    run_governance_simulation_phase();
    println!("\n=== Full Simulation Complete ===");
}
