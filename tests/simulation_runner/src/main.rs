use aethercore_runtime::ExecutionRequest;
use ecliptic_concordance::{Transaction as ConsensusTransaction, Block};

use novavault_flux_finance::{FinancialOperationType as NovaVaultOpType, FinancialOperation};
use celestial_synapse_network_csn as csn;
use voidproof_engine_zkp::ZkProof;

use starsenate_collectives_governance::{ProposalStatus, submit_proposal, cast_vote_on_proposal, tally_votes_and_decide};

use soulstar_matrix_identity::{create_celestial_id, CelestialID};
use symbiotic_trust_lattice_stl as stl;
use verifiable_obligation_nexus_von as von;

// New imports for Ecological phase
use gaiapulse_engine::process_green_operation_attestation;
use econova_incentives::calculate_and_distribute_fluxboost_reward;


use std::collections::HashMap;
use sha2::{Sha256, Digest};
use hex;

fn mock_hash_data<T: std::fmt::Debug>(data: &T) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{:?}", data).as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

fn get_next_mock_block_height() -> u64 {
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
    let initiated_op = nebula_pulse_swarm::initiate_operation(user_did, "PrivateTransferAUC_HyperEngine", format!("{:?}", public_payload_details).into_bytes()).expect("Failed to initiate op");
    println!("  -> NebulaPulse: Initiated op: Type '{}', Originator '{}'", initiated_op.operation_type, initiated_op.originator_id);
    nebula_pulse_swarm::send_data_to_edge(&initiated_op).expect("Failed to send to edge");
    let csn_suggested_fee = csn::get_dynamic_fee_for_novavault("PrivateTransferAUC").unwrap_or(15);
    println!("  -> CSN: Suggested fee: {} micro-AUC", csn_suggested_fee);
    let mut full_public_payload_for_novavault = public_payload_details.clone();
    full_public_payload_for_novavault.insert("fee_paid".to_string(), csn_suggested_fee.to_string());
    let financial_op_result: FinancialOperation = novavault_flux_finance::process_financial_operation(user_did, NovaVaultOpType::PrivateTransferAUC, full_public_payload_for_novavault, private_inputs_data.clone(), block_height).expect("NovaVault processing failed");
    if let Some(ref proof) = financial_op_result.zk_proof { stl::update_trust_score(user_did, stl::FINANCIAL_CONTEXT, 0.05, "Generated ZKP"); }
    let exec_request = ExecutionRequest { module_id: "private_auc_handler_v1".to_string(), function_name: "log_private_op_intent".to_string(), arguments: initiated_op.data.clone() };
    if let Ok(execution_result) = aethercore_runtime::execute_module(exec_request) { println!("  -> AetherCore: Executed. Success: {}, Output: {:?}", execution_result.success, String::from_utf8_lossy(&execution_result.output)); }
    let op_outcome_hash = mock_hash_data(&financial_op_result.payload);
    let consensus_tx: ConsensusTransaction = ecliptic_concordance::submit_for_consensus(op_outcome_hash.clone(), financial_op_result.zk_proof.clone()).expect("Consensus submission failed");
    let finalized_block: Block = ecliptic_concordance::form_and_finalize_block(vec![consensus_tx.clone()]).expect("Block finalization failed");
    println!("  -> EclipticConcordance: Block finalized. ID: '{}', Height: {}", finalized_block.id, finalized_block.height);
    if let Some(ref isn_node_id) = financial_op_result.associated_isn_node_id { if let Some(rn) = cosmic_data_constellation::get_isn_node(isn_node_id) { println!("  -> ISN_CDC: Re-retrieved op record: {:?}", rn.properties); }}
    csn::monitor_novavault_activity_patterns();
    if let Ok(bal) = novavault_flux_finance::get_account_balance(user_did, "AUC_PRIVATE") { println!("  -> NovaVault: Balance for {} (AUC_PRIVATE): {} (mock ISN)", user_did, bal); }
}

