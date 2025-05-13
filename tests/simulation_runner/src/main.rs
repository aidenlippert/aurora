use aethercore_runtime::ExecutionRequest;
// Corrected import for ConcordanceTransaction, removed unused TransactionPayload from direct import
use ecliptic_concordance::{ConcordanceTransaction, Block, submit_transaction_payload, sequencer_create_block, validate_and_apply_block, ConsensusState, get_current_state_summary};
use novavault_flux_finance::{FinancialOperationType as NovaVaultOpType, FinancialOperation};
use celestial_synapse_network_csn as csn;
use starsenate_collectives_governance::{ProposalStatus, submit_proposal, cast_vote_on_proposal, tally_votes_and_decide};
use soulstar_matrix_identity::create_celestial_id;
use symbiotic_trust_lattice_stl as stl;
use verifiable_obligation_nexus_von as von;
use gaiapulse_engine::process_green_operation_attestation;
use econova_incentives::calculate_and_distribute_fluxboost_reward;
use astrocli_deployment_nexus::{compile_dapp_mock, request_dapp_deployment, MockDappCompilation};
use nexus_cosmic_introspection_nci::generate_integrity_report;
use nebulashield_defenses::OperationTrace;
use cosmic_justice_enforcers::MisbehaviorType;
use eonmirror_interface::{ingest_real_world_data, RealWorldDataPoint};
use chronoforge_simulator::generate_prediction_from_isn_data;
use gaiapulse_engine::react_to_environmental_prediction;
use semantic_synapse_interfaces;
use serde_json;

use wasmi::Value;
use std::collections::HashMap;
use sha2::{Sha256, Digest};
use hex;
use std::sync::Mutex; // For the single ConsensusState in the runner
use once_cell::sync::Lazy; // For the single ConsensusState in the runner


fn mock_hash_data<T: std::fmt::Debug>(data: &T) -> String {
    let mut hasher = Sha256::new(); hasher.update(format!("{:?}", data).as_bytes()); hex::encode(hasher.finalize())
}

// get_next_mock_block_height will now take a reference to the consensus state
fn get_next_target_block_height(consensus_state: &ConsensusState) -> u64 {
    consensus_state.current_height + 1
}


