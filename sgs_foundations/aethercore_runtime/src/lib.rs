#![allow(unused_variables, dead_code, unused_imports)]
//! AetherCore Runtime: The Universal Wasm Execution Forge for Aurora.
use std::collections::HashMap;
use std::sync::Mutex;
use once_cell::sync::Lazy;
use uuid::Uuid;

use wasmi::{
    Engine, Module, Store, Linker, Caller, Instance, Extern, Value, AsContextMut, Error as WasmiError,
    Memory, MemoryType, Trap, TrapCode // Added Trap, TrapCode for explicit trapping
};

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
    pub gas_limit: u64, // New field for gas limit
}

#[derive(Debug)]
pub struct ExecutionResult {
    pub output_values: Vec<Value>,
    pub gas_remaining: u64, // Changed from gas_used to gas_remaining
    pub gas_consumed: u64,  // Explicitly state consumed gas
    pub success: bool,
    pub logs: Vec<String>,
    pub error_message: Option<String>, // For trapping or other errors
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
    pub gas_remaining: u64, // Gas tracking within host state for wasmi ticks
    pub trap_message: Option<String>,
}

impl HostState {
    fn consume_gas(&mut self, amount: u64) -> Result<(), Trap> {
        if self.gas_remaining >= amount {
            self.gas_remaining -= amount;
            Ok(())
        } else {
            self.gas_remaining = 0;
            self.trap_message = Some("Out of gas".to_string());
            println!("[HostState:Gas] Out of gas for module {}!", self.module_id_for_log);
            Err(Trap::new(TrapCode::UnreachableCodeReached)) // Simulate out of gas with a trap
        }
    }
}


fn host_log_message_adapter(mut caller: Caller<'_, HostState>, ptr: u32, len: u32) {
    if caller.data_mut().consume_gas(5).is_err() { // Cost for host call
        // Trap has already been set by consume_gas
        return;
    }
    let memory = match caller.get_export("memory") {
        Some(Extern::Memory(mem)) => mem,
        _ => { caller.data_mut().trap_message = Some("Memory export not found for logging".to_string()); return; }
    };
    let mut buffer = vec![0u8; len as usize];
    if memory.read(&caller, ptr as usize, &mut buffer).is_err() {
        caller.data_mut().trap_message = Some("Failed to read Wasm memory for logging".to_string()); return;
    }
    match String::from_utf8(buffer) {
        Ok(message_str) => {
            let log_entry = format!("[WasmLog:{}] {}", caller.data().module_id_for_log, message_str);
            println!("{}", log_entry);
            caller.data_mut().logs.push(log_entry);
        }
        Err(_) => { caller.data_mut().trap_message = Some("Log message not valid UTF-8".to_string()); return; }
    }
}

