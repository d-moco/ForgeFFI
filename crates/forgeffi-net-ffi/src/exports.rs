use forgeffi_base::{ErrorCode, ForgeFfiError, ABI_VERSION};

use crate::mem::{write_error_out, write_out};

#[unsafe(no_mangle)]
pub extern "C" fn tool_netif_abi_version() -> u32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn tool_net_ffi_abi_version() -> u32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "C" fn tool_netif_list_json(out_ptr: *mut *mut u8, out_len: *mut usize) -> i32 {
    if out_ptr.is_null() || out_len.is_null() {
        return ErrorCode::InvalidArgument.as_i32();
    }

    match forgeffi_sys::netif::list_json_bytes() {
        Ok(buf) => {
            unsafe {
                write_out(out_ptr, out_len, buf);
            }
            0
        }
        Err(e) => {
            write_error_out(out_ptr, out_len, &e);
            e.code.as_i32()
        }
    }
}

#[unsafe(no_mangle)]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "C" fn tool_netif_apply_json(
    req_ptr: *const u8,
    req_len: usize,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    if out_ptr.is_null() || out_len.is_null() {
        return ErrorCode::InvalidArgument.as_i32();
    }
    if req_ptr.is_null() || req_len == 0 {
        let e = ForgeFfiError::invalid_argument("请求为空");
        write_error_out(out_ptr, out_len, &e);
        return e.code.as_i32();
    }

    let req_bytes = unsafe { std::slice::from_raw_parts(req_ptr, req_len) };
    let req_str = match std::str::from_utf8(req_bytes) {
        Ok(s) => s,
        Err(e) => {
            let err = ForgeFfiError::invalid_argument(format!("请求不是 UTF-8: {e}"));
            write_error_out(out_ptr, out_len, &err);
            return err.code.as_i32();
        }
    };

    match forgeffi_sys::netif::apply_json_bytes(req_str) {
        Ok(buf) => {
            unsafe {
                write_out(out_ptr, out_len, buf);
            }
            0
        }
        Err(e) => {
            write_error_out(out_ptr, out_len, &e);
            e.code.as_i32()
        }
    }
}

#[unsafe(no_mangle)]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "C" fn tool_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        drop(Vec::from_raw_parts(ptr, len, len));
    }
}