fn run_financial_simulation_phase(
    user_did: &str,
    consensus_state: &mut ConsensusState, // Pass mutable ref
) {
    let target_block_height = consensus_state.current_height + 1; // For ISN records
    println!("\n--- Running Financial Simulation Phase for {} (Targeting Block {}) ---", user_did, target_block_height);
    let mut public_payload_details: HashMap<String, String> = HashMap::new();
    public_payload_details.insert("to_address_public_key_hash".to_string(), "hash_of_cosmic_789_pk".to_string());
    public_payload_details.insert("amount_display".to_string(), "CONFIDENTIAL".to_string());
    public_payload_details.insert("asset".to_string(), "AUC_PRIVATE".to_string());
    let private_inputs_data = b"{\"actual_recipient_encrypted_id\":\"enc_cosmic_789\", \"actual_amount_encrypted\": \"enc_150AUC\"}".to_vec();
    
    let initiated_op_payload_for_nebula = nebula_pulse_swarm::initiate_operation(user_did, "PrivateTransferAUC_HyperEngine", format!("{:?}", public_payload_details).into_bytes()).expect("Op init failed");
    println!("  -> NebulaPulse: Initiated op: Type '{}', Originator '{}'", initiated_op_payload_for_nebula.operation_type, initiated_op_payload_for_nebula.originator_id);
    let _ = nebula_pulse_swarm::package_and_send_to_edge(&initiated_op_payload_for_nebula.originator_id, initiated_op_payload_for_nebula.clone()).expect("Send to edge failed");

    let csn_suggested_fee = csn::get_dynamic_fee_for_novavault("PrivateTransferAUC").unwrap_or(15);
    println!("  -> CSN: Suggested fee: {} micro-AUC", csn_suggested_fee);
    let mut full_public_payload_for_novavault = public_payload_details.clone();
    full_public_payload_for_novavault.insert("fee_paid".to_string(), csn_suggested_fee.to_string());

    let financial_op_result: FinancialOperation = novavault_flux_finance::process_financial_operation(
        user_did, NovaVaultOpType::PrivateTransferAUC, full_public_payload_for_novavault, 
        private_inputs_data.clone(), target_block_height
    ).expect("NV process failed");
    if let Some(ref _proof) = financial_op_result.zk_proof { stl::update_trust_score(user_did, stl::FINANCIAL_CONTEXT, 0.05, "Generated ZKP"); }

    // --- Consensus Part for the Financial Operation ---
    let consensus_payload_data = serde_json::to_vec(&financial_op_result.payload).expect("Failed to serialize fin_op payload for consensus");
    let submitted_tx_id = submit_transaction_payload(consensus_state, consensus_payload_data) // Pass state
        .expect("Failed to submit payload to consensus mempool");
    println!("  -> EclipticConcordance: Payload for financial op submitted to mempool, TxID: {}", submitted_tx_id);

    let sequencer_node_id = "sim_runner_sequencer";
    let new_block = sequencer_create_block(consensus_state, sequencer_node_id) // Pass state
        .expect("Sequencer failed to create block");
    println!("  -> EclipticConcordance: Sequencer created and applied Block Height: {}. ID: '{}'", new_block.height, new_block.id);
    // No need for validate_and_apply_block here as sequencer_create_block updated the state
    // --- End Consensus Part ---

    let exec_req = ExecutionRequest { module_id: "private_auc_handler_v1".to_string(), function_name: "log_private_op_intent".to_string(), arguments: Vec::new(), gas_limit: 500 };
    if let Ok(exec_res) = aethercore_runtime::execute_module(exec_req) { 
        println!("  -> AetherCore (Legacy): Executed. Success: {}, Output: {:?}, GasConsumed: {}", 
                 exec_res.success, 
                 exec_res.output_values.get(0).map_or_else(|| "None".to_string(), |v| format!("{:?}", v)),
                 exec_res.gas_consumed_total
        ); 
    }
    if let Some(ref node_id) = financial_op_result.associated_isn_node_id { if let Some(rn) = cosmic_data_constellation::get_isn_node(node_id) { println!("  -> ISN_CDC: Re-retrieved op record: {:?}", rn.properties); }}
    csn::monitor_novavault_activity_patterns();
    if let Ok(bal) = novavault_flux_finance::get_account_balance(user_did, "AUC_PRIVATE") { println!("  -> NovaVault: Balance for {} (AUC_PRIVATE): {} (mock ISN)", user_did, bal); }
}

fn run_governance_simulation_phase(proposer_did_str: &str, voter_dids: Vec<&str>, consensus_state: &mut ConsensusState) {
    let target_block_height = consensus_state.current_height + 1;
    println!("\n--- Running Governance Simulation Phase (Targeting Block {}) ---", target_block_height);
    let target_module_id = "mock_contract_v1".to_string();
    let new_code_hash = mock_hash_data(&"new_wasm_code_for_v1_1_0_empty_bytecode_upgrade");
    let proposal: starsenate_collectives_governance::Proposal = submit_proposal(proposer_did_str, "Upgrade mock_contract_v1 to v1.1.0", "Critical fix for mock contract.", Some(target_module_id.clone()), &new_code_hash).expect("Proposal submission failed");
    println!("  -> StarSenate: Proposal '{}' submitted. ID: {}, Futarchy Score: {:?}", proposal.title, proposal.id, proposal.futarchy_prediction_score);
    stl::update_trust_score(proposer_did_str, stl::GOVERNANCE_CONTEXT, 0.1, "Submitted proposal");
    for (i, voter_did_str) in voter_dids.iter().enumerate() {
        cast_vote_on_proposal(&proposal.id, voter_did_str, i % 2 == 0).expect("Vote failed");
        stl::update_trust_score(voter_did_str, stl::GOVERNANCE_CONTEXT, 0.05, "Voted");
    }
    match tally_votes_and_decide(&proposal.id, target_block_height) {
        Ok(ProposalStatus::Approved) => {
            println!("  -> StarSenate: Proposal ID '{}' APPROVED.", proposal.id);
            stl::update_trust_score(proposer_did_str, stl::GOVERNANCE_CONTEXT, 0.2, "Proposal approved");
            // This will also need its own transaction to go into a block if it's an on-chain action
            // For now, we directly call AetherCore as if it's an off-chain effect of governance.
            aethercore_runtime::acknowledge_module_upgrade(&target_module_id, "version_1.1.0", &new_code_hash, None).expect("Upgrade ack failed");
            println!("  -> AetherCore: Upgraded module '{}'.", target_module_id);
        }
        Ok(ProposalStatus::Rejected) => { println!("  -> StarSenate: Proposal ID '{}' REJECTED.", proposal.id); stl::update_trust_score(proposer_did_str, stl::GOVERNANCE_CONTEXT, -0.05, "Proposal rejected"); }
        Ok(status) => println!("  -> StarSenate: Proposal ID '{}' status: {:?}", proposal.id, status),
        Err(e) => eprintln!("[GovSim] Error tallying votes: {}", e),
    }
}

