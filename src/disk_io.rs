use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr::null_mut;

#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
type HANDLE = *mut std::ffi::c_void;
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
type BOOL = i32;
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
type DWORD = u32;
#[allow(non_camel_case_types)]
type LARGE_INTEGER = i64;

const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
const GENERIC_READ: DWORD = 0x80000000;
const GENERIC_WRITE: DWORD = 0x40000000;
const FILE_SHARE_READ: DWORD = 0x00000001;
const FILE_SHARE_WRITE: DWORD = 0x00000002;
const OPEN_EXISTING: DWORD = 3;
#[allow(dead_code)]
const FILE_FLAG_NO_BUFFERING: DWORD = 0x20000000;
#[allow(dead_code)]
const FILE_FLAG_WRITE_THROUGH: DWORD = 0x80000000;
const FILE_ATTRIBUTE_NORMAL: DWORD = 0x80;
const FILE_BEGIN: DWORD = 0;

const MEM_COMMIT: DWORD = 0x1000;
const MEM_RESERVE: DWORD = 0x2000;
const PAGE_READWRITE: DWORD = 0x04;
const MEM_RELEASE: DWORD = 0x8000;

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
}

use windows_sys::Win32::System::Ioctl::{IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, DISK_GEOMETRY_EX};
use windows_sys::Win32::System::IO::DeviceIoControl;
use crate::error::Ext4Error;

pub struct AlignedWin32RawDisk {
    handle: HANDLE,
    sector_buf: *mut std::ffi::c_void,
    sector_buf_size: usize,
    pub sector_size: u64,
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
        let share = FILE_SHARE_READ | FILE_SHARE_WRITE;

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
            return Err(Ext4Error::Io { msg: format!("디스크 열기 실패: {}", disk_path) });
        }

        let sector_buf_size = 65536; // 64KB
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
            return Err(Ext4Error::Io { msg: "VirtualAlloc 메모리 할당 실패".to_string() });
        }

        let mut disk = Self { handle, sector_buf, sector_buf_size, sector_size: 512, pos: 0 };
        disk.sector_size = disk.get_sector_size() as u64;
        Ok(disk)
    }

    pub fn seek(&mut self, offset: u64) {
        self.pos = offset;
    }

    pub fn read_aligned(&mut self, start_offset: u64, total_bytes: usize) -> Result<Vec<u8>, Ext4Error> {
        if total_bytes > self.sector_buf_size {
            return Err(Ext4Error::Io { msg: format!("읽기 요청 크기가 버퍼 초과: {}", total_bytes) });
        }
        
        let mut new_pos: i64 = 0;
        let set_res = unsafe {
            SetFilePointerEx(
                self.handle,
                start_offset as i64,
                &mut new_pos as *mut i64,
                FILE_BEGIN,
            )
        };
        if set_res == 0 {
            return Err(Ext4Error::Io { msg: format!("Seek 실패 (Offset {})", start_offset) });
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
            Err(Ext4Error::Io { msg: format!("Read 실패 (Offset {}, req {}, read {})", start_offset, total_bytes, bytes_read) })
        }
    }
    
    pub fn write_aligned(&mut self, start_offset: u64, data: &[u8]) -> Result<(), Ext4Error> {
        let total_bytes = data.len();
        if total_bytes > self.sector_buf_size {
            return Err(Ext4Error::Io { msg: format!("쓰기 요청 크기가 버퍼 초과: {}", total_bytes) });
        }

        let mut new_pos: i64 = 0;
        let set_res = unsafe {
            SetFilePointerEx(
                self.handle,
                start_offset as i64,
                &mut new_pos as *mut i64,
                FILE_BEGIN,
            )
        };
        if set_res == 0 {
            return Err(Ext4Error::Io { msg: format!("Write Seek 실패 (Offset {})", start_offset) });
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
                return Err(Ext4Error::Io { msg: format!("FlushFileBuffers 실패 (Offset {})", start_offset) });
            }
            Ok(())
        } else {
            Err(Ext4Error::Io { msg: format!("Write 실패 (Offset {})", start_offset) })
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

    pub fn get_sector_size(&self) -> u32 {
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

pub struct LwExt4BlockDevice {
    pub disk: std::sync::Arc<std::sync::Mutex<AlignedWin32RawDisk>>,
    pub part_offset: u64,
    pub block_count: u64,
    pub sector_size: u32,
}

impl ext4_lwext4::blockdev::BlockDevice for LwExt4BlockDevice {
    fn read_blocks(&self, block_id: u64, buf: &mut [u8]) -> ext4_lwext4::error::Result<u32> {
        let mut disk = self.disk.lock().unwrap();
        let pos = self.part_offset + (block_id * self.sector_size as u64);
        let data = disk.read_at(pos, buf.len()).map_err(|_| ext4_lwext4::error::Error::Io(std::io::Error::new(std::io::ErrorKind::Other, "IO Error")))?;
        buf.copy_from_slice(&data);
        Ok((buf.len() / self.sector_size as usize) as u32)
    }

    fn write_blocks(&mut self, block_id: u64, buf: &[u8]) -> ext4_lwext4::error::Result<u32> {
        let mut disk = self.disk.lock().unwrap();
        let pos = self.part_offset + (block_id * self.sector_size as u64);
        disk.write_at(pos, buf).map_err(|_| ext4_lwext4::error::Error::Io(std::io::Error::new(std::io::ErrorKind::Other, "IO Error")))?;
        Ok((buf.len() / self.sector_size as usize) as u32)
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
