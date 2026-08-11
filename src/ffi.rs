use std::ffi::{c_char, c_void, CStr};
use crate::ext4::Ext4Bridge;
use ext4_lwext4::types::OpenFlags;

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
            // Dropped here, which umounts the fs
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
    
    match bridge.fs.open(&path_str, OpenFlags::READ) {
        Ok(mut file) => {
            let mut data = Vec::new();
            if file.read_to_end(&mut data).is_err() {
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
    _bridge_ptr: *mut c_void,
    _path: *const c_char,
    _in_buf: *const u8,
    _in_len: usize,
) -> bool {
    // We can implement write similarly later if needed.
    false
}
