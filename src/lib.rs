// [EN] Architecture: Core crate root exposing safe Rust modules and UniFFI bindings for Windows ext4 bridging.
// [KO] 아키텍처: Windows 환경에서 ext4 파일 시스템 접근, VHDX 해석, 원시 디스크 I/O 및 UniFFI 바인딩을 제공하는 핵심 크레이트 루트.
// Why other approaches were rejected: Separate monolithic binaries caused duplicated disk I/O logic; modular library design enables both CLI and DLL C-ABI integration.

pub mod error;
pub mod vhdx;
pub mod disk_io;
pub mod ext4;
pub mod ffi;

uniffi::setup_scaffolding!();
