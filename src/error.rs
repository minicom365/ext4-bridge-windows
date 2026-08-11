use std::fmt;

#[derive(Debug, uniffi::Error)]
pub enum Ext4Error {
    Io { msg: String },
    InvalidSuperblock { magic: u16 },
    PathNotFound { path: String },
    UnsupportedFeature { reason: String },
}

impl fmt::Display for Ext4Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ext4Error::Io { msg } => write!(f, "IO Error: {}", msg),
            Ext4Error::InvalidSuperblock { magic } => write!(f, "Invalid Superblock Magic: 0x{:X}", magic),
            Ext4Error::PathNotFound { path } => write!(f, "Path Not Found: {}", path),
            Ext4Error::UnsupportedFeature { reason } => write!(f, "Unsupported Feature: {}", reason),
        }
    }
}

impl std::error::Error for Ext4Error {}

impl From<String> for Ext4Error {
    fn from(err: String) -> Self {
        Ext4Error::Io { msg: err }
    }
}
