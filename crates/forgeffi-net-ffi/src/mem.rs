use forgeffi_base::{ForgeFfiError, ABI_VERSION};

pub(crate) fn write_error_out(out_ptr: *mut *mut u8, out_len: *mut usize, e: &ForgeFfiError) {
    let v = serde_json::json!({ "abi": ABI_VERSION, "ok": false, "error": e });
    let buf = serde_json::to_vec(&v).unwrap_or_else(|_| b"{\"ok\":false}".to_vec());
    unsafe {
        write_out(out_ptr, out_len, buf);
    }
}

pub(crate) unsafe fn write_out(out_ptr: *mut *mut u8, out_len: *mut usize, mut buf: Vec<u8>) {
    let len = buf.len();
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    unsafe {
        *out_ptr = ptr;
        *out_len = len;
    }
}

