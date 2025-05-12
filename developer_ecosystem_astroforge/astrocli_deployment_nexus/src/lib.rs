#![allow(unused_variables, dead_code, unused_imports)]
//! AstroCLI & Deployment Nexus: Streamlined Creation Flow.
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use aethercore_runtime::deploy_module as aethercore_deploy_module;
use starforge_grants::approve_deployment_request;
// Import create_isn_edge from cosmic_data_constellation
use cosmic_data_constellation::{IsnNode, record_module_deployment, create_isn_edge};

use sha2::{Sha256, Digest};
use hex;
use uuid;

#[derive(Debug, Clone)]
pub struct MockDappCompilation { /* ... same ... */
    pub dapp_name: String,
    pub mock_wasm_bytecode_hash: String,
    pub developer_did: String,
    pub wasm_bytecode: Vec<u8>,
}
#[derive(Debug)]
pub struct DeploymentRequest { /* ... same ... */
    pub request_id: String,
    pub dapp_name: String,
    pub developer_did: String,
    pub mock_wasm_bytecode_hash: String,
    pub deployment_target_info: String,
}

pub fn compile_dapp_mock( /* ... same as corrected in previous fix ... */
    wasm_module_crate_name: &str, developer_did: &str, base_path_for_real_wasm: &str,
) -> Result<MockDappCompilation, String> {
    println!("[AstroCLI] \"Compiling\" DApp (from Wasm crate: '{}') for developer DID '{}'.", wasm_module_crate_name, developer_did);
    let mut wasm_bytecode: Vec<u8> = Vec::new();
    let dapp_name_to_register: String = wasm_module_crate_name.to_string();
    if wasm_module_crate_name == "sample_wasm_module_add" || wasm_module_crate_name == "sample_wasm_host_interaction" {
        let wasm_file_path_str = format!("target/wasm32-unknown-unknown/release/{}.wasm", wasm_module_crate_name.replace("-", "_"));
        let wasm_file_path = Path::new(&wasm_file_path_str);
        println!("[AstroCLI] Attempting to load Wasm bytecode from: {:?}", wasm_file_path);
        match fs::read(wasm_file_path) {
            Ok(bytes) => { println!("[AstroCLI] Successfully loaded {} bytes from {:?}", bytes.len(), wasm_file_path); wasm_bytecode = bytes; }
            Err(e) => { eprintln!("[AstroCLI] WARN: Failed to read Wasm file {:?}: {}. Using empty bytecode for {}.", wasm_file_path, e, wasm_module_crate_name); }
        }
    } else { println!("[AstroCLI_Compiler] Generating placeholder (empty) bytecode for DApp named: {}", wasm_module_crate_name); }
    let mut hasher = Sha256::new();
    if !wasm_bytecode.is_empty() { hasher.update(&wasm_bytecode); }
    else { hasher.update(format!("{}_{}", wasm_module_crate_name, uuid::Uuid::new_v4()).as_bytes()); }
    let bytecode_hash = hex::encode(hasher.finalize());
    Ok(MockDappCompilation { dapp_name: dapp_name_to_register, mock_wasm_bytecode_hash: bytecode_hash, developer_did: developer_did.to_string(), wasm_bytecode })
}


pub fn request_dapp_deployment(
    compilation_output: MockDappCompilation,
    target_info: &str,
    current_block_height: u64,
) -> Result<String, String> { // Returns deployed module AetherCore ID
    let request_id = format!("deploy_req_{}", uuid::Uuid::new_v4());
    println!("[AstroCLI] Requesting deployment for DApp '{}' (SourceHash: {}), Request ID: {}. Target: {}",
        compilation_output.dapp_name, compilation_output.mock_wasm_bytecode_hash, request_id, target_info);

    if !starforge_grants::approve_deployment_request(
        &request_id, &compilation_output.developer_did, &compilation_output.dapp_name,
        &compilation_output.mock_wasm_bytecode_hash, current_block_height
    ) {
        let msg = format!("[AstroCLI] Deployment request REJECTED for DApp '{}'.", compilation_output.dapp_name);
        println!("{}", msg); return Err(msg);
    }
    println!("[AstroCLI] Deployment request APPROVED for DApp '{}'.", compilation_output.dapp_name);

    // AetherCore's deploy_module returns the actual module_id it uses (might differ from dapp_name if name clash)
    let deployed_aethercore_module_id = match aethercore_deploy_module(
        &compilation_output.dapp_name, &compilation_output.mock_wasm_bytecode_hash,
        "version_1.0.0_from_astrocli", compilation_output.wasm_bytecode
    ) {
        Ok(id) => id,
        Err(e) => {
            let msg = format!("[AstroCLI] Failed to deploy DApp '{}' to AetherCore: {}", compilation_output.dapp_name, e);
            eprintln!("{}", msg); return Err(msg);
        }
    };
    println!("[AstroCLI] DApp '{}' deployed to AetherCore. Assigned Module ID: {}", compilation_output.dapp_name, deployed_aethercore_module_id);

    let mut details = HashMap::new();
    details.insert("developer_did".to_string(), compilation_output.developer_did.clone());
    details.insert("wasm_bytecode_hash".to_string(), compilation_output.mock_wasm_bytecode_hash.clone());
    // Record the deployment as an ISN Node
    let deployment_isn_node = match record_module_deployment(&deployed_aethercore_module_id, &compilation_output.dapp_name, current_block_height, details) {
        Ok(node) => {
            println!("[AstroCLI] Module deployment for ID '{}' recorded in ISN. Node ID: {}", deployed_aethercore_module_id, node.id);
            node
        },
        Err(e) => { eprintln!("[AstroCLI] Error recording module deployment in ISN: {}", e); return Err(e); }
    };

    // Create an edge: Developer DID --[deployed_by]--> Module Deployment ISN Node
    // Note: Developer DID itself should be an ISN node. We assume it exists.
    // For this mock, we'll use the developer_did string directly as the "from_node_id".
    // A more robust system would fetch the ISN Node ID for the developer's DID.
    match create_isn_edge(
        &compilation_output.developer_did, // Source: Developer's DID string
        &deployment_isn_node.id,        // Target: The ISN node representing the deployment
        "deployed_module",
        HashMap::new(), // No extra properties on the edge for this mock
        current_block_height
    ) {
        Ok(edge) => println!("[AstroCLI] Created ISN edge: Developer '{}' --[deployed_module ({})]-> Deployment Record '{}'",
            compilation_output.developer_did, edge.id, deployment_isn_node.id),
        Err(e) => eprintln!("[AstroCLI] Error creating ISN edge for deployment: {}", e),
    }

    Ok(deployed_aethercore_module_id) // Return the ID AetherCore uses
}

pub fn status() -> &'static str { /* ... same ... */
    let crate_name = "astrocli_deployment_nexus";
    println!("[{}] placeholder_function called (mock status)", crate_name);
    "skeleton operational (mock)"
}
