// sgs_foundations/aethercore_runtime/src/lib.rs
#![allow(unused_variables, dead_code, unused_imports)]
//! AetherCore Runtime: The Universal Wasm Execution Forge for Aurora.
use std::collections::HashMap;
use std::sync::Mutex;
use once_cell::sync::Lazy;
use uuid::Uuid;
use hex; 

use cosmic_data_constellation::{create_isn_edge, record_confirmed_operation}; 

use wasmi::{
    Engine, Module, Store, Linker, Caller, Instance, Extern, Value, 
    AsContextMut, AsContext, Error as WasmiError, Memory, MemoryType, Func, TypedFunc, ExternRef,
    FuncRef // ADDED FuncRef
};
use wasmi::core::{Trap, TrapCode, F32, F64, ValueType as WasmiValueType};


#[derive(Debug, Clone)]
pub struct DeployedModuleInfo {
    pub version: String,
    pub bytecode_hash: String,
    pub wasm_bytecode: Vec<u8>,
    pub kv_store: HashMap<Vec<u8>, Vec<u8>>, 
}

#[derive(Debug)]
pub struct ExecutionRequest {
    pub module_id: String,
    pub function_name: String,
    pub arguments: Vec<Value>, 
    pub gas_limit: u64,
    pub execution_context_did: Option<String>, 
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
    Mutex::new(HashMap::new())
});

#[derive(Default)]
pub struct HostState {
    pub logs: Vec<String>,
    pub module_id_for_log: String,
    pub host_gas_remaining: u64,
    pub host_function_trap: Option<Trap>,
    pub kv_store_temp_access: Option<HashMap<Vec<u8>, Vec<u8>>>, 
    pub originator_did: Option<String>, 
    pub current_block_height: u64, 
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
        _ => { caller.data_mut().host_function_trap = Some(Trap::new("host_log: 'memory' export not found")); return; }
    };
    let mut buffer = vec![0u8; len as usize];
    if memory.read(caller.as_context(), ptr as usize, &mut buffer).is_err() {
        caller.data_mut().host_function_trap = Some(Trap::new("host_log: Failed to read Wasm memory")); return;
    }
    match String::from_utf8(buffer) {
        Ok(message_str) => {
            let log_entry = format!("[WasmLog:{}] {}", caller.data().module_id_for_log, message_str);
            println!("{}", log_entry);
            caller.data_mut().logs.push(log_entry);
        }
        Err(_) => { caller.data_mut().host_function_trap = Some(Trap::new("host_log: Log message not valid UTF-8")); }
    }
}

fn host_isn_log_adapter(mut caller: Caller<'_, HostState>, message_ptr: u32, message_len: u32) {
    if caller.data_mut().consume_host_gas(100).is_err() { return; } 
    let memory = match caller.get_export("memory") {
        Some(Extern::Memory(mem)) => mem,
        _ => { caller.data_mut().host_function_trap = Some(Trap::new("host_isn_log: 'memory' export not found")); return; }
    };
    let mut buffer = vec![0u8; message_len as usize];
    if memory.read(caller.as_context(), message_ptr as usize, &mut buffer).is_err() { 
        caller.data_mut().host_function_trap = Some(Trap::new("host_isn_log: Failed to read Wasm memory for message")); return;
    }
    match String::from_utf8(buffer) {
        Ok(message_str) => {
            let module_id = caller.data().module_id_for_log.clone();
            let originator = caller.data().originator_did.clone().unwrap_or_else(|| "unknown_wasm_originator".to_string());
            let block_height = caller.data().current_block_height;
            let event_id = format!("wasm_log_event_{}", Uuid::new_v4());
            
            let mut details = HashMap::new();
            details.insert("module_id".to_string(), module_id.clone());
            details.insert("log_message".to_string(), message_str.clone());
            details.insert("originator_did".to_string(), originator.clone());

            println!("[Host:ISNLog] Module '{}' (Originator: '{}') logging to ISN: '{}'", module_id, originator, message_str);
            match record_confirmed_operation("WasmModuleLog", &originator, &event_id, block_height, details) {
                Ok(isn_node) => {
                    caller.data_mut().logs.push(format!("[Host:ISNLogSuccess] Logged to ISN node {}", isn_node.id));
                }
                Err(e) => {
                    let err_msg = format!("host_isn_log: Failed to record to ISN: {}", e);
                    eprintln!("[Host:ISNLogError] {}", err_msg);
                    caller.data_mut().host_function_trap = Some(Trap::new(err_msg));
                }
            }
        }
        Err(_) => { caller.data_mut().host_function_trap = Some(Trap::new("host_isn_log: Message not valid UTF-8")); }
    }
}

