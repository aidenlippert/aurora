// tests/simulation_runner/src/main.rs
use aethercore_runtime::ExecutionRequest; // Keep this for the struct
use wasmi::Value as WasmiValue; // Use wasmi::Value directly for constructing arguments

use ecliptic_concordance::{
    submit_aurora_transaction, 
    sequencer_create_block, ConsensusState, AuroraTransaction, TransferAucPayload
};
use novavault_flux_finance::{process_public_auc_transfer, get_account_balance as novavault_get_balance, ensure_account_exists_with_initial_funds};
// Removed: use celestial_synapse_network_csn as csn; // Not directly used for now
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
use semantic_synapse_interfaces; // For query_isn

use std::collections::HashMap;
use sha2::{Sha256, Digest};
use hex;
use ed25519_dalek::{SigningKey, SecretKey as DalekSecretKey};
use once_cell::sync::Lazy; 

static SIM_TEMP_NONCE_TRACKER: Lazy<std::sync::Mutex<HashMap<String, u64>>> = Lazy::new(|| std::sync::Mutex::new(HashMap::new()));

fn mock_hash_data<T: std::fmt::Debug>(data: &T) -> String {
    let mut hasher = Sha256::new(); hasher.update(format!("{:?}", data).as_bytes()); hex::encode(hasher.finalize())
}

fn get_next_target_block_height(consensus_state: &ConsensusState) -> u64 {
    if consensus_state.current_height == 0 && consensus_state.last_block_hash == "GENESIS_HASH_0.0.1" {
        0 
    } else {
        consensus_state.current_height + 1
    }
}

fn get_simulation_sequencer_key(sequencer_name: &str) -> SigningKey {
    let mut seed_hasher = Sha256::new();
    seed_hasher.update(b"simulation_sequencer_seed_"); 
    seed_hasher.update(sequencer_name.as_bytes());
    let seed_hash_output = seed_hasher.finalize();
    let mut seed_array = [0u8; ed25519_dalek::SECRET_KEY_LENGTH];
    seed_array.copy_from_slice(&seed_hash_output[..ed25519_dalek::SECRET_KEY_LENGTH]);
    let dalek_secret_key = DalekSecretKey::from(seed_array);
    SigningKey::from(&dalek_secret_key)
}

fn get_next_nonce_for_sender(sender_pk_hex: &str) -> u64 {
    let mut nonce_map = SIM_TEMP_NONCE_TRACKER.lock().unwrap();
    let nonce = nonce_map.entry(sender_pk_hex.to_string()).or_insert(0);
    let current_nonce = *nonce;
    *nonce += 1;
    current_nonce
}

