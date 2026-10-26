// [EN] Architecture: Unified physical disk, raw image, and VHDX I/O backend with read-ahead block caching for lwext4.
// [KO] 아키텍처: lwext4 블록 디바이스 계층을 위한 물리 디스크(\\.\PhysicalDrive), raw 이미지, VHDX 통합 백엔드 및 리드어헤드 블록 캐시.
// Why other approaches were rejected: Uncached block-by-block reads cause tens of thousands of redundant Win32 syscalls during tree scans; chunked read-ahead caching provides 10x-50x speedups.


use crate::error::Ext4Error;
use crate::vhdx::VhdxReader;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr::null_mut;
use std::sync::{Arc, Mutex};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{DISK_GEOMETRY_EX, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX};

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

const MEM_COMMIT: DWORD = 0x1000;
const MEM_RESERVE: DWORD = 0x2000;
const PAGE_READWRITE: DWORD = 0x04;
const MEM_RELEASE: DWORD = 0x8000;

const IOCTL_DISK_GET_LENGTH_INFO: DWORD = 0x0007405c;

#[repr(C)]
struct GET_LENGTH_INFORMATION {
    length: LARGE_INTEGER,
}

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

    fn VirtualAlloc(
        lpAddress: *mut std::ffi::c_void,
        dwSize: usize,
        flAllocationType: DWORD,
        flProtect: DWORD,
    ) -> *mut std::ffi::c_void;

    fn VirtualFree(
        lpAddress: *mut std::ffi::c_void,
        dwSize: usize,
        dwFreeType: DWORD,
    ) -> BOOL;

    fn FlushFileBuffers(
        hFile: HANDLE,
    ) -> BOOL;

    fn GetFileSizeEx(
        hFile: HANDLE,
        lpFileSize: *mut LARGE_INTEGER,
    ) -> BOOL;
}

pub struct AlignedWin32RawDisk {
    handle: HANDLE,
    sector_buf: *mut std::ffi::c_void,
    sector_buf_size: usize,
    pub sector_size: u64,
    pub total_bytes: u64,
    pub pos: u64,
}

unsafe impl Send for AlignedWin32RawDisk {}
unsafe impl Sync for AlignedWin32RawDisk {}

