// [EN] Architecture: Core ext4 filesystem bridge with superblock validation, GPT/MBR partition detection, and recursive tree scanner.
// [KO] 아키텍처: 슈퍼블록 매직 검증, GPT/MBR 파티션 오프셋 및 정확한 블록 크기 계산, 초고속 재귀 트리 스캐너를 제공하는 ext4 핵심 브릿지.
// Why other approaches were rejected: Discarding superblock block size and setting block_count to u64::MAX caused arithmetic overflow; true geometry and BFS tree scanner guarantee millisecond X-ray inspection for SniffEverything.

use byteorder::{ByteOrder, LittleEndian};
use ext4_lwext4::fs::Ext4Fs;
use ext4_lwext4::types::OpenFlags;
use crate::disk_io::{DiskBackend, LwExt4BlockDevice};
use crate::error::Ext4Error;
use std::sync::{Arc, Mutex};

/// Directory or file entry returned by tree scanner
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct Ext4NodeEntry {
    pub path: String,
    pub size_bytes: u64,
    pub is_dir: bool,
    pub mtime: u64,
    pub inode: u32,
    pub mode: u32,
}

/// Filesystem statistics
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct Ext4FsStat {
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub used_bytes: u64,
    pub block_size: u32,
    pub total_inodes: u32,
    pub free_inodes: u32,
    pub volume_name: String,
}

/// Cached superblock and Block Group Descriptor Table (BGDT) geometry for O(1) direct inode metadata lookup.
#[derive(Debug, Clone)]
pub struct Ext4InodeGeometry {
    pub block_size: u32,
    pub total_blocks: u64,
    pub inodes_per_group: u32,
    pub inode_size: u16,
    pub inode_table_blocks: Vec<u64>,
}

/// Reads and validates the ext4 superblock at part_offset + 1024 bytes, and pre-loads the Block Group Descriptor Table.
fn read_ext4_superblock_info(disk: &mut DiskBackend, part_offset: u64) -> Result<Ext4InodeGeometry, Ext4Error> {
    let sb_offset = part_offset + 1024;
    let mut sb = [0u8; 1024];
    disk.read_at(sb_offset, &mut sb)?;

    let magic = LittleEndian::read_u16(&sb[56..58]);
    if magic != 0xEF53 {
        return Err(Ext4Error::InvalidSuperblock { magic });
    }

    let log_block = LittleEndian::read_u32(&sb[24..28]);
    let block_size = 1024u32.checked_shl(log_block).unwrap_or(4096);

    let inodes_count = LittleEndian::read_u32(&sb[0..4]);
    let blocks_count_lo = LittleEndian::read_u32(&sb[4..8]) as u64;
    let blocks_count_hi = if sb.len() >= 0x154 {
        LittleEndian::read_u32(&sb[0x150..0x154]) as u64
    } else {
        0
    };
    let total_blocks = (blocks_count_hi << 32) | blocks_count_lo;
    let blocks_per_group = LittleEndian::read_u32(&sb[32..36]).max(1);
    let inodes_per_group = LittleEndian::read_u32(&sb[40..44]).max(1);
    let rev_level = LittleEndian::read_u32(&sb[76..80]);
    let inode_size = if rev_level == 0 {
        128u16
    } else {
        LittleEndian::read_u16(&sb[88..90]).max(128)
    };
    let feature_incompat = LittleEndian::read_u32(&sb[96..100]);
    let desc_size: usize = if (feature_incompat & 0x0080) != 0 {
        (LittleEndian::read_u16(&sb[0xFE..0x100]) as usize).max(32)
    } else {
        32
    };

    let bg_count_by_blocks = total_blocks.div_ceil(blocks_per_group as u64) as usize;
    let bg_count_by_inodes = (inodes_count as usize).div_ceil(inodes_per_group as usize);
    let bg_count = bg_count_by_blocks.max(bg_count_by_inodes).clamp(1, 262_144);

    let bgdt_block = if block_size == 1024 { 2u64 } else { 1u64 };
    let bgdt_offset = part_offset + bgdt_block * (block_size as u64);
    let bgdt_bytes_len = bg_count * desc_size;
    let mut bgdt_buf = vec![0u8; bgdt_bytes_len];
    let mut inode_table_blocks = Vec::with_capacity(bg_count);
    if disk.read_at(bgdt_offset, &mut bgdt_buf).is_ok() {
        for i in 0..bg_count {
            let off = i * desc_size;
            let lo = LittleEndian::read_u32(&bgdt_buf[off + 8..off + 12]) as u64;
            let hi = if desc_size >= 64 && (feature_incompat & 0x0080) != 0 {
                LittleEndian::read_u32(&bgdt_buf[off + 40..off + 44]) as u64
            } else {
                0
            };
            inode_table_blocks.push((hi << 32) | lo);
        }
    }

    Ok(Ext4InodeGeometry {
        block_size,
        total_blocks,
        inodes_per_group,
        inode_size,
        inode_table_blocks,
    })
}

