// [EN] Architecture: C-compatible FFI interface exposing thread-safe bridge operations and callback streaming for SniffEverything.
// [KO] 아키텍처: SniffEverything.exe 및 네이티브 C/Zig 클라이언트를 위한 버퍼 오버플로우 방어 FFI 인터페이스 및 콜백 기반 초고속 트리 스캐너 C ABI.
// Why other approaches were rejected: Raw string passing without length checks invited memory corruption; structured pointer checking, caller-capacity enforcement, and early-abort callbacks ensure memory safety.

use crate::ext4::Ext4Bridge;
use std::ffi::{c_char, c_void, CStr, CString};

/// C callback signature for streaming directory tree nodes directly into SniffEverything without intermediate JSON parsing.
/// Return 0 to continue traversal, or non-zero to abort early.
pub type Ext4ScanCallback = extern "C" fn(
    path: *const c_char,
    size_bytes: u64,
    is_dir: bool,
    mtime: u64,
    inode: u32,
    user_data: *mut c_void,
) -> i32;

#[no_mangle]
pub extern "C" fn ext4_open(
    disk_path: *const c_char,
    part_index: u32,
    write_mode: bool,
) -> *mut c_void {
    if disk_path.is_null() {
        return std::ptr::null_mut();
    }
    let path = unsafe { CStr::from_ptr(disk_path) }.to_string_lossy();
    match Ext4Bridge::new(&path, part_index, write_mode) {
        Ok(bridge) => Box::into_raw(Box::new(bridge)) as *mut c_void,
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn ext4_close(bridge_ptr: *mut c_void) {
    if !bridge_ptr.is_null() {
        unsafe {
            let _ = Box::from_raw(bridge_ptr as *mut Ext4Bridge);
        }
    }
}

#[no_mangle]
pub extern "C" fn ext4_read_h(
    bridge_ptr: *mut c_void,
    path: *const c_char,
    out_buf: *mut u8,
    out_len: *mut usize,
) -> bool {
    if bridge_ptr.is_null() || path.is_null() || out_buf.is_null() || out_len.is_null() {
        return false;
    }

    let bridge = unsafe { &mut *(bridge_ptr as *mut Ext4Bridge) };
    let path_str = unsafe { CStr::from_ptr(path) }.to_string_lossy();

    match bridge.cat_file(&path_str) {
        Ok(data) => {
            let caller_cap = unsafe { *out_len };
            if caller_cap < data.len() {
                // Buffer too small: inform caller of required size without overflowing
                unsafe { *out_len = data.len() };
                return false;
            }
            unsafe {
                std::ptr::copy_nonoverlapping(data.as_ptr(), out_buf, data.len());
                *out_len = data.len();
            }
            true
        }
        Err(_) => false,
    }
}

#[no_mangle]
pub extern "C" fn ext4_write_h(
    bridge_ptr: *mut c_void,
    path: *const c_char,
    in_buf: *const u8,
    in_len: usize,
) -> bool {
    if bridge_ptr.is_null() || path.is_null() || (in_buf.is_null() && in_len > 0) {
        return false;
    }

    let bridge = unsafe { &mut *(bridge_ptr as *mut Ext4Bridge) };
    let path_str = unsafe { CStr::from_ptr(path) }.to_string_lossy();
    let slice = if in_len > 0 {
        unsafe { std::slice::from_raw_parts(in_buf, in_len) }
    } else {
        &[]
    };

    bridge.write_file(&path_str, slice).is_ok()
}

/// Recursively scans filesystem starting at `root_path` and streams each entry into `cb`.
/// Returns total scanned count on success, or -1 on error.
#[no_mangle]
pub extern "C" fn ext4_tree_scan(
    bridge_ptr: *mut c_void,
    root_path: *const c_char,
    max_depth: u32,
    cb: Ext4ScanCallback,
    user_data: *mut c_void,
) -> i32 {
    ext4_tree_scan_ex(bridge_ptr, root_path, max_depth, 0, 0, false, cb, user_data)
}

/// Extended C-ABI tree scanner with explicit caller-controlled `timeout_ms` (`0 = unlimited`),
/// `max_entries` (`0 = unlimited`), and `debug_log` (`stderr` trace output).
#[no_mangle]
pub extern "C" fn ext4_tree_scan_ex(
    bridge_ptr: *mut c_void,
    root_path: *const c_char,
    max_depth: u32,
    timeout_ms: u64,
    max_entries: usize,
    debug_log: bool,
    cb: Ext4ScanCallback,
    user_data: *mut c_void,
) -> i32 {
    if bridge_ptr.is_null() || root_path.is_null() {
        return -1;
    }

    let bridge = unsafe { &*(bridge_ptr as *const Ext4Bridge) };
    let root_str = unsafe { CStr::from_ptr(root_path) }.to_string_lossy();
    let depth_opt = if max_depth == u32::MAX {
        None
    } else {
        Some(max_depth)
    };
    let timeout_opt = if timeout_ms == 0 { None } else { Some(timeout_ms) };
    let max_entries_opt = if max_entries == 0 { None } else { Some(max_entries) };

    match bridge.tree_scan_visit(&root_str, depth_opt, timeout_opt, max_entries_opt, debug_log, &mut |entry| {
        if let Ok(c_path) = CString::new(entry.path.as_str()) {
            cb(
                c_path.as_ptr(),
                entry.size_bytes,
                entry.is_dir,
                entry.mtime,
                entry.inode,
                user_data,
            )
        } else {
            0
        }
    }) {
        Ok(count) => count,
        Err(_) => -1,
    }
}

/// Serializes tree scan results into JSONL format within `out_buf`.
/// If buffer is too small, writes required length into `*out_len` and returns false.
#[no_mangle]
pub extern "C" fn ext4_scan_jsonl(
    bridge_ptr: *mut c_void,
    root_path: *const c_char,
    max_depth: u32,
    out_buf: *mut u8,
    out_len: *mut usize,
) -> bool {
    if bridge_ptr.is_null() || root_path.is_null() || out_buf.is_null() || out_len.is_null() {
        return false;
    }

    let bridge = unsafe { &*(bridge_ptr as *const Ext4Bridge) };
    let root_str = unsafe { CStr::from_ptr(root_path) }.to_string_lossy();
    let depth_opt = if max_depth == u32::MAX {
        None
    } else {
        Some(max_depth)
    };

    match bridge.tree_scan_jsonl(&root_str, depth_opt) {
        Ok(jsonl) => {
            let bytes = jsonl.as_bytes();
            let caller_cap = unsafe { *out_len };
            if caller_cap < bytes.len() {
                unsafe { *out_len = bytes.len() };
                return false;
            }
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf, bytes.len());
                *out_len = bytes.len();
            }
            true
        }
        Err(_) => false,
    }
}

/// Retrieves total filesystem capacity, free space, and block size.
#[no_mangle]
pub extern "C" fn ext4_get_fs_stats(
    bridge_ptr: *mut c_void,
    total_bytes: *mut u64,
    free_bytes: *mut u64,
    block_size: *mut u32,
) -> bool {
    if bridge_ptr.is_null() || total_bytes.is_null() || free_bytes.is_null() || block_size.is_null() {
        return false;
    }

    let bridge = unsafe { &*(bridge_ptr as *const Ext4Bridge) };
    match bridge.stat_fs() {
        Ok(stat) => {
            unsafe {
                *total_bytes = stat.total_bytes;
                *free_bytes = stat.free_bytes;
                *block_size = stat.block_size;
            }
            true
        }
        Err(_) => false,
    }
}