fn run_financial_simulation_phase(user_did_pk_hex: &str, recipient_pk_hex: &str, consensus_state: &mut ConsensusState) {
    let target_block_height = get_next_target_block_height(consensus_state);
    println!("\n--- Running Financial Simulation Phase for Sender {} (Targeting Block {}) ---", user_did_pk_hex, target_block_height);
    
    let transfer_amount = 150u64;
    let nonce = get_next_nonce_for_sender(user_did_pk_hex);

    let transfer_payload_for_novavault = TransferAucPayload {
        sender_pk_hex: user_did_pk_hex.to_string(),
        recipient_pk_hex: recipient_pk_hex.to_string(),
        amount: transfer_amount,
        nonce,
    };
    
    match process_public_auc_transfer(&transfer_payload_for_novavault, target_block_height) {
        Ok(novavault_op_id) => {
            println!("  -> NovaVault: Locally processed public AUC transfer. Op ID: {}", novavault_op_id);
            stl::update_trust_score(user_did_pk_hex, stl::FINANCIAL_CONTEXT, 0.05, "Initiated valid public transfer");

            let aurora_tx = AuroraTransaction::TransferAUC(transfer_payload_for_novavault);
            let submitted_tx_id = submit_aurora_transaction(consensus_state, aurora_tx)
                .expect("Failed to submit AuroraTransaction to consensus mempool");
            println!("  -> EclipticConcordance: AuroraTransaction for public transfer submitted. CTxID: {}", submitted_tx_id);
        }
        Err(e) => {
            eprintln!("[FinancialSim] NovaVault pre-check failed for transfer: {}. Tx not submitted.", e);
            stl::update_trust_score(user_did_pk_hex, stl::FINANCIAL_CONTEXT, -0.1, "Attempted invalid transfer");
            return; 
        }
    }
    
    let sim_sequencer_key = get_simulation_sequencer_key("financial_sequencer");
    let new_block = sequencer_create_block(consensus_state, &sim_sequencer_key)
        .expect("Sequencer failed to create block");
    println!("  -> EclipticConcordance: Sequencer (PK_hex:{:.8}) created Block H:{} with {} Txs.", new_block.proposer_pk_hex(), new_block.height, new_block.transactions.len());

    for tx_wrapper in &new_block.transactions {
        if let AuroraTransaction::TransferAUC(payload_in_block) = &tx_wrapper.payload { // This if let is okay as transactions can be other types in future
            if let Err(e) = process_public_auc_transfer(payload_in_block, new_block.height) {
                 eprintln!("[FinancialSim] Error when validator re-applies TransferAUC tx {} from block H:{}: {}", tx_wrapper.id, new_block.height, e);
            }
        }
    }

    if let Ok(bal_sender) = novavault_get_balance(user_did_pk_hex) { 
        println!("  -> NovaVault: Final Sender {} Balance: {}", user_did_pk_hex, bal_sender); 
    }
    if let Ok(bal_recipient) = novavault_get_balance(recipient_pk_hex) { 
        println!("  -> NovaVault: Final Recipient {} Balance: {}", recipient_pk_hex, bal_recipient); 
    }
}

fn run_governance_simulation_phase(proposer_did_str: &str, voter_dids: Vec<&str>, consensus_state: &mut ConsensusState) {
    let target_block_height = get_next_target_block_height(consensus_state);
    println!("\n--- Running Governance Simulation Phase (Targeting Block {}) ---", target_block_height);
    let target_module_id = "mock_contract_v1".to_string();
    let new_code_hash = mock_hash_data(&"new_wasm_code_for_v1_1_0_empty_bytecode_upgrade");
    let proposal = submit_proposal(proposer_did_str, "Upgrade mock_contract_v1 to v1.1.0", "Critical fix for mock contract.", Some(target_module_id.clone()), &new_code_hash).expect("Proposal submission failed");
    println!("  -> StarSenate: Proposal '{}' submitted. ID: {}, Futarchy Score: {:?}", proposal.title, proposal.id, proposal.futarchy_prediction_score);
    stl::update_trust_score(proposer_did_str, stl::GOVERNANCE_CONTEXT, 0.1, "Submitted proposal");
    for (i, voter_did_str) in voter_dids.iter().enumerate() {
        cast_vote_on_proposal(&proposal.id, voter_did_str, i % 2 == 0).expect("Vote failed");
        stl::update_trust_score(voter_did_str, stl::GOVERNANCE_CONTEXT, 0.05, "Voted");
    }
    let decision_aurora_tx = AuroraTransaction::TransferAUC(TransferAucPayload { 
        sender_pk_hex: "system_gov_marker_pk".to_string(), 
        recipient_pk_hex: "system_gov_log_pk".to_string(),
        amount: 0, 
        nonce: get_next_nonce_for_sender("system_gov_marker_pk"), 
    });
    submit_aurora_transaction(consensus_state, decision_aurora_tx).expect("Submit gov decision payload failed");
    
    let gov_sequencer_key = get_simulation_sequencer_key("gov_sequencer");
    let _gov_block = sequencer_create_block(consensus_state, &gov_sequencer_key)
        .expect("Gov block creation failed");

    match tally_votes_and_decide(&proposal.id, target_block_height) {
        Ok(ProposalStatus::Approved) => {
            println!("  -> StarSenate: Proposal ID '{}' APPROVED.", proposal.id);
            stl::update_trust_score(proposer_did_str, stl::GOVERNANCE_CONTEXT, 0.2, "Proposal approved");
            aethercore_runtime::acknowledge_module_upgrade(&target_module_id, "version_1.1.0", &new_code_hash, None).expect("Upgrade ack failed");
            println!("  -> AetherCore: Upgraded module '{}'.", target_module_id);
        }
        Ok(ProposalStatus::Rejected) => { println!("  -> StarSenate: Proposal ID '{}' REJECTED.", proposal.id); stl::update_trust_score(proposer_did_str, stl::GOVERNANCE_CONTEXT, -0.05, "Proposal rejected"); }
        Ok(status) => println!("  -> StarSenate: Proposal ID '{}' status: {:?}", proposal.id, status),
        Err(e) => eprintln!("[GovSim] Error tallying votes: {}", e),
    }
}