fn run_von_simulation_phase(obligor_did_str: &str, obligee_did_str: &str, consensus_state: &mut ConsensusState) {
    let target_block_height = consensus_state.current_height + 1;
    println!("\n--- Running Verifiable Obligation Nexus (VON) Simulation Phase (Targeting Block {}) ---", target_block_height);
    let due_timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 86400;
    let obligation = von::create_fluxpact_contract(obligor_did_str, obligee_did_str, "Deliver resources", 50, due_timestamp, target_block_height).expect("Obligation creation failed");
    println!("  -> VON: Created Obligation ID: '{}'", obligation.id);
    stl::update_trust_score(obligor_did_str, stl::FINANCIAL_CONTEXT, -0.02, "Created obligation");
    let fulfillment_proof_hash = mock_hash_data(&"Proof of delivery");
    match von::attest_obligation_fulfillment(&obligation.id, obligee_did_str, &fulfillment_proof_hash, target_block_height + 1 ) { // Assuming fulfillment in a subsequent block
        Ok(()) => {
            println!("  -> VON: Obligation ID '{}' fulfilled.", obligation.id);
            stl::update_trust_score(obligor_did_str, stl::FINANCIAL_CONTEXT, 0.15, "Fulfilled obligation");
            stl::update_trust_score(obligee_did_str, stl::FINANCIAL_CONTEXT, 0.01, "Attested fulfillment");
        }
        Err(e) => eprintln!("[VONSim] Error attesting fulfillment: {}", e),
    }
}

fn run_ecological_simulation_phase(green_validator_did: &str, consensus_state: &mut ConsensusState) {
    let target_block_height = consensus_state.current_height + 1;
    println!("\n--- Running Ecological Simulation Phase for DID {} (Targeting Block {}) ---", green_validator_did, target_block_height);
    let operation_description = format!("Validated block #{} with green energy", target_block_height); // Using target block height
    match process_green_operation_attestation(green_validator_did, &operation_description, 5, target_block_height) {
        Ok(credit) => {
            println!("  -> GaiaPulse: Minted EcoCredit for {} tons", credit.amount_co2e_sequestered_tons);
            stl::update_trust_score(green_validator_did, stl::GOVERNANCE_CONTEXT, 0.02, "Performed green op");
        }
        Err(e) => eprintln!("[EcoSim] Error processing green attestation: {}", e),
    }
    let op_id_for_reward = format!("block_proposal_{}", target_block_height);
    if let Ok(Some(boost)) = calculate_and_distribute_fluxboost_reward(green_validator_did, 100, &op_id_for_reward, target_block_height) { 
        println!("  -> EcoNova: FluxBoost of {} distributed.", boost); 
    }
}