fn host_kv_store_set_adapter(mut caller: Caller<'_, HostState>, key_ptr: u32, key_len: u32, val_ptr: u32, val_len: u32) {
    if caller.data_mut().consume_host_gas(50 + (key_len + val_len) as u64).is_err() { return; }
    let memory = match caller.get_export("memory") {
        Some(Extern::Memory(mem)) => mem,
        _ => { caller.data_mut().host_function_trap = Some(Trap::new("host_kv_set: 'memory' export not found")); return; }
    };
    let mut key_buf = vec![0u8; key_len as usize];
    let mut val_buf = vec![0u8; val_len as usize];
    if memory.read(caller.as_context(), key_ptr as usize, &mut key_buf).is_err() ||
       memory.read(caller.as_context(), val_ptr as usize, &mut val_buf).is_err() {
        caller.data_mut().host_function_trap = Some(Trap::new("host_kv_set: Failed to read Wasm memory for key/value")); return;
    }
    
    let module_id = caller.data().module_id_for_log.clone();
    println!("[Host:KVSet] Module '{}' SET Key: 0x{} Value: 0x{}", module_id, hex::encode(&key_buf), hex::encode(&val_buf));

    if let Some(kv_store) = caller.data_mut().kv_store_temp_access.as_mut() {
        kv_store.insert(key_buf, val_buf);
    } else {
        caller.data_mut().host_function_trap = Some(Trap::new("host_kv_set: KV store not initialized in host state"));
    }
}

fn host_kv_store_get_adapter(mut caller: Caller<'_, HostState>, key_ptr: u32, key_len: u32, out_val_ptr: u32, out_val_max_len: u32) -> i32 {
    if caller.data_mut().consume_host_gas(50 + key_len as u64).is_err() { return -1; }
    
    let key_buf = { 
        let memory = match caller.get_export("memory") {
            Some(Extern::Memory(mem)) => mem,
            _ => { caller.data_mut().host_function_trap = Some(Trap::new("host_kv_get: 'memory' export not found for key read")); return -1; }
        };
        let mut temp_key_buf = vec![0u8; key_len as usize];
        if memory.read(caller.as_context(), key_ptr as usize, &mut temp_key_buf).is_err() {
            caller.data_mut().host_function_trap = Some(Trap::new("host_kv_get: Failed to read Wasm memory for key")); return -1;
        }
        temp_key_buf
    }; 

    let module_id_clone = caller.data().module_id_for_log.clone(); 
    println!("[Host:KVGet] Module '{}' GET Key: 0x{}", module_id_clone, hex::encode(&key_buf));

    let value_to_write_opt: Option<Vec<u8>> = caller.data().kv_store_temp_access.as_ref()
        .and_then(|kv_store| kv_store.get(&key_buf).cloned());

    if let Some(value_bytes) = value_to_write_opt {
        if value_bytes.len() > out_val_max_len as usize {
            caller.data_mut().host_function_trap = Some(Trap::new(format!("host_kv_get: Output buffer too small. Needed: {}, Max: {}", value_bytes.len(), out_val_max_len)));
            return -1; 
        }
        let memory = match caller.get_export("memory") { 
            Some(Extern::Memory(mem)) => mem,
            _ => { caller.data_mut().host_function_trap = Some(Trap::new("host_kv_get: 'memory' export not found for value write")); return -1; }
        };
        if memory.write(caller.as_context_mut(), out_val_ptr as usize, &value_bytes).is_err() {
             caller.data_mut().host_function_trap = Some(Trap::new("host_kv_get: Failed to write value to Wasm memory"));
             return -1; 
        }
        if caller.data_mut().consume_host_gas(value_bytes.len() as u64).is_err() { return -1; } 
        println!("[Host:KVGet] Module '{}' GET Key: 0x{} Found Value: 0x{}", module_id_clone, hex::encode(&key_buf), hex::encode(&value_bytes));
        return value_bytes.len() as i32;
    } else {
        println!("[Host:KVGet] Module '{}' GET Key: 0x{} NOT FOUND", module_id_clone, hex::encode(&key_buf));
        return 0; 
    }
}

