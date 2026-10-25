// [EN] Integration tests for C ABI FFI boundary, buffer overflow protection, write operations, and tree callbacks.
// [KO] C ABI FFI 경계, 버퍼 오버플로우 방어, 쓰기 작업 및 콜백 기반 트리 스캐너 통합 테스트.

use ext4_bridge::ffi::*;
use ext4_lwext4::blockdev::FileBlockDevice;
use ext4_lwext4::mkfs::{mkfs, MkfsOptions};
use std::ffi::{c_char, c_void, CStr, CString};
use std::fs::File;
use std::path::PathBuf;

struct TempGuard(PathBuf);
impl Drop for TempGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn create_test_img(label: &str) -> (PathBuf, TempGuard) {
    let temp_dir = std::env::temp_dir();
    let name = format!("ext4_ffi_{}_{}.img", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
    let path = temp_dir.join(name);

    let file = File::create(&path).unwrap();
    file.set_len(16 * 1024 * 1024).unwrap();
    drop(file);

    let dev = FileBlockDevice::open(&path).unwrap();
    let opts = MkfsOptions::ext4().with_label(label);
    mkfs(dev, &opts).unwrap();

    let guard = TempGuard(path.clone());
    (path, guard)
}

#[test]
fn test_ffi_buffer_overflow_defense() {
    let (img_path, _guard) = create_test_img("FFI_OVERFLOW_TEST");
    let c_path = CString::new(img_path.to_str().unwrap()).unwrap();

    let bridge = ext4_open(c_path.as_ptr(), 0, true);
    assert!(!bridge.is_null(), "ext4_open failed");

    // Write 30 bytes
    let test_file = CString::new("/test_overflow.dat").unwrap();
    let payload = b"012345678901234567890123456789"; // 30 bytes
    let ok = ext4_write_h(bridge, test_file.as_ptr(), payload.as_ptr(), payload.len());
    assert!(ok, "ext4_write_h failed");

    // 1. Caller provides tiny buffer (capacity 10)
    let mut tiny_buf = [0u8; 10];
    let mut out_len: usize = 10;
    let read_ok = ext4_read_h(bridge, test_file.as_ptr(), tiny_buf.as_mut_ptr(), &mut out_len);

    // MUST return false, and MUST set out_len to required size 30 WITHOUT memory corruption
    assert!(!read_ok, "ext4_read_h must fail when buffer capacity < file size");
    assert_eq!(out_len, 30, "ext4_read_h must report required capacity (30)");

    // 2. Caller provides adequate buffer (capacity 30)
    let mut adequate_buf = [0u8; 30];
    out_len = 30;
    let read_ok2 = ext4_read_h(bridge, test_file.as_ptr(), adequate_buf.as_mut_ptr(), &mut out_len);
    assert!(read_ok2, "ext4_read_h must succeed with adequate buffer");
    assert_eq!(&adequate_buf, payload);

    ext4_close(bridge);
}

extern "C" fn test_callback(
    path: *const c_char,
    size_bytes: u64,
    is_dir: bool,
    _mtime: u64,
    _inode: u32,
    user_data: *mut c_void,
) -> i32 {
    let counter = unsafe { &mut *(user_data as *mut Vec<(String, u64, bool)>) };
    let path_str = unsafe { CStr::from_ptr(path) }.to_string_lossy().to_string();
    counter.push((path_str, size_bytes, is_dir));
    0 // Continue
}

#[test]
fn test_ffi_tree_scan_callback_and_stats() {
    let (img_path, _guard) = create_test_img("FFI_TREE_TEST");
    let c_path = CString::new(img_path.to_str().unwrap()).unwrap();

    let bridge = ext4_open(c_path.as_ptr(), 0, true);
    assert!(!bridge.is_null());

    let f1 = CString::new("/alpha.txt").unwrap();
    let f2 = CString::new("/beta.txt").unwrap();
    assert!(ext4_write_h(bridge, f1.as_ptr(), b"alpha".as_ptr(), 5));
    assert!(ext4_write_h(bridge, f2.as_ptr(), b"beta_content".as_ptr(), 12));

    // Test ext4_tree_scan callback
    let mut collected: Vec<(String, u64, bool)> = Vec::new();
    let root = CString::new("/").unwrap();
    let count = ext4_tree_scan(
        bridge,
        root.as_ptr(),
        10,
        test_callback,
        &mut collected as *mut _ as *mut c_void,
    );
    assert!(count >= 2, "Scanned count should be >= 2");
    assert!(collected.iter().any(|(p, sz, is_d)| p == "/alpha.txt" && *sz == 5 && !*is_d));
    assert!(collected.iter().any(|(p, sz, is_d)| p == "/beta.txt" && *sz == 12 && !*is_d));

    // Test ext4_get_fs_stats
    let mut total_bytes: u64 = 0;
    let mut free_bytes: u64 = 0;
    let mut block_size: u32 = 0;
    let stat_ok = ext4_get_fs_stats(bridge, &mut total_bytes, &mut free_bytes, &mut block_size);
    assert!(stat_ok);
    assert!(total_bytes >= 15 * 1024 * 1024);
    assert!(free_bytes > 0);
    assert_eq!(block_size, 4096);

    ext4_close(bridge);
}