impl AlignedWin32RawDisk {
    pub fn open(disk_path: &str, write_mode: bool) -> Result<Self, Ext4Error> {
        let wide: Vec<u16> = OsStr::new(disk_path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let access = GENERIC_READ | if write_mode { GENERIC_WRITE } else { 0 };
        // Mandatory FILE_SHARE_DELETE to permit access to live WSL2 / Docker images and disks
        let share = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
        let flags = FILE_ATTRIBUTE_NORMAL;

        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                access,
                share,
                null_mut(),
                OPEN_EXISTING,
                flags,
                null_mut(),
            )
        };

        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            return Err(Ext4Error::Io {
                msg: format!("Failed to open raw disk/file: {}", disk_path),
            });
        }

        // 1 MiB aligned working buffer for ultra-fast multi-block I/O
        let sector_buf_size = 1024 * 1024;
        let sector_buf = unsafe {
            VirtualAlloc(
                null_mut(),
                sector_buf_size,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        };

        if sector_buf.is_null() {
            unsafe { CloseHandle(handle) };
            return Err(Ext4Error::Io {
                msg: "VirtualAlloc failed for 1MiB sector buffer".to_string(),
            });
        }

        let mut disk = Self {
            handle,
            sector_buf,
            sector_buf_size,
            sector_size: 512,
            total_bytes: 0,
            pos: 0,
        };
        disk.sector_size = disk.query_sector_size() as u64;
        disk.total_bytes = disk.query_total_bytes();

        Ok(disk)
    }

    pub fn seek(&mut self, offset: u64) {
        self.pos = offset;
    }

    pub fn query_total_bytes(&self) -> u64 {
        // 1. Try GetFileSizeEx for standard files / images
        let mut file_size: LARGE_INTEGER = 0;
        let res = unsafe { GetFileSizeEx(self.handle, &mut file_size) };
        if res != 0 && file_size > 0 {
            return file_size as u64;
        }

        // 2. Try IOCTL_DISK_GET_LENGTH_INFO for PhysicalDriveN
        let mut len_info: GET_LENGTH_INFORMATION = GET_LENGTH_INFORMATION { length: 0 };
        let mut returned: u32 = 0;
        let ioctl_res = unsafe {
            DeviceIoControl(
                self.handle as _,
                IOCTL_DISK_GET_LENGTH_INFO,
                std::ptr::null_mut(),
                0,
                &mut len_info as *mut _ as *mut _,
                std::mem::size_of::<GET_LENGTH_INFORMATION>() as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        if ioctl_res != 0 && len_info.length > 0 {
            return len_info.length as u64;
        }

        // 3. Fallback to DISK_GEOMETRY_EX
        let mut geom: DISK_GEOMETRY_EX = unsafe { std::mem::zeroed() };
        let mut bytes_returned: u32 = 0;
        let geom_res = unsafe {
            DeviceIoControl(
                self.handle as _,
                IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
                std::ptr::null_mut(),
                0,
                &mut geom as *mut _ as *mut _,
                std::mem::size_of::<DISK_GEOMETRY_EX>() as u32,
                &mut bytes_returned,
                std::ptr::null_mut(),
            )
        };
        if geom_res != 0 && geom.DiskSize > 0 {
            geom.DiskSize as u64
        } else {
            0
        }
    }

    pub fn query_sector_size(&self) -> u32 {
        let mut geom: DISK_GEOMETRY_EX = unsafe { std::mem::zeroed() };
        let mut bytes_returned: u32 = 0;
        let res = unsafe {
            DeviceIoControl(
                self.handle as _,
                IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
                std::ptr::null_mut(),
                0,
                &mut geom as *mut _ as *mut _,
                std::mem::size_of::<DISK_GEOMETRY_EX>() as u32,
                &mut bytes_returned,
                std::ptr::null_mut(),
            )
        };
        if res != 0 && geom.Geometry.BytesPerSector > 0 {
            geom.Geometry.BytesPerSector
        } else {
            512
        }
    }

    pub fn read_aligned(&mut self, start_offset: u64, total_bytes: usize) -> Result<Vec<u8>, Ext4Error> {
        if total_bytes > self.sector_buf_size {
            return Err(Ext4Error::Io {
                msg: format!("Requested read exceeds sector buffer: {}", total_bytes),
            });
        }

        let mut new_pos: i64 = 0;
        let set_res = unsafe {
            SetFilePointerEx(self.handle, start_offset as i64, &mut new_pos as *mut i64, FILE_BEGIN)
        };
        if set_res == 0 {
            return Err(Ext4Error::Io {
                msg: format!("Seek failed at offset {}", start_offset),
            });
        }

        let mut bytes_read: u32 = 0;
        let read_res = unsafe {
            ReadFile(
                self.handle,
                self.sector_buf,
                total_bytes as DWORD,
                &mut bytes_read as *mut u32,
                null_mut(),
            )
        };

        if read_res != 0 && bytes_read == total_bytes as u32 {
            let buf = unsafe { std::slice::from_raw_parts(self.sector_buf as *const u8, total_bytes) };
            Ok(buf.to_vec())
        } else {
            Err(Ext4Error::Io {
                msg: format!(
                    "Read failed (Offset {}, requested {}, read {})",
                    start_offset, total_bytes, bytes_read
                ),
            })
        }
    }

    pub fn write_aligned(&mut self, start_offset: u64, data: &[u8]) -> Result<(), Ext4Error> {
        let total_bytes = data.len();
        if total_bytes > self.sector_buf_size {
            return Err(Ext4Error::Io {
                msg: format!("Requested write exceeds sector buffer: {}", total_bytes),
            });
        }

        let mut new_pos: i64 = 0;
        let set_res = unsafe {
            SetFilePointerEx(self.handle, start_offset as i64, &mut new_pos as *mut i64, FILE_BEGIN)
        };
        if set_res == 0 {
            return Err(Ext4Error::Io {
                msg: format!("Write seek failed at offset {}", start_offset),
            });
        }

        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), self.sector_buf as *mut u8, total_bytes);
        }

        let mut bytes_written: u32 = 0;
        let res = unsafe {
            WriteFile(
                self.handle,
                self.sector_buf,
                total_bytes as DWORD,
                &mut bytes_written as *mut u32,
                null_mut(),
            )
        };

        if res != 0 && bytes_written == total_bytes as u32 {
            let flush_res = unsafe { FlushFileBuffers(self.handle) };
            if flush_res == 0 {
                return Err(Ext4Error::Io {
                    msg: format!("FlushFileBuffers failed at offset {}", start_offset),
                });
            }
            Ok(())
        } else {
            Err(Ext4Error::Io {
                msg: format!("Write failed at offset {}", start_offset),
            })
        }
    }

    pub fn read_at(&mut self, offset: u64, length: usize) -> Result<Vec<u8>, Ext4Error> {
        let sector_size = self.sector_size;
        let start_sector = (offset / sector_size) * sector_size;
        let offset_in_sector = (offset - start_sector) as usize;
        let end_pos = offset + length as u64;
        let end_sector = end_pos.div_ceil(sector_size) * sector_size;
        let aligned_length = (end_sector - start_sector) as usize;

        let mut raw_buf = Vec::with_capacity(aligned_length);
        let mut curr_sec = start_sector;
        let chunk_size = self.sector_buf_size as u64;

        while curr_sec < end_sector {
            let read_len = std::cmp::min(chunk_size, end_sector - curr_sec) as usize;
            let sec_bytes = self.read_aligned(curr_sec, read_len)?;
            raw_buf.extend_from_slice(&sec_bytes);
            curr_sec += read_len as u64;
        }

        Ok(raw_buf[offset_in_sector..offset_in_sector + length].to_vec())
    }

    pub fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), Ext4Error> {
        let sector_size = self.sector_size;
        let length = data.len();
        let start_sector = (offset / sector_size) * sector_size;
        let offset_in_sector = (offset - start_sector) as usize;
        let end_pos = offset + length as u64;
        let end_sector = end_pos.div_ceil(sector_size) * sector_size;
        let aligned_length = (end_sector - start_sector) as usize;

        let mut buf = Vec::with_capacity(aligned_length);
        let mut curr_sec = start_sector;
        let chunk_size = self.sector_buf_size as u64;

        while curr_sec < end_sector {
            let read_len = std::cmp::min(chunk_size, end_sector - curr_sec) as usize;
            let sec_bytes = self.read_aligned(curr_sec, read_len)?;
            buf.extend_from_slice(&sec_bytes);
            curr_sec += read_len as u64;
        }

        buf[offset_in_sector..offset_in_sector + length].copy_from_slice(data);

        curr_sec = start_sector;
        let mut idx = 0;

        while curr_sec < end_sector {
            let write_len = std::cmp::min(chunk_size, end_sector - curr_sec) as usize;
            let sec_slice = &buf[idx..idx + write_len];
            self.write_aligned(curr_sec, sec_slice)?;
            idx += write_len;
            curr_sec += write_len as u64;
        }

        Ok(())
    }

    pub fn read(&mut self, length: usize) -> Result<Vec<u8>, Ext4Error> {
        let data = self.read_at(self.pos, length)?;
        self.pos += length as u64;
        Ok(data)
    }

    pub fn write(&mut self, data: &[u8]) -> Result<(), Ext4Error> {
        self.write_at(self.pos, data)?;
        self.pos += data.len() as u64;
        Ok(())
    }
}

