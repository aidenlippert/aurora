#![no_std] // Indicates we are not linking the standard library, common for Wasm

// This function will be exported from our Wasm module.
// wasmi can call functions with i32, i64, f32, f64 parameters and return types.
// We use no_mangle to prevent Rust from changing the function's name.
#[no_mangle]
pub extern "C" fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[no_mangle]
pub extern "C" fn greet_from_wasm(input_ptr: *const u8, input_len: usize) -> i32 {
    // In a real scenario, you'd read the string from memory.
    // For wasmi and this mock, direct string passing is hard without host functions.
    // This is a very simplified "greet".
    // The return value could signify success (0) or an error code.
    // Let's just return the length for now as a mock operation.
    // Printing from Wasm usually requires importing a host function.
    input_len as i32 
}

// Required for no_std crates if they panic
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
