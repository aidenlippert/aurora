use aethercore_runtime::ExecutionRequest;
use ecliptic_concordance::{Transaction as ConsensusTransaction, Block};
// cosmic_data_constellation::IsnNode is not directly used by name in main, can be removed if desired
// use cosmic_data_constellation::IsnNode;

use novavault_flux_finance::{FinancialOperationType as NovaVaultOpType, FinancialOperation};
use celestial_synapse_network_csn as csn;
use voidproof_engine_zkp::ZkProof; // ZkProof is used for Option<ZkProof> type hint

// Governance imports
use starsenate_collectives_governance::{ProposalStatus, submit_proposal, cast_vote_on_proposal, tally_votes_and_decide};
// oraclesync_futarchy is used via its module name

// New imports for Identity, STL, VON
use soulstar_matrix_identity::{create_celestial_id, CelestialID};
use symbiotic_trust_lattice_stl as stl; // Alias for brevity
use verifiable_obligation_nexus_von as von;

use std::collections::HashMap;
use sha2::{Sha256, Digest};
use hex;

fn mock_hash_data<T: std::fmt::Debug>(data: &T) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{:?}", data).as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

// Helper to get a mock block height, crudely incrementing
fn get_next_mock_block_height() -> u64 {
    // This is a very crude way to simulate increasing block height for different phases
    // In a real system, this would come from the consensus layer.
    static mut MOCK_HEIGHT_COUNTER: u64 = 0;
    unsafe {
        MOCK_HEIGHT_COUNTER += 1;
        MOCK_HEIGHT_COUNTER
    }
}

