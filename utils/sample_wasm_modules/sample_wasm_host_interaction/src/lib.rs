#![no_std]

extern "C" {
    fn host_log_message(ptr: *const u8, len: usize);
}

#[no_mangle]
pub extern "C" fn perform_action_and_log() -> i32 {
    let message = "Hello from Wasm! Logging via host.";
    unsafe {
        host_log_message(message.as_ptr(), message.len());
    }
    42
}

#[no_mangle]
pub extern "C" fn process_and_log_value(value: i32) -> i32 {
    let message_str: &str = if value > 100 {
        "Wasm says: Value > 100 received by host."
    } else {
        "Wasm says: Value <= 100 received by host."
    };
    unsafe {
        host_log_message(message_str.as_ptr(), message_str.len());
    }
    value * 2
}

// Panic handler for actual Wasm compilation
#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic_wasm(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

// Panic handler for no_std host builds (e.g., cargo build --all checks)
#[cfg(not(target_arch = "wasm32"))]
#[panic_handler]
fn panic_host(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}