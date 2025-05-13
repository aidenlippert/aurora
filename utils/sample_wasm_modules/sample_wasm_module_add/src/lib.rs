#![no_std] 

#[no_mangle]
pub extern "C" fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[no_mangle]
pub extern "C" fn greet_from_wasm(_input_ptr: *const u8, input_len: usize) -> i32 {
    input_len as i32 
}

// Panic handler for actual Wasm compilation
#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic_wasm(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

// Panic handler for no_std host builds (e.g., cargo build --all checks)
// when panic = "abort" is active. This prevents the "panic_handler required" error.
#[cfg(not(target_arch = "wasm32"))]
#[panic_handler]
fn panic_host(_info: &core::panic::PanicInfo) -> ! {
    // For host no_std abort, you might want to do something minimal
    // or just loop. For a library that's not meant to be run directly on host no_std,
    // looping is fine to satisfy the compiler.
    // In a real no_std host scenario, you might use core::intrinsics::abort();
    // but that requires nightly.
    loop {} 
}