// We don't need to explicitly import these if they are only used for type annotations
// on function results that are immediately destructured or their types inferred.
// However, for clarity, we can keep the ones that define structs we pattern match or create.
use aethercore_runtime::ExecutionRequest;
// ConsensusTransaction and Block are used for type annotation of variables
use ecliptic_concordance::{Transaction as ConsensusTransaction, Block};
// IsnNode is used for type annotation
use cosmic_data_constellation::IsnNode;

use std::collections::HashMap;
use sha2::{Sha256, Digest};
use hex;

// Helper to create a mock hash of a struct (very basic)
fn mock_hash_struct<T: std.fmt::Debug>(data: &T) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{:?}", data).as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}


fn main() {
    println!("=== Aurora Minimal Transaction Lifecycle Simulation ===");

    // 1. User initiates an operation via NebulaPulse Swarm
    let originator = "user_punk_123";
    let operation_data = b"{\"recipient\":\"user_cosmic_456\", \"amount\":100, \"asset\":\"AUC\"}".to_vec();
    // nebula_pulse_swarm::OperationPayload type is inferred here
    let initiated_op = match nebula_pulse_swarm::initiate_operation(originator, "payment_v1", operation_data) {
        Ok(op) => op,
        Err(e) => {
            eprintln!("Error initiating operation: {}", e);
            return;
        }
    };
    println!("  -> NebulaPulse: Operation initiated: Type '{}', Originator '{}'", initiated_op.operation_type, initiated_op.originator_id);

    let _edge_processing_id = match nebula_pulse_swarm::send_data_to_edge(&initiated_op) {
        Ok(id) => id,
        Err(e) => {
            eprintln!("Error sending data to edge: {}", e);
            return;
        }
    };

    // 2. AetherCore Runtime executes the "smart contract" for the operation
    let exec_request = ExecutionRequest {
        module_id: "mock_contract_v1".to_string(),
        function_name: "process_payment".to_string(),
        arguments: initiated_op.data.clone(),
    };
    // aethercore_runtime::ExecutionResult type is inferred here
    let execution_result = match aethercore_runtime::execute_module(exec_request) {
        Ok(res) => res,
        Err(e) => {
            eprintln!("Error executing module: {}", e);
            return;
        }
    };
    println!("  -> AetherCore: Module executed. Success: {}, Gas: {}, Output: {:?}",
        execution_result.success,
        execution_result.gas_used,
        String::from_utf8_lossy(&execution_result.output)
    );
    for log_msg in execution_result.logs.iter() {
        println!("     AetherCore Log: {}", log_msg);
    }

    if !execution_result.success {
        println!("Execution failed. Aborting simulation.");
        return;
    }

    let exec_result_summary = format!("{:?}", execution_result);
    let exec_result_hash = mock_hash_struct(&exec_result_summary);

    // 3. Ecliptic Concordance achieves consensus on the operation's result
    let consensus_tx: ConsensusTransaction = match ecliptic_concordance::submit_for_consensus(exec_result_hash.clone()) {
        Ok(tx) => tx,
        Err(e) => {
            eprintln!("Error submitting for consensus: {}", e);
            return;
        }
    };
    println!("  -> EclipticConcordance: Submitted for consensus. TxID: '{}', PayloadHash: '{}'", consensus_tx.id, consensus_tx.payload_hash);

    let finalized_block: Block = match ecliptic_concordance::form_and_finalize_block(vec![consensus_tx.clone()]) {
        Ok(block) => block,
        Err(e) => {
            eprintln!("Error forming/finalizing block: {}", e);
            return;
        }
    };
    println!("  -> EclipticConcordance: Block finalized. ID: '{}', Height: {}", finalized_block.id, finalized_block.height);

    // 4. Cosmic Data Constellation (ISN) records the confirmed operation
    let mut op_details = HashMap::new();
    op_details.insert("module_executed".to_string(), "mock_contract_v1".to_string());
    op_details.insert("function_called".to_string(), "process_payment".to_string());
    op_details.insert("execution_output_hash".to_string(), exec_result_hash);
    op_details.insert("raw_input_data".to_string(), String::from_utf8_lossy(&initiated_op.data).to_string());

    let isn_record: IsnNode = match cosmic_data_constellation::record_confirmed_operation(
        &initiated_op.operation_type,
        &initiated_op.originator_id,
        &consensus_tx.id,
        finalized_block.height,
        op_details,
    ) {
        Ok(node) => node,
        Err(e) => {
            eprintln!("Error recording to ISN: {}", e);
            return;
        }
    };
    println!("  -> ISN_CDC: Operation recorded. Node ID: '{}', Type: '{}'", isn_record.id, isn_record.r#type);

    if let Some(retrieved_node) = cosmic_data_constellation::get_isn_node(&isn_record.id) {
        println!("  -> ISN_CDC: Successfully retrieved node: {:?}", retrieved_node);
    } else {
        println!("  -> ISN_CDC: Failed to retrieve node '{}'", isn_record.id);
    }

    println!("\n=== Simulation Complete ===");
}