fn run_governance_simulation_phase(proposer_did_str: &str, voter_dids: Vec<&str>, block_height: u64) {
    println!("\n--- Running Governance Simulation Phase ---");
    let target_module_id = "mock_contract_v1".to_string();
    let new_code_hash = mock_hash_data(&"new_wasm_code_for_v1_1_0");
    let proposal = submit_proposal(proposer_did_str, "Upgrade AetherCore v1.1.0", "Critical fix.", Some(target_module_id.clone()), &new_code_hash).expect("Proposal submission failed");
    println!("  -> StarSenate: Proposal '{}' submitted. ID: {}, Futarchy Score: {:?}", proposal.title, proposal.id, proposal.futarchy_prediction_score);
    stl::update_trust_score(proposer_did_str, stl::GOVERNANCE_CONTEXT, 0.1, "Submitted proposal");
    for (i, voter_did_str) in voter_dids.iter().enumerate() {
        cast_vote_on_proposal(&proposal.id, voter_did_str, i % 2 == 0).expect("Vote failed");
        stl::update_trust_score(voter_did_str, stl::GOVERNANCE_CONTEXT, 0.05, "Voted");
    }
    match tally_votes_and_decide(&proposal.id, block_height) {
        Ok(ProposalStatus::Approved) => {
            println!("  -> StarSenate: Proposal ID '{}' APPROVED.", proposal.id);
            stl::update_trust_score(proposer_did_str, stl::GOVERNANCE_CONTEXT, 0.2, "Proposal approved");
            aethercore_runtime::acknowledge_module_upgrade(&target_module_id, "version_1.1.0", &new_code_hash).expect("Upgrade ack failed");
            println!("  -> AetherCore: Upgraded module '{}'.", target_module_id);
        }
        Ok(ProposalStatus::Rejected) => { println!("  -> StarSenate: Proposal ID '{}' REJECTED.", proposal.id); stl::update_trust_score(proposer_did_str, stl::GOVERNANCE_CONTEXT, -0.05, "Proposal rejected"); }
        Ok(status) => println!("  -> StarSenate: Proposal ID '{}' status: {:?}", proposal.id, status),
        Err(e) => eprintln!("[GovSim] Error tallying votes: {}", e),
    }
}

fn run_von_simulation_phase(obligor_did_str: &str, obligee_did_str: &str, block_height: u64) {
    println!("\n--- Running Verifiable Obligation Nexus (VON) Simulation Phase ---");
    let due_timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 86400;
    let obligation = von::create_fluxpact_contract(obligor_did_str, obligee_did_str, "Deliver resources", 50, due_timestamp, block_height).expect("Obligation creation failed");
    println!("  -> VON: Created Obligation ID: '{}'", obligation.id);
    stl::update_trust_score(obligor_did_str, stl::FINANCIAL_CONTEXT, -0.02, "Created obligation");
    let fulfillment_proof_hash = mock_hash_data(&"Proof of delivery");
    match von::attest_obligation_fulfillment(&obligation.id, obligee_did_str, &fulfillment_proof_hash, block_height +1 ) {
        Ok(()) => {
            println!("  -> VON: Obligation ID '{}' fulfilled.", obligation.id);
            stl::update_trust_score(obligor_did_str, stl::FINANCIAL_CONTEXT, 0.15, "Fulfilled obligation");
            stl::update_trust_score(obligee_did_str, stl::FINANCIAL_CONTEXT, 0.01, "Attested fulfillment");
        }
        Err(e) => eprintln!("[VONSim] Error attesting fulfillment: {}", e),
    }
}

