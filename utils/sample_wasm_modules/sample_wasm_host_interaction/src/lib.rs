#![no_std]
// Removed: #![feature(core_intrinsics)]

extern "C" {
    fn host_log_message(ptr: *const u8, len: usize);
}

#[no_mangle]
pub extern "C" fn perform_action_and_log() -> i32 {
    let message = "Hello from Wasm! Logging via host.";
    // This unsafe block is for calling an extern "C" function, which is correct.
    unsafe {
        host_log_message(message.as_ptr(), message.len());
    }
    42
}

#[no_mangle]
pub extern "C" fn process_and_log_value(value: i32) -> i32 {
    // This is a hack for demo string creation in no_std without an allocator.
    // A real Wasm module would need to manage memory for dynamic strings.
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

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // A simple way to satisfy the panic handler in no_std Wasm on stable.
    // The Wasm instance will effectively hang or trap if it panics.
    loop {}
}