pub fn execute_module(request: ExecutionRequest) -> Result<ExecutionResult, String> {
    println!(
        "[AetherCore] Attempting to execute Wasm module '{}', function '{}', Gas Limit: {}",
        request.module_id, request.function_name, request.gas_limit
    );

    let active_modules_db = ACTIVE_MODULES.lock().unwrap();
    let module_info = match active_modules_db.get(&request.module_id) {
        Some(info) => info.clone(),
        None => return Err(format!("Module '{}' not found in AetherCore.", request.module_id)),
    };
    drop(active_modules_db);

    println!("[AetherCore] Executing module {} (Version: {})", request.module_id, module_info.version);

    // --- Legacy Module Handling (Conceptual Gas) ---
    if module_info.wasm_bytecode.is_empty() {
        let mut gas_consumed_legacy = 0;
        let mut logs_legacy = Vec::new();
        let mut output_legacy = Vec::new();
        let mut success_legacy = false;

        if request.module_id == "mock_contract_v1" && request.function_name == "process_payment" {
            gas_consumed_legacy = 1000; // Arbitrary gas for legacy
            logs_legacy.push("Log: Legacy Payment initiated.".to_string());
            output_legacy.push(Value::I64(gas_consumed_legacy as i64)); // Return gas for consistency
            success_legacy = true;
        } else if request.module_id == "private_auc_handler_v1" && request.function_name == "log_private_op_intent" {
            gas_consumed_legacy = 200;
            logs_legacy.push("Log: Legacy Private operation intent logged.".to_string());
            output_legacy.push(Value::I64(gas_consumed_legacy as i64));
            success_legacy = true;
        } else {
            return Err(format!("Unknown function or unexecutable legacy module '{}'", request.module_id));
        }

        if request.gas_limit < gas_consumed_legacy {
            return Ok(ExecutionResult {
                output_values: Vec::new(), gas_remaining: 0, gas_consumed: request.gas_limit, // Consumed all limit
                success: false, logs: logs_legacy, error_message: Some("Out of gas for legacy module".to_string()),
            });
        }
        return Ok(ExecutionResult {
            output_values: output_legacy, gas_remaining: request.gas_limit - gas_consumed_legacy,
            gas_consumed: gas_consumed_legacy, success: success_legacy, logs: logs_legacy, error_message: None,
        });
    }

    // --- Wasmi Execution with Gas ---
    let engine = Engine::default();
    let module = Module::new(&engine, &module_info.wasm_bytecode[..]).map_err(|e| format!("Wasm parse error: {}", e))?;
    
    let mut linker = Linker::new(&engine);
    linker.func_wrap("env", "host_log_message", host_log_message_adapter).map_err(|e| format!("Linker error: {}", e))?;

    let mut host_state = HostState { module_id_for_log: request.module_id.clone(), gas_remaining: request.gas_limit, ..Default::default() };
    let mut store = Store::new(&engine, host_state);
    
    // Configure wasmi store for fuel/gas consumption (instruction counting)
    store.add_fuel(request.gas_limit).map_err(|e| format!("Failed to set initial fuel: {:?}", e))?;


    let instance = linker.instantiate(&mut store, &module)
        .and_then(|pre| pre.start(&mut store)) // .start consumes pre, so no need to drop manually
        .map_err(|e| format!("Wasm instantiation error: {:?}", e))?;

    let func = instance.get_func(&mut store, &request.function_name)
        .ok_or_else(|| format!("Function '{}' not found in module '{}'", request.function_name, request.module_id))?;

    let func_type = func.ty(&store);
    let num_results = func_type.results().len();
    let mut results_buffer = vec![Value::I32(0); num_results];

    println!("[AetherCore] Invoking Wasm (fuel: {}) '{}' for '{}'", store.fuel_consumed().unwrap_or(0) + store.fuel_remaining(&engine).unwrap_or(0), request.function_name, request.module_id);

    let call_result = func.call(&mut store, &request.arguments, &mut results_buffer);
    
    let gas_consumed_by_wasmi = store.fuel_consumed().unwrap_or(0);
    let final_host_state = store.into_data(); // Retrieve host state regardless of outcome

    match call_result {
        Ok(()) => {
            println!("[AetherCore] Wasm exec successful for '{}'. Results: {:?}. Gas consumed by wasmi: {}", request.module_id, results_buffer, gas_consumed_by_wasmi);
            Ok(ExecutionResult {
                output_values: results_buffer,
                gas_remaining: final_host_state.gas_remaining, // This reflects host func gas
                gas_consumed: gas_consumed_by_wasmi + (request.gas_limit - final_host_state.gas_remaining), // wasmi + host
                success: true,
                logs: final_host_state.logs,
                error_message: None,
            })
        }
        Err(wasmi_error) => {
            let mut total_gas_consumed = gas_consumed_by_wasmi + (request.gas_limit - final_host_state.gas_remaining);
            let error_msg_str = final_host_state.trap_message.clone().unwrap_or_else(|| format!("Wasm Trap: {:?}", wasmi_error));
             if matches!(wasmi_error, WasmiError::Trap(ref trap) if trap.trap_code() == Some(TrapCode::UnreachableCodeReached)) && final_host_state.trap_message.as_deref() == Some("Out of gas") {
                 // If it's our specific out of gas trap from host function
                 total_gas_consumed = request.gas_limit; // All gas limit consumed
             } else if matches!(wasmi_error, WasmiError::Trap(ref trap) if trap.trap_code() == Some(TrapCode::OutOfFuel)) {
                 // If it's wasmi's own OutOfFuel trap
                 total_gas_consumed = request.gas_limit;
             }


            eprintln!("[AetherCore] Wasm execution TRAP/Error for '{}': {}. Gas consumed: {}", request.module_id, error_msg_str, total_gas_consumed);
            Ok(ExecutionResult { // Return Ok with success: false for traps, as per many runtimes
                output_values: Vec::new(),
                gas_remaining: 0,
                gas_consumed: total_gas_consumed,
                success: false,
                logs: final_host_state.logs,
                error_message: Some(error_msg_str),
            })
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