#[derive(uniffi::Object)]
pub struct Ext4Bridge {
    pub fs: Ext4Fs,
    pub disk: Arc<Mutex<DiskBackend>>,
    pub inode_geom: Ext4InodeGeometry,
    pub write_mode: bool,
    pub disk_path: String,
    pub part_offset: u64,
    pub sector_size: u32,
    pub ext4_block_size: u32,
}

#[uniffi::export]
impl Ext4Bridge {
    #[uniffi::constructor]
    pub fn open_fs(disk_path: &str, part_index: u32, write_mode: bool) -> Result<Self, Ext4Error> {
        let mut disk = DiskBackend::open(disk_path, write_mode)?;
        let sector_size = disk.sector_size();

        let (part_offset, part_length) = Self::find_partition_offset_and_size(&mut disk, part_index, sector_size as u64)?;

        // Validate ext4 superblock, get true block size, and pre-load BGDT for O(1) inode metadata lookup
        let inode_geom = read_ext4_superblock_info(&mut disk, part_offset)?;
        let ext4_block_size = inode_geom.block_size;
        let sb_total_blocks = inode_geom.total_blocks;

        // Calculate accurate block count (avoiding u64::MAX overflow)
        let block_count = if part_length > 0 {
            part_length / (sector_size as u64)
        } else if sb_total_blocks > 0 && ext4_block_size >= sector_size {
            sb_total_blocks * ((ext4_block_size / sector_size) as u64)
        } else {
            1048576 // Safe 512MB fallback if geometry is undetermined
        };

        let shared_disk = Arc::new(Mutex::new(disk));
        let bd = LwExt4BlockDevice::new(
            Arc::clone(&shared_disk),
            part_offset,
            block_count,
            sector_size,
        );

        let fs = Ext4Fs::mount(bd, !write_mode)
            .map_err(|e| Ext4Error::Io { msg: format!("lwext4 mount failed: {:?}", e) })?;

        Ok(Self {
            fs,
            disk: shared_disk,
            inode_geom,
            write_mode,
            disk_path: disk_path.to_string(),
            part_offset,
            sector_size,
            ext4_block_size,
        })
    }

    pub fn ls_dir(&self, path: &str) -> Result<Vec<String>, Ext4Error> {
        let mut dir = self.fs.open_dir(path)
            .map_err(|e| Ext4Error::Io { msg: format!("Open dir error: {:?}", e) })?;
        let mut entries = Vec::new();
        while let Some(Ok(entry)) = dir.next_entry() {
            entries.push(format!("{:6} | {:?} | {}", entry.inode(), entry.file_type(), entry.name()));
        }
        Ok(entries)
    }