fn run_von_simulation_phase(obligor_did_str: &str, obligee_did_str: &str, consensus_state: &mut ConsensusState) {
    let target_block_height_create = get_next_target_block_height(consensus_state);
    println!("\n--- Running Verifiable Obligation Nexus (VON) Simulation Phase (Targeting Block Create: {}) ---", target_block_height_create);
    let due_timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 86400;
    let obligation = von::create_fluxpact_contract(obligor_did_str, obligee_did_str, "Deliver resources", 50, due_timestamp, target_block_height_create).expect("Obligation creation failed");
    println!("  -> VON: Created Obligation ID: '{}'", obligation.id);
    stl::update_trust_score(obligor_did_str, stl::FINANCIAL_CONTEXT, -0.02, "Created obligation");
    
    let von_creation_aurora_tx = AuroraTransaction::TransferAUC(TransferAucPayload { 
        sender_pk_hex: "system_von_marker_pk".to_string(), recipient_pk_hex: "system_von_log_pk".to_string(),
        amount: 0, nonce: get_next_nonce_for_sender("system_von_marker_pk"),
    });
    submit_aurora_transaction(consensus_state, von_creation_aurora_tx).expect("Submit VON create payload failed");
    
    let von_create_sequencer_key = get_simulation_sequencer_key("von_sequencer_create");
    let _von_create_block = sequencer_create_block(consensus_state, &von_create_sequencer_key)
        .expect("VON create block failed");

    let target_block_height_fulfill = get_next_target_block_height(consensus_state);
    let fulfillment_proof_hash = mock_hash_data(&"Proof of delivery");
    match von::attest_obligation_fulfillment(&obligation.id, obligee_did_str, &fulfillment_proof_hash, target_block_height_fulfill ) {
        Ok(()) => {
            println!("  -> VON: Obligation ID '{}' fulfilled.", obligation.id);
            stl::update_trust_score(obligor_did_str, stl::FINANCIAL_CONTEXT, 0.15, "Fulfilled obligation");
            stl::update_trust_score(obligee_did_str, stl::FINANCIAL_CONTEXT, 0.01, "Attested fulfillment");
            
            let von_fulfill_aurora_tx = AuroraTransaction::TransferAUC(TransferAucPayload { 
                sender_pk_hex: "system_von_marker_pk".to_string(), recipient_pk_hex: "system_von_log_pk".to_string(),
                amount: 0, nonce: get_next_nonce_for_sender("system_von_marker_pk"),
            });
            submit_aurora_transaction(consensus_state, von_fulfill_aurora_tx).expect("Submit VON fulfill payload failed");
            
            let von_fulfill_sequencer_key = get_simulation_sequencer_key("von_sequencer_fulfill");
            let _von_fulfill_block = sequencer_create_block(consensus_state, &von_fulfill_sequencer_key)
                .expect("VON fulfill block failed");
        }
        Err(e) => eprintln!("[VONSim] Error attesting fulfillment: {}", e),
    }
}