pub fn execute_module(request: ExecutionRequest, current_block_height: u64) -> Result<ExecutionResult, String> {
    println!(
        "[AetherCore] Executing module '{}', fn '{}', GasLimit: {}, Originator DID: {:?}",
        request.module_id, request.function_name, request.gas_limit, request.execution_context_did
    );

    let initial_kv_store_for_exec;
    let wasm_bytecode_clone;
    let module_version_clone;
    { 
        let active_modules_db_read_guard = ACTIVE_MODULES.lock().unwrap(); 
        let module_info_ref = active_modules_db_read_guard.get(&request.module_id)
            .ok_or_else(|| format!("Module '{}' not found.", request.module_id))?;
        initial_kv_store_for_exec = module_info_ref.kv_store.clone();
        wasm_bytecode_clone = module_info_ref.wasm_bytecode.clone();
        module_version_clone = module_info_ref.version.clone();
    } 

    println!("[AetherCore] Executing module {} (Version: {})", request.module_id, module_version_clone);

    if wasm_bytecode_clone.is_empty() {
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
    let module = Module::new(&engine, &wasm_bytecode_clone[..]).map_err(|e| format!("Wasm parse error: {}", e))?;
    
    let mut linker = Linker::new(&engine);
    linker.func_wrap("env", "host_log_message", host_log_message_adapter).map_err(|e| format!("Linker error(log): {}", e))?;
    linker.func_wrap("env", "host_isn_log", host_isn_log_adapter).map_err(|e| format!("Linker error(isn_log): {}", e))?;
    linker.func_wrap("env", "host_kv_store_set", host_kv_store_set_adapter).map_err(|e| format!("Linker error(kv_set): {}", e))?;
    linker.func_wrap("env", "host_kv_store_get", host_kv_store_get_adapter).map_err(|e| format!("Linker error(kv_get): {}", e))?;

    let host_state = HostState { 
        module_id_for_log: request.module_id.clone(), 
        host_gas_remaining: request.gas_limit,
        kv_store_temp_access: Some(initial_kv_store_for_exec), 
        originator_did: request.execution_context_did.clone(),
        current_block_height,
        ..Default::default() 
    };
    let mut store = Store::new(&engine, host_state);
    
    store.add_fuel(request.gas_limit).map_err(|e| format!("Set fuel: {:?}", e))?;

    let instance = linker.instantiate(&mut store, &module)
        .and_then(|pre| pre.start(&mut store))
        .map_err(|e| format!("Wasm instantiate error: {:?}", e))?;
    let func = instance.get_func(&mut store, &request.function_name).ok_or_else(|| format!("Function '{}' not found", request.function_name))?;

    let func_type = func.ty(&store);
    let mut results_buffer: Vec<Value> = func_type.results().iter().map(|value_type| {
        match value_type {
            WasmiValueType::I32 => Value::I32(0),      
            WasmiValueType::I64 => Value::I64(0),      
            WasmiValueType::F32 => Value::F32(F32::from_float(0.0)), 
            WasmiValueType::F64 => Value::F64(F64::from_float(0.0)), 
            WasmiValueType::FuncRef => Value::FuncRef(FuncRef::null()), // Corrected
            WasmiValueType::ExternRef => Value::ExternRef(ExternRef::null()), // Corrected
            other => unimplemented!("Default value for Wasm result type {:?} not yet supported", other),
        }
    }).collect();

    println!("[AetherCore] Invoking Wasm fn '{}' for '{}'", request.function_name, request.module_id);
    let call_outcome = func.call(&mut store, &request.arguments, &mut results_buffer);
    
    let call_outcome_is_ok = call_outcome.is_ok();
    let wasm_trap_code_option = if let Err(WasmiError::Trap(trap)) = &call_outcome {
        trap.trap_code()
    } else {
        None
    };
    
    let wasm_fuel_consumed_by_opcodes = store.fuel_consumed().unwrap_or(0);
    let mut final_host_state_data = store.into_data(); 
    let host_gas_consumed_by_host_functions = request.gas_limit.saturating_sub(final_host_state_data.host_gas_remaining);
    let total_gas_consumed = wasm_fuel_consumed_by_opcodes + host_gas_consumed_by_host_functions;

    if let Some(trap) = final_host_state_data.host_function_trap.take() { 
        eprintln!("[AetherCore] Host function trap for '{}': {:?}. Gas: {}", request.module_id, trap, request.gas_limit);
        return Ok(ExecutionResult { 
            output_values: Vec::new(), 
            gas_consumed_total: request.gas_limit, 
            success: false, 
            logs: final_host_state_data.logs, 
            error_message: Some(format!("Host function trap: {:?}", trap))
        });
    }

    if call_outcome_is_ok {
        println!("[AetherCore] Wasm exec OK for '{}'. Results: {:?}. TotalGas: {}", request.module_id, results_buffer, total_gas_consumed);
        if let Some(modified_kv) = final_host_state_data.kv_store_temp_access.take() {
            let mut active_modules_db_write = ACTIVE_MODULES.lock().unwrap();
            if let Some(persisted_module_info) = active_modules_db_write.get_mut(&request.module_id) {
                persisted_module_info.kv_store = modified_kv;
                println!("[AetherCore] KV store changes committed for module '{}'.", request.module_id);
            } else {
                eprintln!("[AetherCore] CRITICAL: Module '{}' not found during KV commit.", request.module_id);
            }
        }
        Ok(ExecutionResult { output_values: results_buffer, gas_consumed_total: total_gas_consumed, success: true, logs: final_host_state_data.logs, error_message: None })
    } else {
        let wasm_error = call_outcome.err().unwrap(); 
        let error_msg_str = format!("Wasm Error: {:?}", wasm_error);
        
        let final_gas_consumed = if matches!(wasm_trap_code_option, Some(TrapCode::OutOfFuel)) {
            request.gas_limit 
        } else {
            total_gas_consumed 
        };

        eprintln!("[AetherCore] Wasm TRAP/Error for '{}': {}. FinalGasConsumed: {}", request.module_id, error_msg_str, final_gas_consumed);
        Ok(ExecutionResult { output_values: Vec::new(), gas_consumed_total: final_gas_consumed, success: false, logs: final_host_state_data.logs, error_message: Some(error_msg_str) })
    }
}

pub fn deploy_module(
    module_id_suggestion: &str, bytecode_hash: &str, version: &str, wasm_bytecode: Vec<u8>,
) -> Result<String, String> {
    let mut active_modules_db = ACTIVE_MODULES.lock().unwrap();
    let module_id = if active_modules_db.contains_key(module_id_suggestion) {
        format!("{}_{}", module_id_suggestion, Uuid::new_v4().as_simple())
    } else {
        module_id_suggestion.to_string()
    };
    println!("[AetherCore] Deploying Wasm module. ID: '{}', Hash: {}, Version: {}, Size: {} bytes",
        module_id, bytecode_hash, version, wasm_bytecode.len());
    if !wasm_bytecode.is_empty() {
        let engine = Engine::default();
        if let Err(e) = Module::new(&engine, &wasm_bytecode[..]) {
            return Err(format!("Invalid Wasm for module {}: {}", module_id, e));
        }
    } else { println!("[AetherCore] Warning: Deploying module '{}' with empty Wasm bytecode (legacy mock).", module_id); }
    let module_info = DeployedModuleInfo { version: version.to_string(), bytecode_hash: bytecode_hash.to_string(), wasm_bytecode, kv_store: HashMap::new() };
    active_modules_db.insert(module_id.clone(), module_info);
    println!("[AetherCore] Wasm module '{}' successfully deployed.", module_id);
    Ok(module_id)
}

pub fn acknowledge_module_upgrade(
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
    } else { Err(format!("Module {} not found for upgrade.", module_id)) }
}

pub fn status() -> &'static str {
    "AetherCore Runtime Operational (with ISN Log & KV Host Functions)"
}