    pub fn cat_file(&self, path: &str) -> Result<Vec<u8>, Ext4Error> {
        let mut file = self.fs.open(path, OpenFlags::READ)
            .map_err(|e| Ext4Error::Io { msg: format!("Open file error: {:?}", e) })?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)
            .map_err(|e| Ext4Error::Io { msg: format!("Read error: {:?}", e) })?;
        Ok(data)
    }

    pub fn write_file(&self, path: &str, data: &[u8]) -> Result<(), Ext4Error> {
        let mut file = self.fs.open(path, OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE)
            .map_err(|e| Ext4Error::Io { msg: format!("Open file error: {:?}", e) })?;
        file.write_all(data)
            .map_err(|e| Ext4Error::Io { msg: format!("Write error: {:?}", e) })?;
        drop(file);
        let _ = self.fs.sync();
        Ok(())
    }

    pub fn copy_file(&self, src: &str, dest: &str) -> Result<(), Ext4Error> {
        let data = self.cat_file(src)?;
        self.write_file(dest, &data)?;
        Ok(())
    }

    pub fn make_dir(&self, path: &str) -> Result<(), Ext4Error> {
        self.fs.mkdir(path, 0)
            .map_err(|e| Ext4Error::Io { msg: format!("Mkdir error: {:?}", e) })
    }

    pub fn remove_path(&self, path: &str) -> Result<(), Ext4Error> {
        if self.fs.is_dir(path) {
            self.fs.rmdir(path).map_err(|e| Ext4Error::Io { msg: format!("Rmdir error: {:?}", e) })
        } else {
            self.fs.remove(path).map_err(|e| Ext4Error::Io { msg: format!("Remove error: {:?}", e) })
        }
    }

    pub fn rename_path(&self, src: &str, dest: &str) -> Result<(), Ext4Error> {
        self.fs.rename(src, dest)
            .map_err(|e| Ext4Error::Io { msg: format!("Rename error: {:?}", e) })
    }

    pub fn symlink_path(&self, target: &str, linkpath: &str) -> Result<(), Ext4Error> {
        self.fs.symlink(target, linkpath)
            .map_err(|e| Ext4Error::Io { msg: format!("Symlink error: {:?}", e) })
    }

    pub fn chmod_path(&self, path: &str, mode: u32) -> Result<(), Ext4Error> {
        self.fs.set_permissions(path, mode)
            .map_err(|e| Ext4Error::Io { msg: format!("Chmod error: {:?}", e) })
    }

    /// Query comprehensive filesystem statistics
    pub fn stat_fs(&self) -> Result<Ext4FsStat, Ext4Error> {
        let stats = self.fs.stat()
            .map_err(|e| Ext4Error::Io { msg: format!("StatFs error: {:?}", e) })?;

        Ok(Ext4FsStat {
            total_bytes: stats.total_size(),
            free_bytes: stats.free_size().min(stats.total_size()),
            used_bytes: stats.total_size().saturating_sub(stats.free_size()),
            block_size: stats.block_size,
            total_inodes: stats.total_inodes as u32,
            free_inodes: stats.free_inodes as u32,
            volume_name: stats.volume_name,
        })
    }

    /// High-speed BFS recursive directory scanner for SniffEverything.exe
    pub fn tree_scan(&self, root_path: &str, max_depth: Option<u32>) -> Result<Vec<Ext4NodeEntry>, Ext4Error> {
        let mut results = Vec::new();
        self.tree_scan_visit(root_path, max_depth, None, None, false, &mut |entry| {
            results.push(entry);
            0
        })?;
        Ok(results)
    }

    /// Tree scanner emitting one JSON object per line (JSONL)
    pub fn tree_scan_jsonl(&self, root_path: &str, max_depth: Option<u32>) -> Result<String, Ext4Error> {
        let entries = self.tree_scan(root_path, max_depth)?;
        let mut out = String::with_capacity(entries.len() * 128);
        for entry in &entries {
            if let Ok(line) = serde_json::to_string(entry) {
                out.push_str(&line);
                out.push('\n');
            }
        }
        Ok(out)
    }
}

impl Ext4Bridge {
    pub fn new(disk_path: &str, part_index: u32, write_mode: bool) -> Result<Self, Ext4Error> {
        Self::open_fs(disk_path, part_index, write_mode)
    }