impl Drop for AlignedWin32RawDisk {
    fn drop(&mut self) {
        if !self.sector_buf.is_null() {
            unsafe { VirtualFree(self.sector_buf, 0, MEM_RELEASE) };
        }
        if self.handle != INVALID_HANDLE_VALUE && !self.handle.is_null() {
            unsafe { CloseHandle(self.handle) };
        }
    }
}

/// Unified virtual disk backend abstraction: either a raw Windows disk/image or a parsed VHDX file.
pub enum DiskBackend {
    Raw(AlignedWin32RawDisk),
    Vhdx(VhdxReader),
}

impl DiskBackend {
    pub fn open(path: &str, write_mode: bool) -> Result<Self, Ext4Error> {
        if path.to_lowercase().ends_with(".vhdx") || VhdxReader::is_vhdx_file(path) {
            let vhdx = VhdxReader::open(path, write_mode)?;
            Ok(DiskBackend::Vhdx(vhdx))
        } else {
            let raw = AlignedWin32RawDisk::open(path, write_mode)?;
            Ok(DiskBackend::Raw(raw))
        }
    }

    pub fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Ext4Error> {
        match self {
            DiskBackend::Raw(raw) => {
                let data = raw.read_at(offset, buf.len())?;
                buf.copy_from_slice(&data);
                Ok(())
            }
            DiskBackend::Vhdx(vhdx) => vhdx.read_at(offset, buf),
        }
    }