fn run_developer_deployment_phase(developer_did: &str, consensus_state: &mut ConsensusState, wasm_module_crate_name: &str) -> Option<String> {
    let target_block_height = consensus_state.current_height + 1;
    println!("\n--- Running Developer Deployment Simulation Phase for DID {} (Wasm Crate: {}, Targeting Block {}) ---", developer_did, wasm_module_crate_name, target_block_height);
    let wasm_base_path_for_loading = "";
    let compilation_output: MockDappCompilation = match compile_dapp_mock(wasm_module_crate_name, developer_did, wasm_base_path_for_loading) {
        Ok(comp) => comp, Err(e) => { eprintln!("[DevSim] DApp Wasm loading/compilation failed for '{}': {}", wasm_module_crate_name, e); return None; }
    };
    println!("  -> AstroCLI: DApp '{}' (from crate {}) \"compiled\". Wasm Bytecode Hash: {}. Bytecode size: {}", compilation_output.dapp_name, wasm_module_crate_name, compilation_output.mock_wasm_bytecode_hash, compilation_output.wasm_bytecode.len());
    match request_dapp_deployment(compilation_output.clone(), "AetherCore_Target", target_block_height) {
        Ok(deployed_module_id) => {
            println!("  -> AstroCLI: DApp '{}' deployment successful. Deployed (AetherCore) Module ID: '{}'", compilation_output.dapp_name, deployed_module_id);
            stl::update_trust_score(developer_did, stl::GOVERNANCE_CONTEXT, 0.1, &format!("Successfully deployed DApp: {}", compilation_output.dapp_name));
            let gas_limit_for_dapp_exec = 20000; // Give more gas for Wasm
            if deployed_module_id == "sample_wasm_module_add" {
                println!("\n  --- Attempting to execute Wasm DApp '{}' (function: add) ---", deployed_module_id);
                let exec_req = ExecutionRequest { module_id: deployed_module_id.clone(), function_name: "add".to_string(), arguments: vec![Value::I32(700), Value::I32(52)], gas_limit: gas_limit_for_dapp_exec };
                if let Ok(res) = aethercore_runtime::execute_module(exec_req) { println!("  -> AetherCore (sample_add): Success: {}, Output: {:?}, GasConsumed: {}, Logs: {:?}", res.success, res.output_values, res.gas_consumed_total, res.logs); }
            } else if deployed_module_id == "sample_wasm_host_interaction" {
                println!("\n  --- Attempting to execute Wasm DApp '{}' (function: perform_action_and_log) ---", deployed_module_id);
                let exec_req_log = ExecutionRequest { module_id: deployed_module_id.clone(), function_name: "perform_action_and_log".to_string(), arguments: Vec::new(), gas_limit: gas_limit_for_dapp_exec };
                if let Ok(res) = aethercore_runtime::execute_module(exec_req_log) { println!("  -> AetherCore (host_log): Success: {}, Output: {:?}, GasConsumed: {}, Logs: {:?}", res.success, res.output_values, res.gas_consumed_total, res.logs); }
                println!("\n  --- Attempting to execute Wasm DApp '{}' (function: process_and_log_value) ---", deployed_module_id);
                let exec_req_val = ExecutionRequest { module_id: deployed_module_id.clone(), function_name: "process_and_log_value".to_string(), arguments: vec![Value::I32(155)], gas_limit: gas_limit_for_dapp_exec };
                if let Ok(res) = aethercore_runtime::execute_module(exec_req_val) { println!("  -> AetherCore (host_val_log): Success: {}, Output: {:?}, GasConsumed: {}, Logs: {:?}", res.success, res.output_values, res.gas_consumed_total, res.logs); }
            }
            Some(deployed_module_id)
        }
        Err(e) => { eprintln!("[DevSim] DApp deployment failed: {}", e); stl::update_trust_score(developer_did, stl::GOVERNANCE_CONTEXT, -0.1, &format!("Failed DApp deployment: {}", compilation_output.dapp_name)); None }
    }
}
fn run_risk_ethics_simulation_phase(malicious_dev_did: &str, risky_dev_did: &str, normal_dapp_module_id: &str, consensus_state: &mut ConsensusState) {
    let target_block_height = consensus_state.current_height + 1;
    println!("\n--- Running Risk Mitigation & Ethical Oversight Simulation Phase (Targeting Block {}) ---", target_block_height);
    println!("\n  Scenario 1: Developer '{}' attempts to deploy 'malicious_dapp_attempt'...", malicious_dev_did);
    run_developer_deployment_phase(malicious_dev_did, target_block_height, "malicious_dapp_attempt");
    println!("\n  Scenario 2: Developer '{}' attempts to deploy 'risky_dapp_code'...", risky_dev_did);
    run_developer_deployment_phase(risky_dev_did, target_block_height, "risky_dapp_code");
    println!("\n  Scenario 3: Deployed DApp '{}' performs an anomalous operation...", normal_dapp_module_id);
    if normal_dapp_module_id.is_empty() || normal_dapp_module_id == "malicious_dapp_attempt" || normal_dapp_module_id == "risky_dapp_code" { 
        println!("  Skipping anomaly test for '{}' as it's not a valid/good DApp ID.", normal_dapp_module_id); return; 
    }
    let trace = OperationTrace { module_id: normal_dapp_module_id.to_string(), function_name: "critical_function_with_exploit_log".to_string(), gas_used: 60000, logs: vec!["Log: Normal step".to_string(), "Log: attempting_exploit_secret_data".to_string()], return_value_hash: mock_hash_data(&"anomalous_output") };
    if let Some(anomaly) = nebulashield_defenses::detect_anomalous_operation(&trace) {
        println!("  -> NebulaShield: Anomaly {:?} detected for module '{}'.", anomaly, normal_dapp_module_id);
        let misbehavior = MisbehaviorType::AnomalyDetected(format!("{:?}", anomaly));
        if let Ok(()) = cosmic_justice_enforcers::apply_penalty_for_misbehavior(normal_dapp_module_id, misbehavior, 3, target_block_height + 1) { println!("  -> CosmicJustice: Penalty applied for anomalous op of module '{}'.", normal_dapp_module_id); }
        let _ = generate_integrity_report(normal_dapp_module_id, "DAppRuntimeAnomaly", vec![format!("Anomaly detected: {:?}", anomaly)], 3, vec!["Quarantine module.".to_string()], target_block_height + 1);
    } else { println!("  -> NebulaShield: No anomaly detected for module '{}'.", normal_dapp_module_id); }
}
fn run_reality_sync_prediction_phase(sensor_operator_did: &str, consensus_state: &mut ConsensusState) {
    let target_block_height = consensus_state.current_height + 1;
    println!("\n--- Running Reality Sync & Prediction Simulation Phase (Targeting Block {}) ---", target_block_height);
    let mut sensor_metadata = HashMap::new();
    sensor_metadata.insert("unit".to_string(), "ppm".to_string());
    let data_point1 = RealWorldDataPoint {
        source_id: "iot_sensor_zoneA_pollution".to_string(), data_type: "pollution_ppm".to_string(),
        value_as_string: "75.5".to_string(), timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
        location_geohash: Some("u4pruydqqvj".to_string()), metadata: sensor_metadata.clone(),
    };
    let isn_data_node = match ingest_real_world_data(data_point1, target_block_height) {
        Ok(node) => node, Err(e) => { eprintln!("[RealitySync] Error ingesting data: {}", e); return; }
    };
    println!("  -> EonMirror: Ingested data, ISN Node ID: {}", isn_data_node.id);
    stl::update_trust_score(sensor_operator_did, stl::FINANCIAL_CONTEXT, 0.01, "Provided sensor data");
    match generate_prediction_from_isn_data(&isn_data_node.id, "env_pollution_model_v1", target_block_height + 1) {
        Ok(prediction) => {
            println!("  -> ChronoForge: Generated Prediction ID: '{}', Type: '{}'", prediction.prediction_id, prediction.prediction_type);
            let zone_a_guardian_did = "did:aurora:eco_guardian_zone_a";
            stl::initialize_entity_trust(zone_a_guardian_did); // Ensure guardian is known to STL
            react_to_environmental_prediction(&prediction.prediction_type, &prediction.predicted_value_or_event, prediction.confidence_score, target_block_height + 2, Some(zone_a_guardian_did));
        }
        Err(e) => eprintln!("[RealitySync] Error generating prediction: {}", e),
    }
}
fn run_isn_graph_query_phase(developer_did: &str, _consensus_state: &ConsensusState) { // block_height unused here now
    println!("\n--- Running ISN Graph Query Simulation Phase for Developer {} ---", developer_did);
    let query_str = format!("GET_DEPLOYED_MODULES_BY_DEV_DID {}", developer_did);
    println!("  -> ISN Query: {}", query_str);
    match semantic_synapse_interfaces::query_isn(&query_str) {
        Ok(result) => { println!("  -> ISN Query Result (Modules deployed by {}): {}", developer_did, result.data_json); }
        Err(e) => eprintln!("  -> ISN Query Error: {}", e),
    }
}