    /// O(1) direct inode metadata lookup by inode number from the Block Group Descriptor Table (BGDT),
    /// bypassing lwext4 path traversal (`self.fs.metadata(path)`) and `ext4_dir_dx_find_entry` completely!
    pub fn read_inode_meta_fast(&self, inode: u32, block_cache: &mut std::collections::HashMap<u64, Vec<u8>>) -> Option<(u64, u64, u32)> {
        if inode == 0 {
            return None;
        }
        let inodes_per_group = self.inode_geom.inodes_per_group as u64;
        if inodes_per_group == 0 {
            return None;
        }
        let idx0 = (inode - 1) as u64;
        let bg_idx = (idx0 / inodes_per_group) as usize;
        let idx_in_group = idx0 % inodes_per_group;
        let itable_block = *self.inode_geom.inode_table_blocks.get(bg_idx)?;
        if itable_block == 0 {
            return None;
        }
        let block_size = self.inode_geom.block_size as u64;
        let inode_size = self.inode_geom.inode_size as u64;
        let byte_off_in_table = idx_in_group * inode_size;
        let target_fs_block = itable_block + (byte_off_in_table / block_size);
        let off_in_block = (byte_off_in_table % block_size) as usize;

        if !block_cache.contains_key(&target_fs_block) {
            if block_cache.len() >= 1024 {
                block_cache.clear();
            }
            let disk_offset = self.part_offset + target_fs_block * block_size;
            let mut buf = vec![0u8; block_size as usize];
            let mut disk = self.disk.lock().ok()?;
            disk.read_at(disk_offset, &mut buf).ok()?;
            block_cache.insert(target_fs_block, buf);
        }

        let blk = block_cache.get(&target_fs_block)?;
        if off_in_block + 128 > blk.len() {
            return None;
        }
        let ino_buf = &blk[off_in_block..off_in_block + 128];
        let raw_mode = LittleEndian::read_u16(&ino_buf[0x00..0x02]) as u32;
        let size_lo = LittleEndian::read_u32(&ino_buf[0x04..0x08]) as u64;
        let mtime = LittleEndian::read_u32(&ino_buf[0x10..0x14]) as u64;
        let size_hi = LittleEndian::read_u32(&ino_buf[0x6C..0x70]) as u64;
        let size_bytes = (size_hi << 32) | size_lo;
        let mode = raw_mode & 0o7777;
        Some((size_bytes, mtime, mode))
    }

