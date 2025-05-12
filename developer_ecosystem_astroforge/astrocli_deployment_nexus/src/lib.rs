#![allow(unused_variables, dead_code, unused_imports)]
//! AstroCLI & Deployment Nexus: Streamlined Creation Flow.
use std::collections::HashMap;
use std::fs; // For reading Wasm file
use std::path::Path;

use aethercore_runtime::deploy_module as aethercore_deploy_module;
// MockWasmInstruction is no longer needed here as AetherCore takes Vec<u8>
use starforge_grants::approve_deployment_request;
use cosmic_data_constellation::{IsnNode, record_module_deployment};

use sha2::{Sha256, Digest};
use hex;
use uuid;

#[derive(Debug, Clone)]
pub struct MockDappCompilation {
    pub dapp_name: String,
    pub mock_wasm_bytecode_hash: String, // Hash of the Wasm bytecode
    pub developer_did: String,
    pub wasm_bytecode: Vec<u8>, // The actual Wasm bytecode
}

#[derive(Debug)]
pub struct DeploymentRequest {
    pub request_id: String,
    pub dapp_name: String,
    pub developer_did: String,
    pub mock_wasm_bytecode_hash: String,
    pub deployment_target_info: String,
}

// This function now simulates "compiling" by loading a pre-compiled Wasm file
pub fn compile_dapp_mock(
    dapp_name_for_lookup: &str, // e.g., "sample_wasm_module_add"
    developer_did: &str,
    // Path relative to aurora project root where .wasm files are expected
    wasm_files_base_path: &str,
) -> Result<MockDappCompilation, String> {
    println!("[AstroCLI] \"Compiling\" DApp '{}' for developer DID '{}' (loading pre-compiled Wasm).",
        dapp_name_for_lookup, developer_did);

    // Construct the expected path to the .wasm file
    // Assumes Wasm files are in target/wasm32-unknown-unknown/release/
    let wasm_file_path_str = format!("{}/{}/target/wasm32-unknown-unknown/release/{}.wasm",
                                     wasm_files_base_path, // e.g. "utils/sample_wasm_modules"
                                     dapp_name_for_lookup, // e.g. "sample_wasm_module_add"
                                     dapp_name_for_lookup.replace("-", "_")); // Cargo replaces hyphens with underscores in binary name
    let wasm_file_path = Path::new(&wasm_file_path_str);

    println!("[AstroCLI] Attempting to load Wasm bytecode from: {:?}", wasm_file_path);

    let wasm_bytecode = fs::read(wasm_file_path)
        .map_err(|e| format!("Failed to read Wasm file {:?}: {}", wasm_file_path, e))?;

    let mut hasher = Sha256::new();
    hasher.update(&wasm_bytecode);
    let bytecode_hash = hex::encode(hasher.finalize());

    Ok(MockDappCompilation {
        dapp_name: dapp_name_for_lookup.to_string(),
        mock_wasm_bytecode_hash: bytecode_hash,
        developer_did: developer_did.to_string(),
        wasm_bytecode,
    })
}

pub fn request_dapp_deployment(
    compilation_output: MockDappCompilation,
    target_info: &str,
    current_block_height: u64,
) -> Result<String, String> {
    let request_id = format!("deploy_req_{}", uuid::Uuid::new_v4());
    println!("[AstroCLI] Requesting deployment for DApp '{}' (Bytecode Hash: {}), Request ID: {}. Target: {}",
        compilation_output.dapp_name, compilation_output.mock_wasm_bytecode_hash, request_id, target_info);

    if !starforge_grants::approve_deployment_request( // Corrected function name
        &request_id,
        &compilation_output.developer_did,
        &compilation_output.dapp_name,
        &compilation_output.mock_wasm_bytecode_hash,
        current_block_height
    ) {
        let msg = format!("[AstroCLI] Deployment request '{}' for DApp '{}' REJECTED by StarForge/Ethical Review.", request_id, compilation_output.dapp_name);
        println!("{}", msg);
        return Err(msg);
    }
    println!("[AstroCLI] Deployment request '{}' for DApp '{}' APPROVED.", request_id, compilation_output.dapp_name);

    // Pass the actual Wasm bytecode to AetherCore
    match aethercore_deploy_module(
        &compilation_output.dapp_name,
        &compilation_output.mock_wasm_bytecode_hash,
        "version_1.0.0_real_wasm", // New version string
        compilation_output.wasm_bytecode // Pass the Vec<u8>
    ) {
        Ok(deployed_module_id) => {
            println!("[AstroCLI] DApp '{}' successfully deployed to AetherCore. Assigned Module ID: {}",
                compilation_output.dapp_name, deployed_module_id);

            let mut details = HashMap::new();
            details.insert("developer_did".to_string(), compilation_output.developer_did);
            details.insert("wasm_bytecode_hash".to_string(), compilation_output.mock_wasm_bytecode_hash);
            details.insert("deployment_target".to_string(), target_info.to_string());
            details.insert("version".to_string(), "version_1.0.0_real_wasm".to_string());

            match record_module_deployment(&deployed_module_id, &compilation_output.dapp_name, current_block_height, details) {
                Ok(isn_node) => println!("[AstroCLI] Module deployment for ID '{}' recorded in ISN. Node ID: {}", deployed_module_id, isn_node.id),
                Err(e) => eprintln!("[AstroCLI] Error recording module deployment for ID '{}' in ISN: {}", deployed_module_id, e),
            }
            Ok(deployed_module_id)
        }
        Err(e) => {
            let msg = format!("[AstroCLI] Failed to deploy DApp '{}' to AetherCore: {}", compilation_output.dapp_name, e);
            eprintln!("{}", msg); Err(msg)
        }
    }
}

pub fn status() -> &'static str {
    let crate_name = "astrocli_deployment_nexus";
    println!("[{}] placeholder_function called (mock status)", crate_name);
    "skeleton operational (mock)"
}