fn run_ecological_simulation_phase(green_validator_did_pk_hex: &str, reward_funder_pk_hex: &str, consensus_state: &mut ConsensusState) {
    let target_block_height_credit = get_next_target_block_height(consensus_state);
    println!("\n--- Running Ecological Simulation Phase for DID {} (Targeting Block Credit: {}) ---", green_validator_did_pk_hex, target_block_height_credit);
    let operation_description = format!("Validated block #{} with green energy", target_block_height_credit);
    match process_green_operation_attestation(green_validator_did_pk_hex, &operation_description, 5, target_block_height_credit) {
        Ok(credit) => {
            println!("  -> GaiaPulse: Minted EcoCredit for {} tons", credit.amount_co2e_sequestered_tons);
            stl::update_trust_score(green_validator_did_pk_hex, stl::GOVERNANCE_CONTEXT, 0.02, "Performed green op");
            
            let eco_credit_aurora_tx = AuroraTransaction::TransferAUC(TransferAucPayload { 
                sender_pk_hex: "system_eco_marker_pk".to_string(), recipient_pk_hex: "system_eco_log_pk".to_string(),
                amount: 0, nonce: get_next_nonce_for_sender("system_eco_marker_pk"),
            });
            submit_aurora_transaction(consensus_state, eco_credit_aurora_tx).expect("Submit EcoCredit payload failed");
            
            let eco_credit_sequencer_key = get_simulation_sequencer_key("eco_sequencer_credit");
            let _eco_block = sequencer_create_block(consensus_state, &eco_credit_sequencer_key)
                .expect("Eco block creation failed");
        }
        Err(e) => eprintln!("[EcoSim] Error processing green attestation: {}", e),
    }

    let target_block_height_reward = get_next_target_block_height(consensus_state);
    let op_id_for_reward = format!("block_proposal_{}", target_block_height_reward);
    let next_funder_nonce = get_next_nonce_for_sender(reward_funder_pk_hex);

    if let Ok(Some(boost)) = calculate_and_distribute_fluxboost_reward(
        green_validator_did_pk_hex, 100, &op_id_for_reward, 
        target_block_height_reward, reward_funder_pk_hex, next_funder_nonce
    ) { 
        println!("  -> EcoNova: FluxBoost of {} distributed.", boost); 
         let fluxboost_placeholder_tx = AuroraTransaction::TransferAUC(TransferAucPayload {
            sender_pk_hex: "system_eco_reward_marker".to_string(), recipient_pk_hex: "log".to_string(),
            amount: 0, nonce: get_next_nonce_for_sender("system_eco_reward_marker"),
        });
        submit_aurora_transaction(consensus_state, fluxboost_placeholder_tx).expect("Submit FluxBoost placeholder payload failed");

        let eco_reward_sequencer_key = get_simulation_sequencer_key("eco_sequencer_reward");
        let _eco_reward_block = sequencer_create_block(consensus_state, &eco_reward_sequencer_key)
            .expect("Eco reward block failed");
    }
}