    /// True zero-wait streaming BFS visitor with explicit caller-controlled `timeout_ms`, `max_entries`, and `debug_log`:
    /// 1. Closes parent `Ext4Dir` handle BEFORE any subsequent operations so single-instance lwext4 never corrupts active dir iteration state!
    /// 2. Reads `(size_bytes, mtime, mode)` in O(1) directly from the BGDT inode table by `inode` number (`read_inode_meta_fast`), bypassing `self.fs.metadata(path)`!
    /// 3. Invokes `visitor` immediately per entry and aborts BFS the instant `visitor` returns non-zero!
    pub fn tree_scan_visit<F>(
        &self,
        root_path: &str,
        max_depth: Option<u32>,
        timeout_ms: Option<u64>,
        max_entries: Option<usize>,
        debug_log: bool,
        visitor: &mut F,
    ) -> Result<i32, Ext4Error>
    where
        F: FnMut(Ext4NodeEntry) -> i32,
    {
        let max_d = max_depth.unwrap_or(u32::MAX);
        let mut count: i32 = 0;
        let t_start = std::time::Instant::now();
        let deadline = timeout_ms.filter(|&ms| ms > 0).map(|ms| t_start + std::time::Duration::from_millis(ms));
        let mut visited_dir_inodes = std::collections::HashSet::new();
        let mut inode_block_cache = std::collections::HashMap::with_capacity(256);
        let mut queue = std::collections::VecDeque::new();
        queue.push_back((root_path.to_string(), 0u32));

        while let Some((dir_path, depth)) = queue.pop_front() {
            if let Some(dl) = deadline {
                if std::time::Instant::now() >= dl {
                    if debug_log {
                        eprintln!("[DEBUG][ext4] timeout reached before dir='{}' (scanned={} entries, elapsed={:.1}ms)", dir_path, count, t_start.elapsed().as_secs_f64() * 1000.0);
                    }
                    break;
                }
            }
            if debug_log {
                eprintln!("[DEBUG][ext4] open_dir depth={} path='{}' (scanned={})", depth, dir_path, count);
            }
            // Collect raw directory entries first, then drop `dir` BEFORE any `self.fs.metadata()` calls!
            let mut raw_entries = Vec::new();
            {
                let mut dir = match self.fs.open_dir(&dir_path) {
                    Ok(d) => d,
                    Err(e) => {
                        if debug_log {
                            eprintln!("[DEBUG][ext4] open_dir failed path='{}' err={:?}", dir_path, e);
                        }
                        continue;
                    }
                };
                let mut prev_inode_name: Option<(u32, String)> = None;
                while let Some(Ok(entry)) = dir.next_entry() {
                    let name = entry.name().to_string();
                    let ino = entry.inode() as u32;
                    // Guard against lwext4 cursor non-advancement on corrupted/unsupported directory blocks (exact duplicate consecutive cursor position)
                    if let Some((p_ino, ref p_name)) = prev_inode_name {
                        if p_ino == ino && p_name == &name {
                            if debug_log {
                                eprintln!("[DEBUG][ext4] dir cursor stuck at '{}' ino={} in '{}', breaking dir loop", name, ino, dir_path);
                            }
                            break;
                        }
                    }
                    prev_inode_name = Some((ino, name.clone()));
                    if name == "." || name == ".." {
                        continue;
                    }
                    raw_entries.push((name, entry.file_type(), ino));
                }
            }
            if debug_log {
                eprintln!("[DEBUG][ext4] closed_dir path='{}' raw_entries={}", dir_path, raw_entries.len());
            }

            for (name, ftype, inode) in raw_entries {
                if let Some(dl) = deadline {
                    if (count & 63) == 0 && std::time::Instant::now() >= dl {
                        if debug_log {
                            eprintln!("[DEBUG][ext4] timeout reached inside dir='{}' at '{}' (scanned={})", dir_path, name, count);
                        }
                        return Ok(count);
                    }
                }
                if let Some(max_e) = max_entries {
                    if max_e > 0 && (count as usize) >= max_e {
                        if debug_log {
                            eprintln!("[DEBUG][ext4] max_entries={} reached at '{}' (scanned={})", max_e, name, count);
                        }
                        return Ok(count);
                    }
                }
                let full_path = if dir_path == "/" {
                    format!("/{}", name)
                } else {
                    format!("{}/{}", dir_path.trim_end_matches('/'), name)
                };

                let is_dir = ftype.is_dir();
                let is_reg = ftype.is_file();

                // Skip pseudo-filesystems and special device/socket/symlink nodes that have 0 data blocks
                if is_dir && (full_path == "/proc" || full_path == "/sys" || full_path == "/dev" || full_path == "/run") {
                    continue;
                }
                if !is_dir && !is_reg {
                    continue;
                }
                if is_dir && inode > 0 && !visited_dir_inodes.insert(inode) {
                    continue;
                }

                // Only query inode metadata for RegularFile (directories have 0 own bytes; SniffEverything rolls up folder sizes from children!)
                let (size_bytes, mtime, mode) = if is_reg {
                    if !self.write_mode {
                        if let Some(meta_tuple) = self.read_inode_meta_fast(inode, &mut inode_block_cache) {
                            meta_tuple
                        } else {
                            match self.fs.metadata(&full_path) {
                                Ok(meta) => (meta.size, meta.mtime, meta.mode),
                                Err(_) => (0, 0, 0),
                            }
                        }
                    } else {
                        if debug_log {
                            eprintln!("[DEBUG][ext4] write_mode metadata query path='{}' ino={}", full_path, inode);
                        }
                        match self.fs.metadata(&full_path) {
                            Ok(meta) => (meta.size, meta.mtime, meta.mode),
                            Err(e) => {
                                if debug_log {
                                    eprintln!("[DEBUG][ext4] metadata failed path='{}' err={:?}", full_path, e);
                                }
                                self.read_inode_meta_fast(inode, &mut inode_block_cache).unwrap_or((0, 0, 0))
                            }
                        }
                    }
                } else {
                    (0, 0, 0o755)
                };

                let ret = visitor(Ext4NodeEntry {
                    path: full_path.clone(),
                    size_bytes,
                    is_dir,
                    mtime,
                    inode,
                    mode,
                });
                count += 1;
                if ret != 0 {
                    if debug_log {
                        eprintln!("[DEBUG][ext4] visitor requested early abort (ret={}) at count={}", ret, count);
                    }
                    return Ok(count);
                }

                // Only enqueue subdirectories when `depth < max_d` (`max_depth = 0` means root directory only!)
                if is_dir && depth < max_d {
                    queue.push_back((full_path, depth + 1));
                }
            }
        }

        Ok(count)
    }

