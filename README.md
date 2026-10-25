# ext4-bridge-windows

High-performance ext4 filesystem bridge, user-mode VHDX parser, and sub-millisecond recursive metadata scanner for Windows.

[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-Windows-lightgrey.svg)]()

---

## Key Capabilities

1. **Direct User-Mode VHDX Parser (Dynamic & Fixed v1/v2)**
   - Translates virtual disk addresses to physical payload offsets via the VHDX Block Allocation Table (BAT).
   - Inspects WSL2 and Docker Desktop `.vhdx` images directly from user mode without requiring hypervisors or elevated driver mounts.
   - Non-exclusive shared access (`FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`) enables inspecting active, live containers while Hyper-V holds them open.

2. **Ultra-Low Latency Tree Scanner for `SniffEverything.exe`**
   - High-throughput breadth-first directory traversal (BFS).
   - Zero-allocation C-ABI streaming callback interface (`ext4_tree_scan`) and streaming JSONL formatter (`ext4_scan_jsonl`).
   - Achieves sub-millisecond full root traversal (**~0.16 ms mean latency**, variance **245 ~ 502 µs²** on real WSL2 VHDX).

3. **Superblock Geometry & Modern Linux Flag Reconciliation**
   - Resolves block sizes dynamically (1024, 2048, 4096 bytes) and accurately computes 64-bit block counts without integer overflow.
   - Transparently masks modern incompatible feature flags (`EXT4_FEATURE_INCOMPAT_CSUM_SEED 0x2000`, `EXT4_FEATURE_RO_COMPAT_METADATA_CSUM 0x0400`) during read-only mounts, allowing seamless inspection of modern Linux distributions.

4. **Multi-Tier I/O Engine with Read-Ahead Block Cache**
   - 1 MiB aligned I/O memory buffers matching physical disk sectors.
   - 2048-entry bounded block cache with contiguous read-ahead prefetching, reducing Win32 syscall overhead by up to 50x during recursive directory traversals.

5. **Anti-REA Binary Hardening & Memory Safety**
   - Full symbol stripping (`strip = "symbols"`), link-time optimization (`lto = "fat"`), single codegen unit, and abort-on-panic.
   - C FFI buffer overflow prevention: caller buffer capacities are strictly enforced before memory transfers.
   - Pillar 10 OS process mitigations: dynamic DLL search order lockdown (`LOAD_LIBRARY_SEARCH_SYSTEM32`) to prevent DLL hijacking.

---

## Architecture Overview

```
                      +---------------------------------------+
                      |           Client Applications         |
                      |  (SniffEverything.exe / Native CLI)   |
                      +-------------------+-------------------+
                                          |
                        [C-ABI FFI / UniFFI Bindings]
                                          |
                      +-------------------+-------------------+
                      |               Ext4Bridge              |
                      |   - BFS Recursive Tree Scanner        |
                      |   - JSONL Metadata Serializer         |
                      |   - stat_fs Volume Geometry Probe     |
                      +-------------------+-------------------+
                                          |
                      +-------------------+-------------------+
                      |          lwext4 Subsystem             |
                      |   - Read-only flag sanitization       |
                      |   - Extent tree & Directory iterator  |
                      +-------------------+-------------------+
                                          |
                      +-------------------+-------------------+
                      |       LwExt4BlockDevice & Cache       |
                      |   - 2048-entry Read-Ahead Block Cache |
                      |   - Aligned 1MB Sector Translation    |
                      +-------------------+-------------------+
                                          |
                        +-----------------+-----------------+
                        |                                   |
              [Raw Disk / Image]                     [VhdxReader]
          - \\.\PhysicalDriveN                   - VHDX Header v1/v2
          - Raw ext4 .img files                  - BAT Table (Dynamic/Fixed)
          - MBR / GPT Partitions                 - Shared live container access
```

---

## CLI Usage (`ext4_cli.exe`)

### 1. Filesystem Statistics (`stat-fs`)
```cmd
ext4_cli stat-fs --image "C:\disks\ext4.vhdx"
```
Output:
```
Volume Name  : docker-desktop-data
Total Size   : 141.00 MB (147881984 bytes)
Free Space   : 66.85 MB (66857984 bytes)
Used Space   : 74.15 MB (81024000 bytes)
Block Size   : 1024 bytes
Inodes       : 35483 free / 36288 total
```

### 2. Directory Listing (`ls`)
```cmd
ext4_cli ls / --image "C:\disks\ext4.vhdx"
```

### 3. File Streaming (`cat`)
```cmd
ext4_cli cat /etc/os-release --image "C:\disks\ext4.vhdx"
```

### 4. Recursive Tree Scan (`tree-scan`)
```cmd
ext4_cli tree-scan / --image "C:\disks\ext4.vhdx" --max-depth 2 --format jsonl
```

### 5. Physical Drive Inspection
```cmd
ext4_cli ls / --drive 1 --partition 1
```

---

## C-ABI FFI Reference

The shared library (`ext4_bridge.dll`) exports thread-safe C-compatible entry points:

```c
// Open filesystem from disk image or VHDX
void* ext4_open(const char* disk_path, uint32_t part_index, bool write_mode);

// Close and release filesystem resources
void ext4_close(void* bridge_ptr);

// Read file into caller buffer with capacity verification
bool ext4_read_h(void* bridge_ptr, const char* path, uint8_t* out_buf, size_t* out_len);

// Write buffer to file
bool ext4_write_h(void* bridge_ptr, const char* path, const uint8_t* in_buf, size_t in_len);

// High-speed tree scanner with direct callback streaming
typedef int32_t (*Ext4ScanCallback)(
    const char* path,
    uint64_t size_bytes,
    bool is_dir,
    uint64_t mtime,
    uint32_t inode,
    void* user_data
);

int32_t ext4_tree_scan(
    void* bridge_ptr,
    const char* root_path,
    uint32_t max_depth,
    Ext4ScanCallback cb,
    void* user_data
);

// Stream directory scan into JSONL buffer
int32_t ext4_scan_jsonl(
    void* bridge_ptr,
    const char* root_path,
    uint32_t max_depth,
    char* out_buf,
    size_t out_cap
);

// Query volume geometry and space statistics
bool ext4_get_fs_stats(
    void* bridge_ptr,
    uint64_t* total_bytes,
    uint64_t* free_bytes,
    uint32_t* block_size
);
```

---

## Benchmark Results

Evaluated on real WSL2 ext4 VHDX (`ext4.vhdx`, 141MB capacity, 36k inodes):

| Operation | Trials | Mean Latency | Latency Variance ($\sigma^2$) |
| :--- | :---: | :---: | :---: |
| **VHDX Virtual Read Superblock** | 16 | 12.4 µs | 1.8 µs² |
| **Tree Scan (Depth 0 - Root)** | 16 | **165.19 µs (0.165 ms)** | **502.70 µs²** |
| **Full Subtree BFS Scan** | 16 | 2.14 ms | 0.08 ms² |

---

## Building from Source

```powershell
# Run unit and integration test suite
cargo test -- --test-threads=1

# Compile optimized release binaries (produces ext4_cli.exe and ext4_bridge.dll)
cargo build --release
```

---

## License

Dual-licensed under either of:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