fn run_financial_simulation_phase(user_did: &str, block_height: u64) {
    println!("\n--- Running Financial Simulation Phase for {} ---", user_did);
    let public_payload_details = {
        let mut map = HashMap::new();
        map.insert("to_address_public_key_hash".to_string(), "hash_of_cosmic_789_pk".to_string());
        map.insert("amount_display".to_string(), "CONFIDENTIAL".to_string());
        map.insert("asset".to_string(), "AUC_PRIVATE".to_string());
        map
    };
    let private_inputs_data = b"{\"actual_recipient_encrypted_id\":\"enc_cosmic_789\", \"actual_amount_encrypted\": \"enc_150AUC\"}".to_vec();

    let initiated_op = nebula_pulse_swarm::initiate_operation(
        user_did, // Use the DID
        "PrivateTransferAUC_HyperEngine",
        format!("{:?}", public_payload_details).into_bytes(),
    ).expect("Failed to initiate op");
    println!("  -> NebulaPulse: Initiated op: Type '{}', Originator '{}'", initiated_op.operation_type, initiated_op.originator_id);
    nebula_pulse_swarm::send_data_to_edge(&initiated_op).expect("Failed to send to edge");

    let csn_suggested_fee = csn::get_dynamic_fee_for_novavault("PrivateTransferAUC").unwrap_or(15);
    println!("  -> CSN: Suggested fee for PrivateTransferAUC: {} micro-AUC", csn_suggested_fee);
    let mut full_public_payload_for_novavault = public_payload_details.clone();
    full_public_payload_for_novavault.insert("fee_paid".to_string(), csn_suggested_fee.to_string());

    let financial_op_result: FinancialOperation = novavault_flux_finance::process_financial_operation(
        user_did,
        NovaVaultOpType::PrivateTransferAUC,
        full_public_payload_for_novavault,
        private_inputs_data.clone(), // Clone private_inputs_data
        block_height
    ).expect("NovaVault processing failed");
    println!("  -> NovaVault: Processed financial op: ID '{}', Type: {:?}", financial_op_result.id, financial_op_result.op_type);
    if let Some(ref proof) = financial_op_result.zk_proof {
        println!("     NovaVault obtained ZK Proof ID: '{}'", proof.proof_id);
        // Successful ZKP generation could positively impact STL score for financial reliability
        stl::update_trust_score(user_did, stl::FINANCIAL_CONTEXT, 0.05, "Generated ZKP for private transfer");
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
    match novavault_flux_finance::get_account_balance(user_did, "AUC_PRIVATE") {
        Ok(balance) => println!("  -> NovaVault: Balance for {} (AUC_PRIVATE): {} (mock ISN)", user_did, balance),
        Err(e) => eprintln!("  -> NovaVault: Error getting balance: {}", e),
    }
}

fn run_governance_simulation_phase(proposer_did_str: &str, voter_dids: Vec<&str>, block_height: u64) {
    println!("\n--- Running Governance Simulation Phase ---");
    let proposal_title = "Upgrade AetherCore mock_contract_v1 to v1.1.0";
    let proposal_desc = "This is a critical_fix to improve payment processing efficiency in mock_contract_v1.";
    let target_module_id = "mock_contract_v1".to_string();
    let new_code_hash = mock_hash_data(&"new_wasm_code_for_v1_1_0"); // Hash of the conceptual new code

    // 1. Submit Proposal to StarSenate
    let proposal = match submit_proposal(
        proposer_did_str,
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
    // Update STL for proposer
    stl::update_trust_score(proposer_did_str, stl::GOVERNANCE_CONTEXT, 0.1, "Submitted a proposal");


    for (i, voter_did_str) in voter_dids.iter().enumerate() {
        let in_favor = i % 2 == 0; // Alternate votes for mock
        cast_vote_on_proposal(&proposal.id, voter_did_str, in_favor)
            .expect("Vote failed");
        // Update STL for voter
        stl::update_trust_score(voter_did_str, stl::GOVERNANCE_CONTEXT, 0.05, "Participated in voting");
    }

    match tally_votes_and_decide(&proposal.id, block_height) {
        Ok(ProposalStatus::Approved) => {
            println!("  -> StarSenate: Proposal ID '{}' APPROVED.", proposal.id);
            stl::update_trust_score(proposer_did_str, stl::GOVERNANCE_CONTEXT, 0.2, "Proposal approved"); // Bigger boost for successful proposal
            match aethercore_runtime::acknowledge_module_upgrade(&target_module_id, "version_1.1.0", &new_code_hash) {
                Ok(()) => println!("  -> AetherCore: Successfully acknowledged upgrade for module '{}'.", target_module_id),
                Err(e) => eprintln!("[GovSim] Error acknowledging module upgrade: {}", e),
            }
        }
        Ok(ProposalStatus::Rejected) => {
            println!("  -> StarSenate: Proposal ID '{}' REJECTED.", proposal.id);
            stl::update_trust_score(proposer_did_str, stl::GOVERNANCE_CONTEXT, -0.05, "Proposal rejected");
        }
        Ok(status) => { // Other statuses like Pending, VotingOpen, Executed
            println!("  -> StarSenate: Proposal ID '{}' has status: {:?}", proposal.id, status);
        }
        Err(e) => {
            eprintln!("[GovSim] Error tallying votes for proposal '{}': {}", proposal.id, e);
        }
    }
}

fn run_von_simulation_phase(obligor_did_str: &str, obligee_did_str: &str, block_height: u64) {
    println!("\n--- Running Verifiable Obligation Nexus (VON) Simulation Phase ---");

    let description = "Deliver 10 units of mock_resource by tomorrow.";
    let collateral: u64 = 50; // mock AUC
    let due_timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 86400; // 1 day

    // 1. Create an Obligation
    let obligation = match von::create_fluxpact_contract(
        obligor_did_str,
        obligee_did_str,
        description,
        collateral,
        due_timestamp,
        block_height
    ) {
        Ok(ob) => ob,
        Err(e) => { eprintln!("[VONSim] Error creating obligation: {}", e); return; }
    };
    println!("  -> VON: Created Obligation ID: '{}', Obligor: '{}', Obligee: '{}'",
        obligation.id, obligation.obligor_did, obligation.obligee_did);
    // STL: Creating an obligation might slightly decrease obligor's financial trust until fulfilled
    stl::update_trust_score(obligor_did_str, stl::FINANCIAL_CONTEXT, -0.02, "Created new obligation");


    // 2. Attest Fulfillment
    let fulfillment_proof_hash = mock_hash_data(&"Proof that mock_resource was delivered");
    match von::attest_obligation_fulfillment(&obligation.id, obligee_did_str, &fulfillment_proof_hash, block_height +1 ) {
        Ok(()) => {
            println!("  -> VON: Obligation ID '{}' successfully attested as fulfilled by '{}'.", obligation.id, obligee_did_str);
            // STL: Fulfilling an obligation significantly boosts financial trust for obligor
            stl::update_trust_score(obligor_did_str, stl::FINANCIAL_CONTEXT, 0.15, "Successfully fulfilled obligation");
            // STL: Obligee (attestor) might also get a small boost for participation if relevant
            stl::update_trust_score(obligee_did_str, stl::FINANCIAL_CONTEXT, 0.01, "Attested obligation fulfillment");
        }
        Err(e) => eprintln!("[VONSim] Error attesting fulfillment for obligation '{}': {}", obligation.id, e),
    }
}

fn main() {
    println!("=== Aurora Full Lifecycle Simulation (Financial + Governance + Identity/STL/VON) ===");

    // --- Phase 0: Identity Creation & STL Initialization ---
    println!("\n--- Running Identity Creation & STL Initialization Phase ---");
    let block_height_init = get_next_mock_block_height();

    let user_punk_did = create_celestial_id("user_punk_789", "pk_punk", block_height_init).expect("ID creation failed").did;
    let dev_aurora_did = create_celestial_id("developer_aurora_core_001", "pk_dev_core", block_height_init).expect("ID creation failed").did;
    let voter_alpha_did = create_celestial_id("voter_alpha_stl", "pk_voter_a", block_height_init).expect("ID creation failed").did;
    let voter_beta_did = create_celestial_id("voter_beta_stl", "pk_voter_b", block_height_init).expect("ID creation failed").did;
    let voter_gamma_did = create_celestial_id("voter_gamma_stl", "pk_voter_g", block_height_init).expect("ID creation failed").did;
    let obligee_did_str = create_celestial_id("obligee_user_001", "pk_obligee", block_height_init).expect("ID creation failed").did;


    println!("  -> SoulStar: Created DIDs: {}, {}, {}, {}, {}, {}",
        user_punk_did, dev_aurora_did, voter_alpha_did, voter_beta_did, voter_gamma_did, obligee_did_str);

    // Initialize trust for these new DIDs
    stl::initialize_entity_trust(&user_punk_did);
    stl::initialize_entity_trust(&dev_aurora_did);
    stl::initialize_entity_trust(&voter_alpha_did);
    stl::initialize_entity_trust(&voter_beta_did);
    stl::initialize_entity_trust(&voter_gamma_did);
    stl::initialize_entity_trust(&obligee_did_str);


    // --- Run other phases ---
    run_financial_simulation_phase(&user_punk_did, get_next_mock_block_height());
    run_governance_simulation_phase(&dev_aurora_did, vec![&voter_alpha_did, &voter_beta_did, &voter_gamma_did], get_next_mock_block_height());
    run_von_simulation_phase(&user_punk_did, &obligee_did_str, get_next_mock_block_height());

    // --- Final STL Scores (Example) ---
    println!("\n--- Final Mock STL Scores ---");
    println!("  DID: {}, Governance Score: {:.2}, Financial Score: {:.2}",
        user_punk_did,
        stl::get_contextual_trust_score(&user_punk_did, stl::GOVERNANCE_CONTEXT),
        stl::get_contextual_trust_score(&user_punk_did, stl::FINANCIAL_CONTEXT));
    println!("  DID: {}, Governance Score: {:.2}, Financial Score: {:.2}",
        dev_aurora_did,
        stl::get_contextual_trust_score(&dev_aurora_did, stl::GOVERNANCE_CONTEXT),
        stl::get_contextual_trust_score(&dev_aurora_did, stl::FINANCIAL_CONTEXT));
     println!("  DID: {}, Governance Score: {:.2}, Financial Score: {:.2}",
        voter_alpha_did,
        stl::get_contextual_trust_score(&voter_alpha_did, stl::GOVERNANCE_CONTEXT),
        stl::get_contextual_trust_score(&voter_alpha_did, stl::FINANCIAL_CONTEXT));


    println!("\n=== Full Simulation Complete ===");
}