    /// Finds (partition_offset_bytes, partition_length_bytes).
    /// Transparently detects unpartitioned direct ext4 (WSL2 VHDX default) and MBR/GPT tables.
    fn find_partition_offset_and_size(
        disk: &mut DiskBackend,
        target_index: u32,
        sector_size: u64,
    ) -> Result<(u64, u64), Ext4Error> {
        let total_disk_bytes = disk.total_bytes();

        // 1. Direct ext4 check (WSL2 ext4.vhdx or unpartitioned image where superblock is at offset 1024)
        if target_index <= 1 {
            let mut sb_probe = [0u8; 64];
            if disk.read_at(1024, &mut sb_probe).is_ok() {
                let magic = LittleEndian::read_u16(&sb_probe[56..58]);
                if magic == 0xEF53 {
                    return Ok((0, total_disk_bytes));
                }
            }
        }

        if target_index == 0 {
            return Ok((0, total_disk_bytes));
        }

        // 2. Read MBR (Sector 0)
        let mut mbr = vec![0u8; sector_size as usize];
        disk.read_at(0, &mut mbr)?;

        let mut is_gpt = false;
        for i in 0..4 {
            let offset = 0x1BE + (i * 16);
            if offset + 4 < mbr.len() && mbr[offset + 4] == 0xEE {
                is_gpt = true;
                break;
            }
        }

        if is_gpt {
            // Read GPT Header (Sector 1)
            let mut gpt_header = vec![0u8; sector_size as usize];
            disk.read_at(sector_size, &mut gpt_header)?;
            if gpt_header.len() < 92 || &gpt_header[0..8] != b"EFI PART" {
                return Err(Ext4Error::Io { msg: "Invalid GPT signature".into() });
            }

            let entry_lba = LittleEndian::read_u64(&gpt_header[0x48..0x50]);
            let num_entries = LittleEndian::read_u32(&gpt_header[0x50..0x54]);
            let entry_size = LittleEndian::read_u32(&gpt_header[0x54..0x58]);

            let mut valid_count = 0;
            let table_bytes_len = (num_entries * entry_size) as usize;
            let mut table_bytes = vec![0u8; table_bytes_len];
            disk.read_at(entry_lba * sector_size, &mut table_bytes)?;

            for i in 0..num_entries {
                let entry_off = (i * entry_size) as usize;
                if entry_off + (entry_size as usize) > table_bytes.len() {
                    break;
                }
                let entry = &table_bytes[entry_off..entry_off + entry_size as usize];
                let is_zero = entry[0..16].iter().all(|&b| b == 0);

                if !is_zero {
                    valid_count += 1;
                    if valid_count == target_index {
                        let first_lba = LittleEndian::read_u64(&entry[0x20..0x28]);
                        let last_lba = LittleEndian::read_u64(&entry[0x28..0x30]);
                        let part_offset = first_lba * sector_size;
                        let part_length = (last_lba.saturating_sub(first_lba) + 1) * sector_size;
                        return Ok((part_offset, part_length));
                    }
                }
            }
            Err(Ext4Error::PartitionNotFound {
                index: target_index,
                msg: format!("GPT partition {} not found", target_index),
            })
        } else {
            let mut valid_count = 0;
            for i in 0..4 {
                let offset = 0x1BE + (i * 16);
                let part_type = mbr[offset + 4];
                if part_type != 0 {
                    valid_count += 1;
                    if valid_count == target_index {
                        let lba_start = LittleEndian::read_u32(&mbr[offset + 8..offset + 12]) as u64;
                        let lba_count = LittleEndian::read_u32(&mbr[offset + 12..offset + 16]) as u64;
                        let part_offset = lba_start * sector_size;
                        let part_length = lba_count * sector_size;
                        return Ok((part_offset, part_length));
                    }
                }
            }
            Err(Ext4Error::PartitionNotFound {
                index: target_index,
                msg: format!("MBR partition {} not found", target_index),
            })
        }
    }
}