fn run_developer_deployment_phase(developer_did: &str, consensus_state: &mut ConsensusState, wasm_module_crate_name: &str) -> Option<String> {
    let target_block_height = get_next_target_block_height(consensus_state);
    println!("\n--- Running Developer Deployment Simulation Phase for DID {} (Wasm Crate: {}, Targeting Block {}) ---", developer_did, wasm_module_crate_name, target_block_height);
    let wasm_base_path_for_loading = "";
    let compilation_output: MockDappCompilation = match compile_dapp_mock(wasm_module_crate_name, developer_did, wasm_base_path_for_loading) {
        Ok(comp) => comp, Err(e) => { eprintln!("[DevSim] DApp Wasm loading/compilation failed for '{}': {}", wasm_module_crate_name, e); return None; }
    };
    println!("  -> AstroCLI: DApp '{}' (from crate {}) \"compiled\". Wasm Bytecode Hash: {}. Bytecode size: {}", compilation_output.dapp_name, wasm_module_crate_name, compilation_output.mock_wasm_bytecode_hash, compilation_output.wasm_bytecode.len());
    
    let dapp_deploy_aurora_tx = AuroraTransaction::TransferAUC(TransferAucPayload { 
        sender_pk_hex: "system_dev_marker_pk".to_string(), recipient_pk_hex: "system_dev_log_pk".to_string(),
        amount: 0, nonce: get_next_nonce_for_sender("system_dev_marker_pk"),
    });
    submit_aurora_transaction(consensus_state, dapp_deploy_aurora_tx).expect("Submit DAppDeploy placeholder payload failed");
    
    let dev_sequencer_key = get_simulation_sequencer_key("dev_sequencer");
    let _dapp_deploy_intent_block = sequencer_create_block(consensus_state, &dev_sequencer_key)
        .expect("DAppDeploy intent block failed");

    match request_dapp_deployment(compilation_output.clone(), "AetherCore_Target", target_block_height) {
        Ok(deployed_module_id) => {
            println!("  -> AstroCLI: DApp '{}' deployment successful. Deployed (AetherCore) Module ID: '{}'", compilation_output.dapp_name, deployed_module_id);
            stl::update_trust_score(developer_did, stl::GOVERNANCE_CONTEXT, 0.1, &format!("Successfully deployed DApp: {}", compilation_output.dapp_name));
            let gas_limit_for_dapp_exec = 20000;
            let current_block_for_exec = consensus_state.current_height +1; 
            let exec_context = Some(developer_did.to_string());

            if deployed_module_id == "sample_wasm_module_add" {
                println!("\n  --- Attempting to execute Wasm DApp '{}' (function: add) ---", deployed_module_id);
                let exec_req = ExecutionRequest { 
                    module_id: deployed_module_id.clone(), 
                    function_name: "add".to_string(), 
                    arguments: vec![WasmiValue::I32(700), WasmiValue::I32(52)], 
                    gas_limit: gas_limit_for_dapp_exec,
                    execution_context_did: exec_context.clone(),
                };
                if let Ok(res) = aethercore_runtime::execute_module(exec_req, current_block_for_exec) { 
                    println!("  -> AetherCore (sample_add): Success: {}, Output: {:?}, GasConsumed: {}, Logs: {:?}", res.success, res.output_values, res.gas_consumed_total, res.logs); 
                }
            } else if deployed_module_id.starts_with("sample_wasm_host_interaction") { // Use starts_with due to potential UUID
                println!("\n  --- Attempting to execute Wasm DApp '{}' (function: perform_action_and_log) ---", deployed_module_id);
                let exec_req_log = ExecutionRequest { 
                    module_id: deployed_module_id.clone(), 
                    function_name: "perform_action_and_log".to_string(), 
                    arguments: Vec::new(), 
                    gas_limit: gas_limit_for_dapp_exec,
                    execution_context_did: exec_context.clone(),
                };
                if let Ok(res) = aethercore_runtime::execute_module(exec_req_log, current_block_for_exec) { 
                    println!("  -> AetherCore (host_log): Success: {}, Output: {:?}, GasConsumed: {}, Logs: {:?}", res.success, res.output_values, res.gas_consumed_total, res.logs); 
                }
                
                // Test new host calls
                println!("\n  --- Testing new host calls for Wasm DApp '{}' ---", deployed_module_id);
                let exec_req_isn_log = ExecutionRequest {
                    module_id: deployed_module_id.clone(),
                    function_name: "log_message_to_isn".to_string(),
                    arguments: vec![], gas_limit: gas_limit_for_dapp_exec,
                    execution_context_did: exec_context.clone(),
                };
                if let Ok(res) = aethercore_runtime::execute_module(exec_req_isn_log, current_block_for_exec) {
                     println!("  -> AetherCore (log_message_to_isn): Success: {}, Gas: {}, Logs: {:?}", res.success, res.gas_consumed_total, res.logs);
                }
                let exec_req_kv_set = ExecutionRequest {
                    module_id: deployed_module_id.clone(),
                    function_name: "store_data_in_kv".to_string(),
                    arguments: vec![], gas_limit: gas_limit_for_dapp_exec,
                    execution_context_did: exec_context.clone(),
                };
                if let Ok(res) = aethercore_runtime::execute_module(exec_req_kv_set, current_block_for_exec) {
                     println!("  -> AetherCore (store_data_in_kv): Success: {}, Gas: {}, Logs: {:?}", res.success, res.gas_consumed_total, res.logs);
                }
                let exec_req_kv_get = ExecutionRequest {
                    module_id: deployed_module_id.clone(),
                    function_name: "retrieve_and_log_data_from_kv".to_string(),
                    arguments: vec![], gas_limit: gas_limit_for_dapp_exec,
                    execution_context_did: exec_context.clone(),
                };
                if let Ok(res) = aethercore_runtime::execute_module(exec_req_kv_get, current_block_for_exec) {
                    println!("  -> AetherCore (retrieve_and_log_data_from_kv): Success: {}, Gas: {}, Logs: {:?}", res.success, res.gas_consumed_total, res.logs);
                }
            }
            Some(deployed_module_id)
        }
        Err(e) => { eprintln!("[DevSim] DApp deployment failed: {}", e); stl::update_trust_score(developer_did, stl::GOVERNANCE_CONTEXT, -0.1, &format!("Failed DApp deployment: {}", compilation_output.dapp_name)); None }
    }
}

