#![allow(unused_variables, dead_code, unused_imports)]
//! AetherCore Runtime: The Universal Wasm Execution Forge for Aurora.
use std::collections::HashMap;
use std::sync::Mutex;
use once_cell::sync::Lazy;
use uuid::Uuid;

use wasmi::{
    Engine, Module, Store, Linker, Caller, Instance, Extern, Value, AsContextMut, Error as WasmiError,
    Memory, MemoryType
};
use wasmi::core::{Trap, TrapCode};

#[derive(Debug, Clone)]
pub struct DeployedModuleInfo {
    pub version: String,
    pub bytecode_hash: String,
    pub wasm_bytecode: Vec<u8>,
}

#[derive(Debug)]
pub struct ExecutionRequest {
    pub module_id: String,
    pub function_name: String,
    pub arguments: Vec<Value>,
    pub gas_limit: u64,
}

#[derive(Debug)]
pub struct ExecutionResult {
    pub output_values: Vec<Value>,
    pub gas_consumed_total: u64,
    pub success: bool,
    pub logs: Vec<String>,
    pub error_message: Option<String>,
}

static ACTIVE_MODULES: Lazy<Mutex<HashMap<String, DeployedModuleInfo>>> = Lazy::new(|| {
    let mut initial_modules = HashMap::new();
    initial_modules.insert("mock_contract_v1".to_string(), DeployedModuleInfo {
        version: "version_1.0.0".to_string(),
        bytecode_hash: "legacy_hash_mock_v1".to_string(),
        wasm_bytecode: Vec::new(),
    });
    initial_modules.insert("private_auc_handler_v1".to_string(), DeployedModuleInfo {
        version: "version_1.0.0".to_string(),
        bytecode_hash: "legacy_hash_private_auc_v1".to_string(),
        wasm_bytecode: Vec::new(),
    });
    Mutex::new(initial_modules)
});

#[derive(Default)]
pub struct HostState {
    pub logs: Vec<String>,
    pub module_id_for_log: String,
    pub host_gas_remaining: u64,
    pub host_function_trap: Option<Trap>,
}

impl HostState {
    fn consume_host_gas(&mut self, amount: u64) -> Result<(), ()> {
        if self.host_gas_remaining >= amount {
            self.host_gas_remaining -= amount;
            Ok(())
        } else {
            self.host_gas_remaining = 0;
            let trap_msg = format!("Out of gas for host function in module {}!", self.module_id_for_log);
            println!("[HostState:Gas] {}", trap_msg);
            self.host_function_trap = Some(Trap::new(trap_msg));
            Err(())
        }
    }
}

fn host_log_message_adapter(mut caller: Caller<'_, HostState>, ptr: u32, len: u32) {
    if caller.data_mut().consume_host_gas(10).is_err() { return; }
    let memory = match caller.get_export("memory") {
        Some(Extern::Memory(mem)) => mem,
        _ => { 
            let trap_msg = "[HostFuncError] 'memory' export not found for logging.".to_string();
            println!("{}", trap_msg);
            caller.data_mut().host_function_trap = Some(Trap::new(trap_msg));
            return; 
        }
    };
    let mut buffer = vec![0u8; len as usize];
    if memory.read(&caller, ptr as usize, &mut buffer).is_err() {
        let trap_msg = "[HostFuncError] Failed to read Wasm memory for logging.".to_string();
        println!("{}", trap_msg);
        caller.data_mut().host_function_trap = Some(Trap::new(trap_msg));
        return;
    }
    match String::from_utf8(buffer) {
        Ok(message_str) => {
            let log_entry = format!("[WasmLog:{}] {}", caller.data().module_id_for_log, message_str);
            println!("{}", log_entry);
            caller.data_mut().logs.push(log_entry);
        }
        Err(_) => { 
            let trap_msg = "[HostFuncError] Log message not valid UTF-8.".to_string();
            println!("{}", trap_msg);
            caller.data_mut().host_function_trap = Some(Trap::new(trap_msg));
            return;
        }
    }
}