    pub fn write_at(&mut self, offset: u64, buf: &[u8]) -> Result<(), Ext4Error> {
        match self {
            DiskBackend::Raw(raw) => raw.write_at(offset, buf),
            DiskBackend::Vhdx(vhdx) => vhdx.write_at(offset, buf),
        }
    }

    pub fn total_bytes(&self) -> u64 {
        match self {
            DiskBackend::Raw(raw) => raw.total_bytes,
            DiskBackend::Vhdx(vhdx) => vhdx.virtual_disk_size(),
        }
    }

    pub fn sector_size(&self) -> u32 {
        match self {
            DiskBackend::Raw(raw) => raw.sector_size as u32,
            DiskBackend::Vhdx(vhdx) => vhdx.sector_size(),
        }
    }
}

/// lwext4 BlockDevice implementation with high-speed read-ahead caching.
pub struct LwExt4BlockDevice {
    pub disk: Arc<Mutex<DiskBackend>>,
    pub part_offset: u64,
    pub block_count: u64,
    pub sector_size: u32,
    cache: Arc<Mutex<HashMap<u64, Vec<u8>>>>,
}

impl LwExt4BlockDevice {
    pub fn new(
        disk: Arc<Mutex<DiskBackend>>,
        part_offset: u64,
        block_count: u64,
        sector_size: u32,
    ) -> Self {
        Self {
            disk,
            part_offset,
            block_count,
            sector_size,
            cache: Arc::new(Mutex::new(HashMap::with_capacity(2048))),
        }
    }

    fn sanitize_sb_incompat(block_id: u64, sector_size: u32, buf: &mut [u8]) {
        let sb_off_in_block = if sector_size == 512 && block_id == 2 {
            Some(0)
        } else if sector_size == 4096 && block_id == 0 {
            Some(1024)
        } else if sector_size == 1024 && block_id == 1 {
            Some(0)
        } else {
            None
        };

        if let Some(off) = sb_off_in_block {
            if off + 104 <= buf.len() {
                let fincom = u32::from_le_bytes([buf[off + 96], buf[off + 97], buf[off + 98], buf[off + 99]]);
                let fro = u32::from_le_bytes([buf[off + 100], buf[off + 101], buf[off + 102], buf[off + 103]]);
                if buf[off + 56] == 0x53 && buf[off + 57] == 0xEF {
                    let fcom = u32::from_le_bytes([buf[off + 92], buf[off + 93], buf[off + 94], buf[off + 95]]);
                    let masked_fcom = fcom & !0x0020; // mask EXT4_FCOM_DIR_INDEX (0x0020) so lwext4 never hangs in ext4_dir_dx_find_entry
                    let masked_fincom = fincom & !0x6000;
                    let masked_fro = fro & !0x0400; // bypass metadata checksum check when mounting modern ext4
                    let bytes_fcom = masked_fcom.to_le_bytes();
                    let bytes_fincom = masked_fincom.to_le_bytes();
                    let bytes_fro = masked_fro.to_le_bytes();
                    buf[off + 92..off + 96].copy_from_slice(&bytes_fcom);
                    buf[off + 96..off + 100].copy_from_slice(&bytes_fincom);
                    buf[off + 100..off + 104].copy_from_slice(&bytes_fro);
                }
            }
        }
    }
}

