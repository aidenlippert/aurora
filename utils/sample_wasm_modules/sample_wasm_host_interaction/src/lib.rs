#![no_std]
// No #![feature(alloc_error_handler)]

extern crate alloc; // Required for alloc::string::String and alloc::format!

use wee_alloc::WeeAlloc;

#[global_allocator]
static ALLOC: WeeAlloc = WeeAlloc::INIT;

extern "C" {
    fn host_log_message(ptr: *const u8, len: usize);
    fn host_isn_log(message_ptr: *const u8, message_len: usize);
    fn host_kv_store_set(key_ptr: *const u8, key_len: usize, val_ptr: *const u8, val_len: usize);
    fn host_kv_store_get(key_ptr: *const u8, key_len: usize, out_val_ptr: *mut u8, out_val_max_len: u32) -> i32;
}

#[no_mangle]
pub extern "C" fn perform_action_and_log() -> i32 {
    let message = "Hello from Wasm! Logging via host_log_message.";
    unsafe {
        host_log_message(message.as_ptr(), message.len());
    }
    42
}

#[no_mangle]
pub extern "C" fn process_and_log_value(value: i32) -> i32 {
    let log_message: alloc::string::String = if value > 100 {
        alloc::format!("Wasm says: Value {} (> 100) received.", value)
    } else {
        alloc::format!("Wasm says: Value {} (<= 100) received.", value)
    };
    unsafe {
        host_log_message(log_message.as_ptr(), log_message.len());
    }
    value * 2
}

#[no_mangle]
pub extern "C" fn log_message_to_isn() -> i32 {
    let isn_message = "This message is logged from Wasm directly to ISN!";
    unsafe {
        host_isn_log(isn_message.as_ptr(), isn_message.len());
    }
    100 
}

#[no_mangle]
pub extern "C" fn store_data_in_kv() -> i32 {
    let key = "my_wasm_key";
    let value = "Stored by Wasm in KV store!";
    unsafe {
        host_kv_store_set(key.as_ptr(), key.len(), value.as_ptr(), value.len());
    }
    200 
}

#[no_mangle]
pub extern "C" fn retrieve_and_log_data_from_kv() -> i32 {
    let key_to_get = "my_wasm_key";
    let mut output_buffer: [u8; 64] = [0; 64]; 
    let log_final_message: alloc::string::String; // Moved declaration up

    let bytes_written = unsafe {
        host_kv_store_get(
            key_to_get.as_ptr(), 
            key_to_get.len(), 
            output_buffer.as_mut_ptr(), 
            output_buffer.len() as u32
        )
    };

    if bytes_written > 0 {
        match core::str::from_utf8(&output_buffer[..bytes_written as usize]) {
            Ok(s) => {
                log_final_message = alloc::format!("Wasm KV Get Result for key '{}': {}", key_to_get, s);
            }
            Err(_) => {
                let hex_val_vec: alloc::vec::Vec<alloc::string::String> = output_buffer[..bytes_written as usize].iter().map(|b| alloc::format!("{:02x}", b)).collect();
                let hex_val = hex_val_vec.join("");
                log_final_message = alloc::format!("Wasm KV Get Result for key '{}': Bytes(non-UTF8): {}", key_to_get, hex_val);
            }
        }
    } else if bytes_written == 0 {
         log_final_message = alloc::format!("Wasm KV Get: Key '{}' NOT FOUND in KV", key_to_get);
    } else { 
         log_final_message = alloc::format!("Wasm KV Get: Host error for key '{}'", key_to_get);
    }
    
    unsafe {
        host_log_message(log_final_message.as_ptr(), log_final_message.len());
    }
    
    300
}

// Universal panic handler for no_std environments (including wasm32)
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // For Wasm, unreachable is the typical panic behavior.
    // For other no_std targets, this would also halt.
    #[cfg(target_arch = "wasm32")]
    {
        core::arch::wasm32::unreachable();
    }
    // For non-wasm32 no_std targets (if this code were ever compiled for one,
    // though it's intended as a cdylib for wasm32).
    #[cfg(not(target_arch = "wasm32"))]
    {
        loop {}
    }
}