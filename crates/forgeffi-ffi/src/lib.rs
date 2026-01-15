#![allow(unsafe_code)]

#[cfg(feature = "net")]
pub use forgeffi_net_ffi::*;

#[cfg(feature = "fs")]
pub use forgeffi_fs_ffi::*;

#[cfg(feature = "sys")]
pub use forgeffi_sys_ffi::*;

#[unsafe(no_mangle)]
pub extern "C" fn tool_ffi_abi_version() -> u32 {
    forgeffi_base::ABI_VERSION
}