pub fn execute_module(request: ExecutionRequest) -> Result<ExecutionResult, String> {
    println!(
        "[AetherCore] Executing module '{}', fn '{}', GasLimit: {}",
        request.module_id, request.function_name, request.gas_limit
    );

    let active_modules_db = ACTIVE_MODULES.lock().unwrap();
    let module_info = active_modules_db.get(&request.module_id).cloned()
        .ok_or_else(|| format!("Module '{}' not found.", request.module_id))?;
    drop(active_modules_db);

    println!("[AetherCore] Executing module {} (Version: {})", request.module_id, module_info.version);

    if module_info.wasm_bytecode.is_empty() {
        let (output, gas_consumed, success, logs) = match (request.module_id.as_str(), request.function_name.as_str()) {
            ("mock_contract_v1", "process_payment") => (vec![Value::I64(1000)], 1000, true, vec!["Lgcy:PayInit".to_string()]),
            ("private_auc_handler_v1", "log_private_op_intent") => (vec![Value::I64(200)], 200, true, vec!["Lgcy:PrivOpLog".to_string()]),
            _ => return Err(format!("Unknown legacy fn {}::{}", request.module_id, request.function_name)),
        };
        if request.gas_limit < gas_consumed {
            return Ok(ExecutionResult { output_values: Vec::new(), gas_consumed_total: request.gas_limit, success: false, logs, error_message: Some("Out of gas (legacy)".to_string()) });
        }
        return Ok(ExecutionResult { output_values: output, gas_consumed_total: gas_consumed, success, logs, error_message: None });
    }

    let engine = Engine::default();
    let module = Module::new(&engine, &module_info.wasm_bytecode[..]).map_err(|e| format!("Wasm parse error: {}", e))?;
    
    let mut linker = Linker::new(&engine);
    linker.func_wrap("env", "host_log_message", host_log_message_adapter).map_err(|e| format!("Linker error: {}", e))?;

    let host_state = HostState { module_id_for_log: request.module_id.clone(), host_gas_remaining: request.gas_limit, ..Default::default() };
    let mut store = Store::new(&engine, host_state);
    
    store.add_fuel(request.gas_limit).map_err(|e| format!("Set fuel: {:?}", e))?;

    let instance = linker.instantiate(&mut store, &module)
        .and_then(|pre| pre.start(&mut store))
        .map_err(|e| format!("Wasm instantiate error: {:?}", e))?;
    let func = instance.get_func(&mut store, &request.function_name).ok_or_else(|| format!("Function '{}' not found", request.function_name))?;

    let func_type = func.ty(&store);
    let mut results_buffer = vec![Value::I32(0); func_type.results().len()];

    println!("[AetherCore] Invoking Wasm fn '{}' for '{}'", request.function_name, request.module_id);

    let call_outcome = func.call(&mut store, &request.arguments, &mut results_buffer);
    
    let wasm_fuel_consumed_by_opcodes = match store.fuel_consumed() {
        Some(fuel) => fuel,
        None => 0, // Should not happen if add_fuel was called and successful
    };
    let final_host_state = store.into_data();
    let host_gas_consumed_by_host_functions = request.gas_limit.saturating_sub(final_host_state.host_gas_remaining);
    // total_gas_consumed needs to be careful not to double count if wasmi's fuel includes host calls already.
    // For this mock, wasmi's `add_fuel` and `fuel_consumed` are for Wasm opcodes.
    // Our `host_gas_remaining` is a separate counter for host function costs.
    let total_gas_consumed = wasm_fuel_consumed_by_opcodes + host_gas_consumed_by_host_functions;


    match call_outcome {
        Ok(()) => {
            if let Some(trap) = final_host_state.host_function_trap {
                eprintln!("[AetherCore] Host function trap for '{}': {:?}. Gas: {}", request.module_id, trap, total_gas_consumed);
                return Ok(ExecutionResult { output_values: Vec::new(), gas_consumed_total: total_gas_consumed, success: false, logs: final_host_state.logs, error_message: Some(format!("Host function trap: {:?}", trap))});
            }
            println!("[AetherCore] Wasm exec OK for '{}'. Results: {:?}. TotalGas: {}", request.module_id, results_buffer, total_gas_consumed);
            Ok(ExecutionResult { output_values: results_buffer, gas_consumed_total: total_gas_consumed, success: true, logs: final_host_state.logs, error_message: None })
        }
        Err(wasmi_error) => {
            let error_msg_str = final_host_state.host_function_trap.as_ref().map_or_else(
                || format!("Wasm Error: {:?}", wasmi_error),
                |trap_from_host| format!("Host function trap: {:?}",trap_from_host) // Prioritize host trap message
            );
            
            // Corrected fuel exhaustion check using matches!
            let final_gas_consumed = if matches!(&wasmi_error, WasmiError::Trap(t) if matches!(t.trap_code(), Some(TrapCode::OutOfFuel))) || final_host_state.host_function_trap.is_some() {
                request.gas_limit 
            } else {
                total_gas_consumed 
            };

            eprintln!("[AetherCore] Wasm TRAP/Error for '{}': {}. FinalGasConsumed: {}", request.module_id, error_msg_str, final_gas_consumed);
            Ok(ExecutionResult { output_values: Vec::new(), gas_consumed_total: final_gas_consumed, success: false, logs: final_host_state.logs, error_message: Some(error_msg_str) })
        }
    }
}

