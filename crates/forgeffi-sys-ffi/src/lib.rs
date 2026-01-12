#![allow(unsafe_code)]

#[unsafe(no_mangle)]
pub extern "C" fn tool_sys_ffi_abi_version() -> u32 {
    1
}