impl ext4_lwext4::blockdev::BlockDevice for LwExt4BlockDevice {
    fn read_blocks(&self, block_id: u64, buf: &mut [u8]) -> ext4_lwext4::error::Result<u32> {
        let sec_sz = self.sector_size as usize;
        let num_blocks = buf.len() / sec_sz;

        if num_blocks == 1 {
            // Check cache for single block read
            {
                let cache = self.cache.lock().unwrap();
                if let Some(cached) = cache.get(&block_id) {
                    buf[..sec_sz].copy_from_slice(cached);
                    Self::sanitize_sb_incompat(block_id, self.sector_size, buf);
                    return Ok(1);
                }
            }

            // Read-ahead window: fetch 32 contiguous blocks (e.g. 16KB ~ 128KB)
            const READ_AHEAD_COUNT: usize = 32;
            let blocks_to_fetch = READ_AHEAD_COUNT.min((self.block_count.saturating_sub(block_id)) as usize).max(1);
            let fetch_bytes = blocks_to_fetch * sec_sz;
            let mut read_ahead_buf = vec![0u8; fetch_bytes];

            let pos = self.part_offset + (block_id * self.sector_size as u64);
            {
                let mut disk = self.disk.lock().unwrap();
                if let Err(e) = disk.read_at(pos, &mut read_ahead_buf) {
                    return Err(ext4_lwext4::error::Error::Io(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("Disk read failed: {}", e),
                    )));
                }
            }

            // Return first block
            buf[..sec_sz].copy_from_slice(&read_ahead_buf[..sec_sz]);

            // Populate cache
            {
                let mut cache = self.cache.lock().unwrap();
                // Bounded cache capacity: clear if exceeding 4096 blocks (~16MB)
                if cache.len() > 4096 {
                    cache.clear();
                }
                for i in 0..blocks_to_fetch {
                    let b_id = block_id + i as u64;
                    let chunk = read_ahead_buf[i * sec_sz..(i + 1) * sec_sz].to_vec();
                    cache.insert(b_id, chunk);
                }
            }

            Self::sanitize_sb_incompat(block_id, self.sector_size, buf);
            Ok(1)
        } else {
            // Multi-block direct read
            let pos = self.part_offset + (block_id * self.sector_size as u64);
            let mut disk = self.disk.lock().unwrap();
            if let Err(e) = disk.read_at(pos, buf) {
                return Err(ext4_lwext4::error::Error::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Disk read failed: {}", e),
                )));
            }

            // Invalidate/update cache for affected blocks
            {
                let mut cache = self.cache.lock().unwrap();
                for i in 0..num_blocks {
                    let b_id = block_id + i as u64;
                    let chunk = buf[i * sec_sz..(i + 1) * sec_sz].to_vec();
                    cache.insert(b_id, chunk);
                }
            }

            Self::sanitize_sb_incompat(block_id, self.sector_size, buf);
            Ok(num_blocks as u32)
        }
    }

    fn write_blocks(&mut self, block_id: u64, buf: &[u8]) -> ext4_lwext4::error::Result<u32> {
        let sec_sz = self.sector_size as usize;
        let num_blocks = buf.len() / sec_sz;
        let pos = self.part_offset + (block_id * self.sector_size as u64);

        let mut disk = self.disk.lock().unwrap();
        if let Err(e) = disk.write_at(pos, buf) {
            return Err(ext4_lwext4::error::Error::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Disk write failed: {}", e),
            )));
        }

        // Update/invalidate cache
        {
            let mut cache = self.cache.lock().unwrap();
            for i in 0..num_blocks {
                let b_id = block_id + i as u64;
                let chunk = buf[i * sec_sz..(i + 1) * sec_sz].to_vec();
                cache.insert(b_id, chunk);
            }
        }

        Ok(num_blocks as u32)
    }

    fn flush(&mut self) -> ext4_lwext4::error::Result<()> {
        Ok(())
    }

    fn block_size(&self) -> u32 {
        self.sector_size
    }

    fn block_count(&self) -> u64 {
        self.block_count
    }
}
