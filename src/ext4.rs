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

/// Reads and validates the ext4 superblock at part_offset + 1024 bytes.
/// Returns (block_size, total_blocks_in_sb).
fn read_ext4_superblock_info(disk: &mut DiskBackend, part_offset: u64) -> Result<(u32, u64), Ext4Error> {
    let sb_offset = part_offset + 1024;
    let mut sb = [0u8; 1024];
    disk.read_at(sb_offset, &mut sb)?;

    let magic = LittleEndian::read_u16(&sb[56..58]);
    if magic != 0xEF53 {
        return Err(Ext4Error::InvalidSuperblock { magic });
    }

    let log_block = LittleEndian::read_u32(&sb[24..28]);
    let block_size = 1024u32.checked_shl(log_block).unwrap_or(4096);

    let blocks_count_lo = LittleEndian::read_u32(&sb[4..8]) as u64;
    let blocks_count_hi = if sb.len() >= 0x154 {
        LittleEndian::read_u32(&sb[0x150..0x154]) as u64
    } else {
        0
    };
    let total_blocks = (blocks_count_hi << 32) | blocks_count_lo;

    Ok((block_size, total_blocks))
}

#[derive(uniffi::Object)]
pub struct Ext4Bridge {
    pub fs: Ext4Fs,
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

        // Validate ext4 superblock and get true block size
        let (ext4_block_size, sb_total_blocks) = read_ext4_superblock_info(&mut disk, part_offset)?;

        // Calculate accurate block count (avoiding u64::MAX overflow)
        let block_count = if part_length > 0 {
            part_length / (sector_size as u64)
        } else if sb_total_blocks > 0 && ext4_block_size >= sector_size {
            sb_total_blocks * ((ext4_block_size / sector_size) as u64)
        } else {
            1048576 // Safe 512MB fallback if geometry is undetermined
        };

        let bd = LwExt4BlockDevice::new(
            Arc::new(Mutex::new(disk)),
            part_offset,
            block_count,
            sector_size,
        );

        let fs = Ext4Fs::mount(bd, !write_mode)
            .map_err(|e| Ext4Error::Io { msg: format!("lwext4 mount failed: {:?}", e) })?;

        Ok(Self {
            fs,
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
        let max_d = max_depth.unwrap_or(u32::MAX);
        let mut results = Vec::new();
        // Queue stores (directory_path, current_depth)
        let mut queue = std::collections::VecDeque::new();
        queue.push_back((root_path.to_string(), 0u32));

        while let Some((dir_path, depth)) = queue.pop_front() {
            let mut dir = match self.fs.open_dir(&dir_path) {
                Ok(d) => d,
                Err(_) => continue,
            };

            while let Some(Ok(entry)) = dir.next_entry() {
                let name = entry.name();
                if name == "." || name == ".." {
                    continue;
                }

                let full_path = if dir_path == "/" {
                    format!("/{}", name)
                } else {
                    format!("{}/{}", dir_path.trim_end_matches('/'), name)
                };

                let is_dir = entry.file_type().is_dir();
                let inode = entry.inode() as u32;

                let (size_bytes, mtime, mode) = match self.fs.metadata(&full_path) {
                    Ok(meta) => (meta.size, meta.mtime, meta.mode),
                    Err(_) => (0, 0, 0),
                };

                results.push(Ext4NodeEntry {
                    path: full_path.clone(),
                    size_bytes,
                    is_dir,
                    mtime,
                    inode,
                    mode,
                });

                if is_dir && depth + 1 <= max_d {
                    queue.push_back((full_path, depth + 1));
                }
            }
        }

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