fn main() {
    println!("=== Aurora Full Lifecycle Simulation (All Phases including Gas & Graph Query) ===");
    
    // Single ConsensusState instance for the entire simulation run
    let mut consensus_state = ConsensusState::new("simulation_runner_node".to_string());


    println!("\n--- Running Identity Creation & STL Initialization Phase ---");
    let initial_block_height = consensus_state.current_height +1; // Block for identity records

    let user_punk_did = create_celestial_id("user_punk_789", "pk_punk", initial_block_height).unwrap().did;
    let dev_aurora_did = create_celestial_id("developer_aurora_core_001", "pk_dev_core", initial_block_height).unwrap().did;
    let voter_alpha_did = create_celestial_id("voter_alpha_stl_green", "pk_voter_a", initial_block_height).unwrap().did;
    let dapp_developer_did = create_celestial_id("dapp_dev_cosmic", "pk_dapp_dev", initial_block_height).unwrap().did;
    let malicious_dev_did = create_celestial_id("malicious_dev_007", "pk_mal_dev", initial_block_height).unwrap().did;
    let risky_dev_did = create_celestial_id("risky_dev_008", "pk_risky_dev", initial_block_height).unwrap().did;
    let other_voters_temp = vec![create_celestial_id("voter_beta_stl", "pk_voter_b", initial_block_height).unwrap().did, create_celestial_id("voter_gamma_stl", "pk_voter_g", initial_block_height).unwrap().did];
    let other_voters: Vec<&str> = other_voters_temp.iter().map(AsRef::as_ref).collect();
    let obligee_did_str = create_celestial_id("obligee_user_001", "pk_obligee", initial_block_height).unwrap().did;
    println!("  -> SoulStar: Created DIDs.");
    let mut all_dids_for_stl_strings = vec![user_punk_did.clone(), dev_aurora_did.clone(), voter_alpha_did.clone(), other_voters_temp[0].clone(), other_voters_temp[1].clone(), obligee_did_str.clone(), dapp_developer_did.clone(), malicious_dev_did.clone(), risky_dev_did.clone()];
    all_dids_for_stl_strings.iter().for_each(|did_str| stl::initialize_entity_trust(did_str));

    // Simulate a block being created for identity records
    let _identity_block = sequencer_create_block(&mut consensus_state, "identity_sequencer").expect("Identity block creation failed");


    run_financial_simulation_phase(&user_punk_did, &mut consensus_state);
    run_governance_simulation_phase(&dev_aurora_did, other_voters.clone(), &mut consensus_state);
    run_von_simulation_phase(&user_punk_did, &obligee_did_str, &mut consensus_state);
    run_ecological_simulation_phase(&voter_alpha_did, &mut consensus_state);
    
    let deployed_adder_module_id = run_developer_deployment_phase(&dapp_developer_did, &mut consensus_state, "sample_wasm_module_add")
        .unwrap_or_else(|| "sample_wasm_module_add".to_string());
    run_developer_deployment_phase(&dapp_developer_did, &mut consensus_state, "sample_wasm_host_interaction");
    
    run_reality_sync_prediction_phase(&voter_alpha_did, &mut consensus_state);
    run_risk_ethics_simulation_phase(&malicious_dev_did, &risky_dev_did, &deployed_adder_module_id, &mut consensus_state);
    run_isn_graph_query_phase(&dapp_developer_did, &consensus_state); // Pass immutable ref

    println!("\n--- Final Mock STL Scores ---");
    let zone_a_guardian_did = "did:aurora:eco_guardian_zone_a";
    if !all_dids_for_stl_strings.contains(&zone_a_guardian_did.to_string()) {
        all_dids_for_stl_strings.push(zone_a_guardian_did.to_string());
        stl::initialize_entity_trust(zone_a_guardian_did);
    }
    for did_str_owned in all_dids_for_stl_strings.iter() {
        println!("  DID: {}, Gov: {:.2}, Fin: {:.2}", did_str_owned,
            stl::get_contextual_trust_score(did_str_owned, stl::GOVERNANCE_CONTEXT),
            stl::get_contextual_trust_score(did_str_owned, stl::FINANCIAL_CONTEXT));
    }
    println!("\n=== Full Simulation Complete ===");
}
