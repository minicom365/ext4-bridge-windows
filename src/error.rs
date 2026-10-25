// [EN] Architecture: Unified error taxonomy for ext4 filesystem, disk I/O, VHDX parsing, and FFI operations.
// [KO] 아키텍처: ext4 파일 시스템 조작, 디스크 I/O, VHDX 파싱, FFI 경계 전반에 대한 통합 에러 타입 정의.
// Why other approaches were rejected: Silent error swallowing (empty returns/dummy fallbacks) violates reliable telemetry; strongly-typed errors ensure caller diagnosability.

use std::fmt;

#[derive(Debug, uniffi::Error)]
pub enum Ext4Error {
    Io { msg: String },
    InvalidSuperblock { magic: u16 },
    PathNotFound { path: String },
    UnsupportedFeature { reason: String },
    InvalidVhdx { msg: String },
    PartitionNotFound { index: u32, msg: String },
    BufferTooSmall { required: u64, provided: u64 },
}

impl fmt::Display for Ext4Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ext4Error::Io { msg } => write!(f, "IO Error: {}", msg),
            Ext4Error::InvalidSuperblock { magic } => write!(f, "Invalid Superblock Magic: 0x{:04X}", magic),
            Ext4Error::PathNotFound { path } => write!(f, "Path Not Found: {}", path),
            Ext4Error::UnsupportedFeature { reason } => write!(f, "Unsupported Feature: {}", reason),
            Ext4Error::InvalidVhdx { msg } => write!(f, "Invalid VHDX: {}", msg),
            Ext4Error::PartitionNotFound { index, msg } => write!(f, "Partition #{} Not Found: {}", index, msg),
            Ext4Error::BufferTooSmall { required, provided } => {
                write!(f, "Buffer Too Small: required {} bytes, provided {} bytes", required, provided)
            }
        }
    }
}

impl std::error::Error for Ext4Error {}

impl From<String> for Ext4Error {
    fn from(err: String) -> Self {
        Ext4Error::Io { msg: err }
    }
}

impl From<std::io::Error> for Ext4Error {
    fn from(err: std::io::Error) -> Self {
        Ext4Error::Io { msg: err.to_string() }
    }
}
