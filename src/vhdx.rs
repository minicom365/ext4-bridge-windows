// [EN] Architecture: Zero-overhead Windows user-mode VHDX (Fixed & Dynamic v1/v2) Block Allocation Table (BAT) parser.
// [KO] 아키텍처: 가상 디스크 드라이버 마운트 없이 유저모드에서 WSL2/Docker 및 고정/동적 VHDX 블록 할당 테이블(BAT)을 직접 매핑하는 변환기.
// Why other approaches were rejected: Requiring Win32 Virtual Disk service or WSL mounting requires admin privileges and causes file locks; direct BAT parsing works unprivileged in milliseconds with FILE_SHARE_DELETE.

use byteorder::{ByteOrder, LittleEndian};
use crate::error::Ext4Error;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr::null_mut;

#[allow(non_camel_case_types)]
type HANDLE = *mut std::ffi::c_void;
#[allow(non_camel_case_types)]
type BOOL = i32;
#[allow(non_camel_case_types)]
type DWORD = u32;
#[allow(non_camel_case_types)]
type LARGE_INTEGER = i64;

const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
const GENERIC_READ: DWORD = 0x80000000;
const GENERIC_WRITE: DWORD = 0x40000000;
const FILE_SHARE_READ: DWORD = 0x00000001;
const FILE_SHARE_WRITE: DWORD = 0x00000002;
const FILE_SHARE_DELETE: DWORD = 0x00000004;
const OPEN_EXISTING: DWORD = 3;
const FILE_ATTRIBUTE_NORMAL: DWORD = 0x80;
const FILE_BEGIN: DWORD = 0;

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(
        lpFileName: *const u16,
        dwDesiredAccess: DWORD,
        dwShareMode: DWORD,
        lpSecurityAttributes: *mut std::ffi::c_void,
        dwCreationDisposition: DWORD,
        dwFlagsAndAttributes: DWORD,
        hTemplateFile: HANDLE,
    ) -> HANDLE;

    fn ReadFile(
        hFile: HANDLE,
        lpBuffer: *mut std::ffi::c_void,
        nNumberOfBytesToRead: DWORD,
        lpNumberOfBytesRead: *mut DWORD,
        lpOverlapped: *mut std::ffi::c_void,
    ) -> BOOL;

    fn WriteFile(
        hFile: HANDLE,
        lpBuffer: *const std::ffi::c_void,
        nNumberOfBytesToWrite: DWORD,
        lpNumberOfBytesWritten: *mut DWORD,
        lpOverlapped: *mut std::ffi::c_void,
    ) -> BOOL;

    fn SetFilePointerEx(
        hFile: HANDLE,
        liDistanceToMove: LARGE_INTEGER,
        lpNewFilePointer: *mut LARGE_INTEGER,
        dwMoveMethod: DWORD,
    ) -> BOOL;

    fn CloseHandle(hObject: HANDLE) -> BOOL;
}

// Well-known GUIDs for VHDX region and metadata items
const GUID_BAT_REGION: [u8; 16] = [
    0x66, 0x77, 0xc2, 0x2d, 0x23, 0xf6, 0x00, 0x42, 0x9d, 0x64, 0x11, 0x5e, 0x9b, 0xfd, 0x4a, 0x08,
];
const GUID_METADATA_REGION: [u8; 16] = [
    0xa4, 0x40, 0x7d, 0x8b, 0x99, 0x07, 0x40, 0x4b, 0xac, 0x85, 0x83, 0xe6, 0xe0, 0x82, 0xc3, 0x80,
];
const GUID_METADATA_REGION_ALT: [u8; 16] = [
    0x06, 0xa2, 0x7c, 0x8b, 0x90, 0x47, 0x9a, 0x4b, 0xb8, 0xfe, 0x57, 0x5f, 0x05, 0x0f, 0x88, 0x6e,
];

