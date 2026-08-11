use std::sync::{Arc, Mutex};
use byteorder::{ByteOrder, LittleEndian};
use ext4_lwext4::fs::Ext4Fs;
use ext4_lwext4::types::OpenFlags;
use crate::disk_io::{AlignedWin32RawDisk, LwExt4BlockDevice};
use crate::error::Ext4Error;

/// ext4 슈퍼블록(오프셋 1024)에서 블록 크기를 읽는다.
/// s_log_block_size(오프셋 +24, u32 LE): block_size = 1024 << s_log_block_size
fn read_ext4_block_size(disk: &mut AlignedWin32RawDisk, part_offset: u64) -> u32 {
    // 슈퍼블록은 파티션 시작 + 1024 bytes
    let sb_offset = part_offset + 1024;
    match disk.read_at(sb_offset, 264) {
        Ok(sb) if sb.len() >= 60 => {
            let magic = LittleEndian::read_u16(&sb[56..58]);
            if magic == 0xEF53 {
                let log_block = LittleEndian::read_u32(&sb[24..28]);
                1024u32 << log_block
            } else {
                4096 // fallback
            }
        }
        _ => 4096, // fallback
    }
}


#[derive(uniffi::Object)]
pub struct Ext4Bridge {
    pub fs: Ext4Fs,
}

#[uniffi::export]
impl Ext4Bridge {
    #[uniffi::constructor]
    pub fn open_fs(disk_path: &str, part_index: u32, write_mode: bool) -> Result<Self, Ext4Error> {
        let mut disk = AlignedWin32RawDisk::open(disk_path, write_mode)?;
        let sector_size = disk.get_sector_size() as u64;

        let part_offset = if part_index == 0 {
            0
        } else {
            Self::find_partition_offset(&mut disk, part_index, sector_size)?
        };

        // ext4 블록 크기(4096)를 슈퍼블록에서 읽는다.
        // 물리 섹터 크기(512)와 혼동하면 block_id 오프셋이 8배 어긋남.
        let _ext4_block_size = read_ext4_block_size(&mut disk, part_offset);

        let bd = LwExt4BlockDevice {
            disk: Arc::new(Mutex::new(disk)),
            part_offset,
            block_count: u64::MAX / sector_size,
            sector_size: sector_size as u32, // 물리 섹터 크기 512
        };
        
        let fs = Ext4Fs::mount(bd, !write_mode)
            .map_err(|e| Ext4Error::Io { msg: format!("lwext4 mount failed: {:?}", e) })?;

        Ok(Self { fs })
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
        self.fs.mkdir(path, 0o777)
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
}

impl Ext4Bridge {
    pub fn new(disk_path: &str, part_index: u32, write_mode: bool) -> Result<Self, Ext4Error> {
        Self::open_fs(disk_path, part_index, write_mode)
    }

    fn find_partition_offset(disk: &mut AlignedWin32RawDisk, target_index: u32, sector_size: u64) -> Result<u64, Ext4Error> {
        // Read MBR (Sector 0)
        disk.seek(0);
        let mbr = disk.read(sector_size as usize)?;
        
        let mut is_gpt = false;
        
        for i in 0..4 {
            let offset = 0x1BE + (i * 16);
            let part_type = mbr[offset + 4];
            if part_type == 0xEE {
                is_gpt = true;
                break;
            }
        }

        if is_gpt {
            // Read GPT Header (Sector 1)
            disk.seek(sector_size);
            let gpt_header = disk.read(sector_size as usize)?;
            let magic = &gpt_header[0..8];
            if magic != b"EFI PART" {
                return Err(Ext4Error::Io { msg: "Invalid GPT Magic".into() });
            }

            let entry_lba = LittleEndian::read_u64(&gpt_header[0x48..0x50]);
            let num_entries = LittleEndian::read_u32(&gpt_header[0x50..0x54]);
            let entry_size = LittleEndian::read_u32(&gpt_header[0x54..0x58]);

            let mut valid_count = 0;
            disk.seek(entry_lba * sector_size);
            let table_bytes = disk.read((num_entries * entry_size) as usize)?;

            for i in 0..num_entries {
                let entry_off = (i * entry_size) as usize;
                let entry = &table_bytes[entry_off..entry_off + entry_size as usize];
                
                let mut is_zero = true;
                for &b in &entry[0..16] {
                    if b != 0 {
                        is_zero = false;
                        break;
                    }
                }
                
                if !is_zero {
                    valid_count += 1;
                    if valid_count == target_index {
                        let first_lba = LittleEndian::read_u64(&entry[0x20..0x28]);
                        return Ok(first_lba * sector_size);
                    }
                }
            }
            Err(Ext4Error::Io { msg: format!("GPT partition {} not found", target_index) })
        } else {
            let mut valid_count = 0;
            for i in 0..4 {
                let offset = 0x1BE + (i * 16);
                let part_type = mbr[offset + 4];
                if part_type != 0 {
                    valid_count += 1;
                    if valid_count == target_index {
                        let lba_start = LittleEndian::read_u32(&mbr[offset + 8..offset + 12]) as u64;
                        return Ok(lba_start * sector_size);
                    }
                }
            }
            Err(Ext4Error::Io { msg: format!("MBR partition {} not found", target_index) })
        }
    }
}