fn run_ecological_simulation_phase(green_validator_did: &str, block_height: u64) {
    println!("\n--- Running Ecological Simulation Phase for DID {} ---", green_validator_did);
    let operation_description = format!("Validated block #{} with green energy", block_height);
    let co2_offset_tons = 5; // Mock offset for a green block
    let base_block_reward = 100; // Mock base reward for proposing a block

    // 1. GaiaPulse Engine processes green attestation and mints EcoFlux Credits
    match process_green_operation_attestation(
        green_validator_did,
        &operation_description,
        co2_offset_tons,
        block_height,
    ) {
        Ok(credit) => {
            println!("  -> GaiaPulse/NovaCarbon: Minted EcoFlux Credit ID: '{}' for {} tons CO2e.", credit.id, credit.amount_co2e_sequestered_tons);
            // STL: Positive eco-action boosts trust in an "ecological" context (if we add one)
            // For now, maybe a small boost to general governance or financial if it's a validator
            stl::update_trust_score(green_validator_did, stl::GOVERNANCE_CONTEXT, 0.02, "Performed green operation");
        }
        Err(e) => eprintln!("[EcoSim] Error processing green attestation: {}", e),
    }

    // 2. EcoNova Incentives calculates and distributes FluxBoost Reward
    let operation_id_for_reward = format!("block_proposal_{}", block_height);
    match calculate_and_distribute_fluxboost_reward(
        green_validator_did,
        base_block_reward,
        &operation_id_for_reward,
        block_height,
    ) {
        Ok(Some(boost_amount)) => {
            println!("  -> EcoNovaIncentives: FluxBoost Reward of {} AUC distributed.", boost_amount);
        }
        Ok(None) => {
            println!("  -> EcoNovaIncentives: No FluxBoost Reward for this operation.");
        }
        Err(e) => eprintln!("[EcoSim] Error with FluxBoost Reward: {}", e),
    }
}


fn main() {
    println!("=== Aurora Full Lifecycle Simulation (Financial + Governance + Identity/STL/VON + Ecological) ===");

    println!("\n--- Running Identity Creation & STL Initialization Phase ---");
    let block_height_init = get_next_mock_block_height();
    let user_punk_did = create_celestial_id("user_punk_789", "pk_punk", block_height_init).unwrap().did;
    let dev_aurora_did = create_celestial_id("developer_aurora_core_001", "pk_dev_core", block_height_init).unwrap().did;
    let voter_alpha_did = create_celestial_id("voter_alpha_stl_green", "pk_voter_a", block_height_init).unwrap().did; // Made one voter "green" for eco sim
    let voter_beta_did = create_celestial_id("voter_beta_stl", "pk_voter_b", block_height_init).unwrap().did;
    let voter_gamma_did = create_celestial_id("voter_gamma_stl", "pk_voter_g", block_height_init).unwrap().did;
    let obligee_did_str = create_celestial_id("obligee_user_001", "pk_obligee", block_height_init).unwrap().did;
    println!("  -> SoulStar: Created DIDs.");
    stl::initialize_entity_trust(&user_punk_did);
    stl::initialize_entity_trust(&dev_aurora_did);
    stl::initialize_entity_trust(&voter_alpha_did);
    stl::initialize_entity_trust(&voter_beta_did);
    stl::initialize_entity_trust(&voter_gamma_did);
    stl::initialize_entity_trust(&obligee_did_str);

    run_financial_simulation_phase(&user_punk_did, get_next_mock_block_height());
    run_governance_simulation_phase(&dev_aurora_did, vec![&voter_alpha_did, &voter_beta_did, &voter_gamma_did], get_next_mock_block_height());
    run_von_simulation_phase(&user_punk_did, &obligee_did_str, get_next_mock_block_height());
    run_ecological_simulation_phase(&voter_alpha_did, get_next_mock_block_height()); // voter_alpha is our "green" validator

    println!("\n--- Final Mock STL Scores ---");
    println!("  DID: {}, Gov: {:.2}, Fin: {:.2}", user_punk_did, stl::get_contextual_trust_score(&user_punk_did, stl::GOVERNANCE_CONTEXT), stl::get_contextual_trust_score(&user_punk_did, stl::FINANCIAL_CONTEXT));
    println!("  DID: {}, Gov: {:.2}, Fin: {:.2}", dev_aurora_did, stl::get_contextual_trust_score(&dev_aurora_did, stl::GOVERNANCE_CONTEXT), stl::get_contextual_trust_score(&dev_aurora_did, stl::FINANCIAL_CONTEXT));
    println!("  DID: {}, Gov: {:.2}, Fin: {:.2}", voter_alpha_did, stl::get_contextual_trust_score(&voter_alpha_did, stl::GOVERNANCE_CONTEXT), stl::get_contextual_trust_score(&voter_alpha_did, stl::FINANCIAL_CONTEXT));

    println!("\n=== Full Simulation Complete ===");
}