// Metadata Item GUIDs
const GUID_FILE_PARAMETERS: [u8; 16] = [
    0x37, 0x67, 0xa1, 0xca, 0x36, 0xfa, 0x43, 0x4d, 0xb3, 0xb6, 0x33, 0xf0, 0xaa, 0x44, 0xe7, 0x6b,
];
const GUID_VIRTUAL_DISK_SIZE_1: [u8; 16] = [
    0x61, 0x42, 0xa5, 0x2f, 0x76, 0xb2, 0x34, 0x4c, 0xbd, 0x61, 0x61, 0xea, 0xf8, 0x15, 0x04, 0xe4,
];
const GUID_VIRTUAL_DISK_SIZE_2: [u8; 16] = [
    0x24, 0x42, 0xa5, 0x2f, 0x1b, 0xcd, 0x76, 0x48, 0xb2, 0x11, 0x5d, 0xbe, 0xd8, 0x3b, 0xf4, 0xb8,
];
const GUID_LOGICAL_SECTOR_SIZE_1: [u8; 16] = [
    0x1d, 0xbf, 0x41, 0x81, 0x63, 0xa9, 0x09, 0x47, 0xb7, 0x01, 0x7f, 0x8d, 0xec, 0x61, 0x04, 0x4b,
];
const GUID_LOGICAL_SECTOR_SIZE_2: [u8; 16] = [
    0x1d, 0xbf, 0x41, 0x81, 0x6f, 0xa9, 0x09, 0x47, 0xba, 0x47, 0xf2, 0x33, 0xa8, 0xfa, 0xab, 0x5f,
];
const GUID_PHYSICAL_SECTOR_SIZE: [u8; 16] = [
    0xc7, 0x48, 0xa3, 0xcd, 0x5d, 0x44, 0x71, 0x49, 0x9c, 0xc9, 0xe9, 0x88, 0x52, 0x51, 0xc5, 0x56,
];

#[derive(Debug, Clone)]
pub struct VhdxMetadata {
    pub block_size: u32,
    pub is_fixed: bool,
    pub has_parent: bool,
    pub virtual_disk_size: u64,
    pub logical_sector_size: u32,
    pub physical_sector_size: u32,
    pub chunk_ratio: u64,
}

pub struct VhdxReader {
    handle: HANDLE,
    pub metadata: VhdxMetadata,
    pub bat: Vec<u64>,
    pub payload_blocks: u64,
}

unsafe impl Send for VhdxReader {}
unsafe impl Sync for VhdxReader {}

impl VhdxReader {
    pub fn is_vhdx_file(path: &str) -> bool {
        let wide: Vec<u16> = OsStr::new(path).encode_wide().chain(std::iter::once(0)).collect();
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null_mut(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            return false;
        }

        let mut magic = [0u8; 8];
        let mut read_bytes: DWORD = 0;
        let ok = unsafe {
            ReadFile(
                handle,
                magic.as_mut_ptr() as *mut _,
                8,
                &mut read_bytes,
                null_mut(),
            )
        };
        unsafe { CloseHandle(handle) };

        ok != 0 && read_bytes == 8 && &magic == b"vhdxfile"
    }