pub fn deploy_module( /* ... same ... */
    module_id_suggestion: &str, bytecode_hash: &str, version: &str, wasm_bytecode: Vec<u8>,
) -> Result<String, String> {
    let module_id = if ACTIVE_MODULES.lock().unwrap().contains_key(module_id_suggestion) {
        format!("{}_{}", module_id_suggestion, Uuid::new_v4().as_simple())
    } else {
        module_id_suggestion.to_string()
    };
    println!("[AetherCore] Deploying Wasm module. Suggested ID/Name: '{}', Assigned ID: '{}', Hash: {}, Version: {}, Bytecode size: {} bytes",
        module_id_suggestion, module_id, bytecode_hash, version, wasm_bytecode.len());
    let engine = Engine::default();
    if wasm_bytecode.is_empty() {
        println!("[AetherCore] Warning: Deploying module '{}' with empty Wasm bytecode.", module_id);
    } else if let Err(e) = Module::new(&engine, &wasm_bytecode[..]) {
        return Err(format!("Invalid Wasm bytecode for module {}: {}", module_id, e));
    }
    let module_info = DeployedModuleInfo { version: version.to_string(), bytecode_hash: bytecode_hash.to_string(), wasm_bytecode };
    ACTIVE_MODULES.lock().unwrap().insert(module_id.clone(), module_info);
    println!("[AetherCore] Wasm module '{}' successfully deployed.", module_id);
    Ok(module_id)
}
pub fn acknowledge_module_upgrade( /* ... same ... */
    module_id: &str, new_version_info: &str, changes_hash: &str, new_bytecode: Option<Vec<u8>>,
) -> Result<(), String> {
    println!("[AetherCore] Acknowledging upgrade for module ID: '{}'. New version: '{}'. Changes hash: '{}'.",
        module_id, new_version_info, changes_hash);
    let mut active_modules_db = ACTIVE_MODULES.lock().unwrap();
    if let Some(module_data) = active_modules_db.get_mut(module_id) {
        module_data.version = new_version_info.to_string();
        module_data.bytecode_hash = changes_hash.to_string();
        if let Some(bytecode) = new_bytecode {
            if bytecode.is_empty() {
                println!("[AetherCore] Warning: Upgrading module '{}' with empty Wasm bytecode.", module_id);
                module_data.wasm_bytecode = bytecode;
            } else {
                let engine = Engine::default();
                if let Err(e) = Module::new(&engine, &bytecode[..]) {
                    return Err(format!("Invalid Wasm bytecode for upgrade of module {}: {}", module_id, e));
                }
                module_data.wasm_bytecode = bytecode;
                println!("[AetherCore] Module {} bytecode updated.", module_id);
            }
        }
        println!("[AetherCore] Module {} successfully upgraded to {} (mock).", module_id, new_version_info);
        Ok(())
    } else {
        Err(format!("Module {} not found for upgrade.", module_id))
    }
}

pub fn status() -> &'static str { /* ... same ... */
    let crate_name = "aethercore_runtime";
    println!("[{}] placeholder_function called (mock status)", crate_name);
    "skeleton operational (mock)"
}