fn run_risk_ethics_simulation_phase(malicious_dev_did: &str, risky_dev_did: &str, normal_dapp_module_id: &str, consensus_state: &mut ConsensusState) {
    let target_block_height = get_next_target_block_height(consensus_state);
    println!("\n--- Running Risk Mitigation & Ethical Oversight Simulation Phase (Targeting Block {}) ---", target_block_height);
    println!("\n  Scenario 1: Developer '{}' attempts to deploy 'malicious_dapp_attempt'...", malicious_dev_did);
    run_developer_deployment_phase(malicious_dev_did, consensus_state, "malicious_dapp_attempt");
    println!("\n  Scenario 2: Developer '{}' attempts to deploy 'risky_dapp_code'...", risky_dev_did);
    run_developer_deployment_phase(risky_dev_did, consensus_state, "risky_dapp_code");
    println!("\n  Scenario 3: Deployed DApp '{}' performs an anomalous operation...", normal_dapp_module_id);
    if normal_dapp_module_id.is_empty() || normal_dapp_module_id == "malicious_dapp_attempt" || normal_dapp_module_id == "risky_dapp_code" { 
        println!("  Skipping anomaly test for '{}' as it's not a valid deployed module for this test.", normal_dapp_module_id); return; 
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
    let target_block_height = get_next_target_block_height(consensus_state);
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
    
    let rw_data_aurora_tx = AuroraTransaction::TransferAUC(TransferAucPayload { 
        sender_pk_hex: "system_reality_marker_pk".to_string(), recipient_pk_hex: "system_reality_log_pk".to_string(),
        amount: 0, nonce: get_next_nonce_for_sender("system_reality_marker_pk"),
    });
    submit_aurora_transaction(consensus_state, rw_data_aurora_tx).expect("Submit RWData placeholder payload failed");
    
    let reality_sequencer_key = get_simulation_sequencer_key("reality_sequencer");
    let _rw_block = sequencer_create_block(consensus_state, &reality_sequencer_key)
        .expect("RWData block failed");

    match generate_prediction_from_isn_data(&isn_data_node.id, "env_pollution_model_v1", get_next_target_block_height(consensus_state)) {
        Ok(prediction) => {
            println!("  -> ChronoForge: Generated Prediction ID: '{}', Type: '{}'", prediction.prediction_id, prediction.prediction_type);
            let zone_a_guardian_did = "did:aurora:eco_guardian_zone_a";
            stl::initialize_entity_trust(zone_a_guardian_did);
            react_to_environmental_prediction(&prediction.prediction_type, &prediction.predicted_value_or_event, prediction.confidence_score, get_next_target_block_height(consensus_state), Some(zone_a_guardian_did));
        }
        Err(e) => eprintln!("[RealitySync] Error generating prediction: {}", e),
    }
}

fn run_isn_graph_query_phase(developer_did: &str, _consensus_state: &ConsensusState) {
    println!("\n--- Running ISN Graph Query Simulation Phase for Developer {} ---", developer_did);
    let query_str = format!("GET_DEPLOYED_MODULES_BY_DEV_DID {}", developer_did);
    println!("  -> ISN Query: {}", query_str);
    match semantic_synapse_interfaces::query_isn(&query_str) {
        Ok(result) => { println!("  -> ISN Query Result (Modules deployed by {}): {}", developer_did, result.data_json); }
        Err(e) => eprintln!("  -> ISN Query Error: {}", e),
    }
}

fn main() {
    println!("=== Aurora Full Lifecycle Simulation (Step 1 & 2 - Wasm Host Calls) ===");
    
    let mut consensus_state = ConsensusState::new("simulation_runner_node".to_string());

    println!("\n--- Running Identity Creation & STL Initialization Phase ---");
    let initial_block_height_for_records = get_next_target_block_height(&consensus_state); 

    let user_punk_pk_hex = create_celestial_id("user_punk_789", "pk_punk", initial_block_height_for_records).unwrap().did;
    let dev_aurora_pk_hex = create_celestial_id("developer_aurora_core_001", "pk_dev_core", initial_block_height_for_records).unwrap().did;
    let voter_alpha_pk_hex = create_celestial_id("voter_alpha_stl_green", "pk_voter_a", initial_block_height_for_records).unwrap().did;
    let dapp_developer_pk_hex = create_celestial_id("dapp_dev_cosmic", "pk_dapp_dev", initial_block_height_for_records).unwrap().did;
    let malicious_dev_pk_hex = create_celestial_id("malicious_dev_007", "pk_mal_dev", initial_block_height_for_records).unwrap().did;
    let risky_dev_pk_hex = create_celestial_id("risky_dev_008", "pk_risky_dev", initial_block_height_for_records).unwrap().did;
    let other_voters_temp_pks: Vec<String> = vec![
        create_celestial_id("voter_beta_stl", "pk_voter_b", initial_block_height_for_records).unwrap().did, 
        create_celestial_id("voter_gamma_stl", "pk_voter_g", initial_block_height_for_records).unwrap().did
    ];
    let other_voters_pks_refs: Vec<&str> = other_voters_temp_pks.iter().map(AsRef::as_ref).collect();
    let obligee_pk_hex = create_celestial_id("obligee_user_001", "pk_obligee", initial_block_height_for_records).unwrap().did;
    
    let system_reward_funder_pk_hex = "did:aurora:system_reward_funder".to_string(); 

    println!("  -> SoulStar: Created DIDs (using them as PK_Hex for simulation).");
    
    let mut all_pks_for_stl_init = vec![
        user_punk_pk_hex.clone(), dev_aurora_pk_hex.clone(), voter_alpha_pk_hex.clone(), 
        dapp_developer_pk_hex.clone(), malicious_dev_pk_hex.clone(), risky_dev_pk_hex.clone(),
        obligee_pk_hex.clone(), system_reward_funder_pk_hex.clone()
    ];
    all_pks_for_stl_init.extend(other_voters_temp_pks.clone());
    
    all_pks_for_stl_init.push("system_gov_marker_pk".to_string());
    all_pks_for_stl_init.push("system_von_marker_pk".to_string());
    all_pks_for_stl_init.push("system_eco_marker_pk".to_string());
    all_pks_for_stl_init.push("system_dev_marker_pk".to_string());
    all_pks_for_stl_init.push("system_reality_marker_pk".to_string());

    for pk_hex_str in all_pks_for_stl_init.iter() {
        stl::initialize_entity_trust(pk_hex_str);
        novavault_flux_finance::ensure_account_exists_with_initial_funds(pk_hex_str);
        get_next_nonce_for_sender(pk_hex_str); 
    }
    
    let identity_sequencer_key = get_simulation_sequencer_key("identity_sequencer");
    let _identity_block = sequencer_create_block(&mut consensus_state, &identity_sequencer_key)
        .expect("Identity block creation failed");

    run_financial_simulation_phase(&user_punk_pk_hex, &dev_aurora_pk_hex, &mut consensus_state);
    run_governance_simulation_phase(&dev_aurora_pk_hex, other_voters_pks_refs.clone(), &mut consensus_state);
    run_von_simulation_phase(&user_punk_pk_hex, &obligee_pk_hex, &mut consensus_state);
    run_ecological_simulation_phase(&voter_alpha_pk_hex, &system_reward_funder_pk_hex, &mut consensus_state);
    
    let deployed_adder_module_id = run_developer_deployment_phase(&dapp_developer_pk_hex, &mut consensus_state, "sample_wasm_module_add")
        .unwrap_or_else(|| "sample_wasm_module_add".to_string()); 
    
    let deployed_host_interaction_module_id = run_developer_deployment_phase(&dapp_developer_pk_hex, &mut consensus_state, "sample_wasm_host_interaction")
        .unwrap_or_else(|| "sample_wasm_host_interaction_sim_fallback".to_string());
    
    if deployed_host_interaction_module_id != "sample_wasm_host_interaction_sim_fallback" {
        println!("\n--- Testing Wasm Host Calls for Module: {} ---", deployed_host_interaction_module_id);
        let current_block_for_exec = consensus_state.current_height + 1;
        let exec_context = Some(dapp_developer_pk_hex.clone());
        let gas_limit_wasm = 500_000;

        let exec_req_isn_log = ExecutionRequest {
            module_id: deployed_host_interaction_module_id.clone(),
            function_name: "log_message_to_isn".to_string(),
            arguments: vec![], gas_limit: gas_limit_wasm,
            execution_context_did: exec_context.clone(),
        };
        if let Ok(res) = aethercore_runtime::execute_module(exec_req_isn_log, current_block_for_exec) {
             println!("  -> AetherCore (log_message_to_isn): Success: {}, Gas: {}, Logs: {:?}", res.success, res.gas_consumed_total, res.logs);
            if !res.success { println!("    Error: {:?}", res.error_message); }
        }

        let exec_req_kv_set = ExecutionRequest {
            module_id: deployed_host_interaction_module_id.clone(),
            function_name: "store_data_in_kv".to_string(),
            arguments: vec![], gas_limit: gas_limit_wasm,
            execution_context_did: exec_context.clone(),
        };
        if let Ok(res) = aethercore_runtime::execute_module(exec_req_kv_set, current_block_for_exec) {
             println!("  -> AetherCore (store_data_in_kv): Success: {}, Gas: {}, Logs: {:?}", res.success, res.gas_consumed_total, res.logs);
             if !res.success { println!("    Error: {:?}", res.error_message); }
        }
        
        let exec_req_kv_get = ExecutionRequest {
            module_id: deployed_host_interaction_module_id.clone(),
            function_name: "retrieve_and_log_data_from_kv".to_string(),
            arguments: vec![], gas_limit: gas_limit_wasm,
            execution_context_did: exec_context.clone(),
        };
        if let Ok(res) = aethercore_runtime::execute_module(exec_req_kv_get, current_block_for_exec) {
            println!("  -> AetherCore (retrieve_and_log_data_from_kv): Success: {}, Gas: {}, Logs: {:?}", res.success, res.gas_consumed_total, res.logs);
             if !res.success { println!("    Error: {:?}", res.error_message); }
        }
    }

    run_reality_sync_prediction_phase(&voter_alpha_pk_hex, &mut consensus_state);
    run_risk_ethics_simulation_phase(&malicious_dev_pk_hex, &risky_dev_pk_hex, &deployed_adder_module_id, &mut consensus_state);
    run_isn_graph_query_phase(&dapp_developer_pk_hex, &consensus_state);

    println!("\n--- Final Mock STL Scores & NovaVault Balances ---");
    let zone_a_guardian_did = "did:aurora:eco_guardian_zone_a";
    if !all_pks_for_stl_init.contains(&zone_a_guardian_did.to_string()) {
        all_pks_for_stl_init.push(zone_a_guardian_did.to_string());
        stl::initialize_entity_trust(zone_a_guardian_did);
        novavault_flux_finance::ensure_account_exists_with_initial_funds(zone_a_guardian_did);
         get_next_nonce_for_sender(zone_a_guardian_did); 
    }
    for pk_hex_str_owned in all_pks_for_stl_init.iter() {
        println!("  PK_Hex: {}, GovSTL: {:.2}, FinSTL: {:.2}, AUC_Balance: {}, NextNonce: {}", 
            pk_hex_str_owned,
            stl::get_contextual_trust_score(pk_hex_str_owned, stl::GOVERNANCE_CONTEXT),
            stl::get_contextual_trust_score(pk_hex_str_owned, stl::FINANCIAL_CONTEXT),
            novavault_flux_finance::get_account_balance(pk_hex_str_owned).unwrap_or(0),
            SIM_TEMP_NONCE_TRACKER.lock().unwrap().get(pk_hex_str_owned).cloned().unwrap_or(0)
        );
    }
    println!("\n=== Full Simulation Complete ===");
}