    pub fn open(path: &str, write_mode: bool) -> Result<Self, Ext4Error> {
        let wide: Vec<u16> = OsStr::new(path).encode_wide().chain(std::iter::once(0)).collect();
        let access = GENERIC_READ | if write_mode { GENERIC_WRITE } else { 0 };
        let share = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;

        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                access,
                share,
                null_mut(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            )
        };

        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            return Err(Ext4Error::Io { msg: format!("Failed to open VHDX file: {}", path) });
        }

        let mut reader = Self {
            handle,
            metadata: VhdxMetadata {
                block_size: 1048576,
                is_fixed: false,
                has_parent: false,
                virtual_disk_size: 0,
                logical_sector_size: 512,
                physical_sector_size: 4096,
                chunk_ratio: 0,
            },
            bat: Vec::new(),
            payload_blocks: 0,
        };

        match reader.init_vhdx() {
            Ok(()) => Ok(reader),
            Err(e) => {
                unsafe { CloseHandle(handle) };
                Err(e)
            }
        }
    }

    fn read_raw(&self, offset: u64, buf: &mut [u8]) -> Result<(), Ext4Error> {
        let mut new_pos: i64 = 0;
        let set_res = unsafe {
            SetFilePointerEx(self.handle, offset as i64, &mut new_pos as *mut i64, FILE_BEGIN)
        };
        if set_res == 0 {
            return Err(Ext4Error::Io { msg: format!("VHDX seek failed to offset {}", offset) });
        }

        let mut read_bytes: DWORD = 0;
        let res = unsafe {
            ReadFile(
                self.handle,
                buf.as_mut_ptr() as *mut _,
                buf.len() as DWORD,
                &mut read_bytes,
                null_mut(),
            )
        };

        if res != 0 && read_bytes == buf.len() as DWORD {
            Ok(())
        } else {
            Err(Ext4Error::Io {
                msg: format!("VHDX raw read failed (offset {}, req {}, read {})", offset, buf.len(), read_bytes),
            })
        }
    }

    fn write_raw(&self, offset: u64, buf: &[u8]) -> Result<(), Ext4Error> {
        let mut new_pos: i64 = 0;
        let set_res = unsafe {
            SetFilePointerEx(self.handle, offset as i64, &mut new_pos as *mut i64, FILE_BEGIN)
        };
        if set_res == 0 {
            return Err(Ext4Error::Io { msg: format!("VHDX write seek failed to offset {}", offset) });
        }

        let mut written: DWORD = 0;
        let res = unsafe {
            WriteFile(
                self.handle,
                buf.as_ptr() as *const _,
                buf.len() as DWORD,
                &mut written,
                null_mut(),
            )
        };

        if res != 0 && written == buf.len() as DWORD {
            Ok(())
        } else {
            Err(Ext4Error::Io {
                msg: format!("VHDX raw write failed (offset {}, req {}, written {})", offset, buf.len(), written),
            })
        }
    }

    fn init_vhdx(&mut self) -> Result<(), Ext4Error> {
        // 1. Verify VHDX file signature (first 8 bytes)
        let mut sig = [0u8; 8];
        self.read_raw(0, &mut sig)?;
        if &sig != b"vhdxfile" {
            return Err(Ext4Error::InvalidVhdx { msg: "File does not start with 'vhdxfile' signature".into() });
        }

        // 2. Read Region Table (Offset 0x30000 = 196608)
        let mut region_header = [0u8; 64 * 1024];
        self.read_raw(196608, &mut region_header)?;
        if &region_header[0..4] != b"regi" {
            // Try secondary region table at 0x40000 = 262144
            self.read_raw(262144, &mut region_header)?;
            if &region_header[0..4] != b"regi" {
                return Err(Ext4Error::InvalidVhdx { msg: "Invalid region table magic".into() });
            }
        }

        let entry_count = LittleEndian::read_u32(&region_header[8..12]) as usize;
        let mut bat_offset: u64 = 0;
        let mut bat_length: u32 = 0;
        let mut metadata_offset: u64 = 0;
        let mut metadata_length: u32 = 0;

        for i in 0..entry_count.min(2047) {
            let off = 16 + i * 32;
            let guid = &region_header[off..off + 16];
            let file_off = LittleEndian::read_u64(&region_header[off + 16..off + 24]);
            let len = LittleEndian::read_u32(&region_header[off + 24..off + 28]);

            if guid == GUID_BAT_REGION {
                bat_offset = file_off;
                bat_length = len;
            } else if guid == GUID_METADATA_REGION || guid == GUID_METADATA_REGION_ALT {
                metadata_offset = file_off;
                metadata_length = len;
            }
        }

        if bat_offset == 0 || metadata_offset == 0 {
            return Err(Ext4Error::InvalidVhdx { msg: "Missing BAT or Metadata region in VHDX".into() });
        }

        // 3. Read Metadata Table
        let read_meta_len = (metadata_length as usize).min(1024 * 1024);
        let mut meta_buf = vec![0u8; read_meta_len];
        self.read_raw(metadata_offset, &mut meta_buf)?;
        if &meta_buf[0..8] != b"metadata" {
            return Err(Ext4Error::InvalidVhdx { msg: "Invalid metadata table signature".into() });
        }

        let meta_entries = LittleEndian::read_u16(&meta_buf[10..12]) as usize;
        let mut block_size: u32 = 1048576;
        let mut is_fixed = false;
        let mut has_parent = false;
        let mut virtual_disk_size: u64 = 0;
        let mut logical_sector_size: u32 = 512;
        let mut physical_sector_size: u32 = 4096;

        for i in 0..meta_entries.min(2047) {
            let entry_off = 32 + i * 32;
            if entry_off + 32 > meta_buf.len() {
                break;
            }
            let item_guid = &meta_buf[entry_off..entry_off + 16];
            let item_offset = LittleEndian::read_u32(&meta_buf[entry_off + 16..entry_off + 20]) as usize;
            let item_len = LittleEndian::read_u32(&meta_buf[entry_off + 20..entry_off + 24]) as usize;

            if item_offset + item_len <= meta_buf.len() {
                let payload = &meta_buf[item_offset..item_offset + item_len];
                if item_guid == GUID_FILE_PARAMETERS && payload.len() >= 8 {
                    block_size = LittleEndian::read_u32(&payload[0..4]);
                    let flags = LittleEndian::read_u32(&payload[4..8]);
                    is_fixed = (flags & 0x01) != 0;
                    has_parent = (flags & 0x02) != 0;
                } else if (item_guid == GUID_VIRTUAL_DISK_SIZE_1 || item_guid == GUID_VIRTUAL_DISK_SIZE_2) && payload.len() >= 8 {
                    virtual_disk_size = LittleEndian::read_u64(&payload[0..8]);
                } else if (item_guid == GUID_LOGICAL_SECTOR_SIZE_1 || item_guid == GUID_LOGICAL_SECTOR_SIZE_2) && payload.len() >= 4 {
                    logical_sector_size = LittleEndian::read_u32(&payload[0..4]);
                } else if item_guid == GUID_PHYSICAL_SECTOR_SIZE && payload.len() >= 4 {
                    physical_sector_size = LittleEndian::read_u32(&payload[0..4]);
                }
            }
        }

        if block_size == 0 || virtual_disk_size == 0 {
            return Err(Ext4Error::InvalidVhdx {
                msg: format!("Invalid VHDX geometry: block_size={}, virtual_disk_size={}", block_size, virtual_disk_size),
            });
        }

        let chunk_ratio = if block_size > 0 {
            (8388608u64 * (logical_sector_size as u64)) / (block_size as u64)
        } else {
            0
        };

        self.metadata = VhdxMetadata {
            block_size,
            is_fixed,
            has_parent,
            virtual_disk_size,
            logical_sector_size,
            physical_sector_size,
            chunk_ratio,
        };

        self.payload_blocks = (virtual_disk_size + block_size as u64 - 1) / block_size as u64;

        // 4. Read Block Allocation Table (BAT)
        let total_bat_entries = (bat_length / 8) as usize;
        // Cap BAT load to actual required entries to preserve memory
        let entries_to_load = if is_fixed || chunk_ratio == 0 {
            (self.payload_blocks as usize).min(total_bat_entries)
        } else {
            let max_idx = self.payload_blocks + (self.payload_blocks / chunk_ratio) + 2;
            (max_idx as usize).min(total_bat_entries)
        };

        let mut bat_bytes = vec![0u8; entries_to_load * 8];
        self.read_raw(bat_offset, &mut bat_bytes)?;

        self.bat = Vec::with_capacity(entries_to_load);
        for chunk in bat_bytes.chunks_exact(8) {
            self.bat.push(LittleEndian::read_u64(chunk));
        }

        Ok(())
    }

    /// Translates a virtual disk offset to (Option<file_offset>, contiguous_bytes_in_block).
    /// If Option is None, the block is unallocated/zeroed in the VHDX.
    #[inline]
    fn translate_offset(&self, virtual_offset: u64) -> (Option<u64>, usize) {
        if virtual_offset >= self.metadata.virtual_disk_size {
            return (None, 0);
        }

        let block_size = self.metadata.block_size as u64;
        let block_idx = virtual_offset / block_size;
        let in_block_offset = (virtual_offset % block_size) as usize;
        let bytes_left_in_block = (block_size as usize) - in_block_offset;

        let bat_idx = if self.metadata.is_fixed {
            block_idx
        } else if self.metadata.chunk_ratio > 0 && (self.bat.len() as u64) > self.payload_blocks {
            block_idx + (block_idx / self.metadata.chunk_ratio)
        } else {
            block_idx
        };

        if (bat_idx as usize) < self.bat.len() {
            let entry = self.bat[bat_idx as usize];
            let state = entry & 0x07;
            // 6 = Fully Present, 7 = Partially Present
            if state == 6 || state == 7 {
                let file_offset = (entry >> 20) * (1024 * 1024) + (in_block_offset as u64);
                (Some(file_offset), bytes_left_in_block)
            } else {
                (None, bytes_left_in_block) // Not present / unmapped -> zeros
            }
        } else {
            (None, bytes_left_in_block)
        }
    }

    pub fn read_at(&mut self, mut offset: u64, buf: &mut [u8]) -> Result<(), Ext4Error> {
        let mut buf_idx = 0;
        let total_len = buf.len();

        while buf_idx < total_len {
            let (file_offset_opt, block_avail) = self.translate_offset(offset);
            if block_avail == 0 {
                // Past virtual disk size: fill rest with zeros
                buf[buf_idx..].fill(0);
                break;
            }
            let to_read = block_avail.min(total_len - buf_idx);

            match file_offset_opt {
                Some(file_offset) => {
                    self.read_raw(file_offset, &mut buf[buf_idx..buf_idx + to_read])?;
                }
                None => {
                    buf[buf_idx..buf_idx + to_read].fill(0);
                }
            }

            buf_idx += to_read;
            offset += to_read as u64;
        }

        Ok(())
    }

    pub fn write_at(&mut self, mut offset: u64, buf: &[u8]) -> Result<(), Ext4Error> {
        let mut buf_idx = 0;
        let total_len = buf.len();

        while buf_idx < total_len {
            let (file_offset_opt, block_avail) = self.translate_offset(offset);
            if block_avail == 0 {
                return Err(Ext4Error::Io {
                    msg: format!("Write beyond VHDX virtual capacity: offset {}", offset),
                });
            }
            let to_write = block_avail.min(total_len - buf_idx);

            match file_offset_opt {
                Some(file_offset) => {
                    self.write_raw(file_offset, &buf[buf_idx..buf_idx + to_write])?;
                }
                None => {
                    return Err(Ext4Error::UnsupportedFeature {
                        reason: format!("Allocating new blocks in sparse dynamic VHDX is not supported (offset {})", offset),
                    });
                }
            }

            buf_idx += to_write;
            offset += to_write as u64;
        }

        Ok(())
    }

    pub fn virtual_disk_size(&self) -> u64 {
        self.metadata.virtual_disk_size
    }

    pub fn sector_size(&self) -> u32 {
        self.metadata.logical_sector_size
    }
}

impl Drop for VhdxReader {
    fn drop(&mut self) {
        if self.handle != INVALID_HANDLE_VALUE && !self.handle.is_null() {
            unsafe { CloseHandle(self.handle) };
        }
    }
}
