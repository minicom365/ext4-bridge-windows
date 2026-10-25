// [EN] Integration tests for raw ext4 filesystem formatting, file operations, permissions, and tree scanning.
// [KO] raw ext4 파일 시스템 생성(mkfs), CRUD 파일 조작, 권한, BFS 트리 스캐너 통합 테스트.

use ext4_bridge::ext4::Ext4Bridge;
use ext4_lwext4::blockdev::FileBlockDevice;
use ext4_lwext4::mkfs::{mkfs, MkfsOptions};
use std::fs::File;
use std::path::PathBuf;

struct TempImageGuard {
    path: PathBuf,
}

impl Drop for TempImageGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn create_formatted_ext4_image(size_bytes: u64, volume_label: &str) -> (PathBuf, TempImageGuard) {
    let temp_dir = std::env::temp_dir();
    let file_name = format!("ext4_test_{}_{}.img", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
    let path = temp_dir.join(file_name);

    let file = File::create(&path).expect("Failed to create temporary image file");
    file.set_len(size_bytes).expect("Failed to set image size");
    drop(file);

    let dev = FileBlockDevice::open(&path).expect("Failed to open file block device for mkfs");
    let opts = MkfsOptions::ext4().with_label(volume_label);
    mkfs(dev, &opts).expect("Failed to mkfs ext4 filesystem");

    let guard = TempImageGuard { path: path.clone() };
    (path, guard)
}

#[test]
fn test_ext4_mount_and_stat_fs() {
    let (img_path, _guard) = create_formatted_ext4_image(16 * 1024 * 1024, "UNIT_TEST_VOL");
    let bridge = Ext4Bridge::open_fs(img_path.to_str().unwrap(), 0, false).expect("open_fs failed");

    let stat = bridge.stat_fs().expect("stat_fs failed");
    eprintln!("STAT: {:?}", stat);
    assert_eq!(stat.volume_name, "UNIT_TEST_VOL");
    assert!(stat.total_bytes >= 15 * 1024 * 1024, "Total bytes should be ~16MB");
    assert!(stat.free_bytes > 0, "Free space should be > 0");
    assert!(stat.free_bytes <= stat.total_bytes);
    assert_eq!(stat.block_size, 4096, "ext4 block size should default to 4096");
}

#[test]
fn test_ext4_crud_operations() {
    let (img_path, _guard) = create_formatted_ext4_image(16 * 1024 * 1024, "CRUD_VOL");
    let bridge = Ext4Bridge::open_fs(img_path.to_str().unwrap(), 0, true).expect("open_fs write mode failed");

    // 1. Mkdir
    bridge.make_dir("/docs").expect("mkdir failed");
    bridge.make_dir("/docs/reports").expect("mkdir sub failed");

    // 2. Write file
    let payload = b"Antigravity Ext4 Engine Active";
    bridge.write_file("/docs/reports/status.txt", payload).expect("write_file failed");

    // 3. Cat file
    let read_back = bridge.cat_file("/docs/reports/status.txt").expect("cat_file failed");
    assert_eq!(read_back, payload);

    // 4. Copy file
    bridge.copy_file("/docs/reports/status.txt", "/docs/reports/status_backup.txt").expect("copy_file failed");
    let backup_data = bridge.cat_file("/docs/reports/status_backup.txt").expect("cat backup failed");
    assert_eq!(backup_data, payload);

    // 5. Rename
    bridge.rename_path("/docs/reports/status_backup.txt", "/docs/reports/status_renamed.txt").expect("rename failed");
    let renamed_data = bridge.cat_file("/docs/reports/status_renamed.txt").expect("cat renamed failed");
    assert_eq!(renamed_data, payload);

    // 6. Symlink
    bridge.symlink_path("/docs/reports/status.txt", "/docs/symlink_status").expect("symlink failed");

    // 7. Remove
    bridge.remove_path("/docs/reports/status_renamed.txt").expect("remove file failed");
}

#[test]
fn test_ext4_tree_scanner_and_jsonl() {
    let (img_path, _guard) = create_formatted_ext4_image(16 * 1024 * 1024, "SCAN_VOL");
    let bridge = Ext4Bridge::open_fs(img_path.to_str().unwrap(), 0, true).expect("open_fs failed");

    bridge.make_dir("/var").unwrap();
    bridge.make_dir("/var/log").unwrap();
    bridge.write_file("/var/log/syslog", b"System boot ok\n").unwrap();
    bridge.write_file("/var/log/auth.log", b"Login success\n").unwrap();
    bridge.make_dir("/etc").unwrap();
    bridge.write_file("/etc/hostname", b"antigravity-node\n").unwrap();

    // Run BFS tree scan
    let entries = bridge.tree_scan("/", None).expect("tree_scan failed");
    assert!(entries.len() >= 5, "Should find at least 5 created entries");

    let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
    assert!(paths.contains(&"/var"));
    assert!(paths.contains(&"/var/log"));
    assert!(paths.contains(&"/var/log/syslog"));
    assert!(paths.contains(&"/var/log/auth.log"));
    assert!(paths.contains(&"/etc/hostname"));

    // Verify sizes
    let syslog_entry = entries.iter().find(|e| e.path == "/var/log/syslog").unwrap();
    assert_eq!(syslog_entry.size_bytes, 15);
    assert!(!syslog_entry.is_dir);

    // Verify JSONL
    let jsonl = bridge.tree_scan_jsonl("/", None).expect("tree_scan_jsonl failed");
    assert!(jsonl.contains("\"path\":\"/var/log/syslog\""));
    assert!(jsonl.contains("\"size_bytes\":15"));
}
