// [EN] Architecture: CLI executable for Windows ext4 inspection, modification, VHDX mounting, and tree scanning.
// [KO] 아키텍처: Windows 환경에서 PhysicalDrive, raw 디스크 이미지 및 VHDX를 조작·검사하고 트리 스캔을 수행하는 CLI 진입점.
// Why other approaches were rejected: Requiring WSL2 mount commands or GUI explorer drivers fails in headless automation; standalone CLI provides instant zero-dependency scriptability.

use clap::{Parser, Subcommand};
use ext4_bridge::ext4::Ext4Bridge;
use ext4_lwext4::types::OpenFlags;
use std::io::Read;
use std::path::PathBuf;

const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x00000800;

#[derive(Parser)]
#[command(
    name = "ext4_cli",
    author = "ext4-bridge-windows developers",
    version = "0.2.0",
    about = "High-performance ext4 bridge for Windows: VHDX (dynamic/fixed), raw disks, images, and tree scanner"
)]
struct CliArgs {
    #[command(subcommand)]
    command: Cli,
}

#[derive(Subcommand)]
enum Cli {
    #[command(about = "List directory contents in ext4 filesystem")]
    Ls {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image or .vhdx file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based, 0 for whole disk)")]
        partition: u32,
        #[arg(help = "Absolute path in the ext4 filesystem")]
        path: String,
    },
    #[command(about = "Print file content to stdout (text or hex dump)")]
    Cat {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image or .vhdx file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based, 0 for whole disk)")]
        partition: u32,
        #[arg(long, help = "Hex dump mode")]
        hex: bool,
        #[arg(help = "Absolute path in the ext4 filesystem")]
        path: String,
    },
    #[command(about = "Write text content directly to a file")]
    WriteText {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image or .vhdx file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based, 0 for whole disk)")]
        partition: u32,
        #[arg(help = "Absolute path in the ext4 filesystem")]
        path: String,
        #[arg(help = "Text content to write")]
        data: String,
    },
    #[command(about = "Write binary data to a file from stdin or local file")]
    WriteBin {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image or .vhdx file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based, 0 for whole disk)")]
        partition: u32,
        #[arg(help = "Absolute path in the ext4 filesystem")]
        path: String,
        #[arg(short, long, help = "Path to local file (reads stdin if omitted)")]
        from_file: Option<PathBuf>,
    },
    #[command(about = "Copy a file from ext4 to local Windows filesystem")]
    Cp {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image or .vhdx file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based, 0 for whole disk)")]
        partition: u32,
        #[arg(help = "Source file path inside ext4")]
        src: String,
        #[arg(help = "Destination path on the local filesystem")]
        dest: PathBuf,
    },
    #[command(about = "Compare ext4 file content with a local Windows file")]
    Diff {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image or .vhdx file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based, 0 for whole disk)")]
        partition: u32,
        #[arg(help = "Source file path inside ext4")]
        ext4_path: String,
        #[arg(help = "Local file path to compare with")]
        local_path: PathBuf,
    },
    #[command(about = "Display inode metadata and size for a file or directory")]
    Stat {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image or .vhdx file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based, 0 for whole disk)")]
        partition: u32,
        #[arg(help = "Absolute path in the ext4 filesystem")]
        path: String,
    },
    #[command(about = "Remove a file or directory from ext4 filesystem")]
    Rm {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image or .vhdx file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based, 0 for whole disk)")]
        partition: u32,
        #[arg(help = "Absolute path of file or directory in ext4")]
        path: String,
    },
    #[command(about = "Move or rename a file or directory inside ext4")]
    Mv {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image or .vhdx file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based, 0 for whole disk)")]
        partition: u32,
        #[arg(help = "Source path inside ext4")]
        src: String,
        #[arg(help = "Destination path inside ext4")]
        dest: String,
    },
    #[command(about = "Create a directory inside ext4 filesystem")]
    Mkdir {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image or .vhdx file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based, 0 for whole disk)")]
        partition: u32,
        #[arg(help = "Absolute directory path to create in ext4")]
        path: String,
    },
    #[command(about = "Create a symbolic link inside ext4 filesystem")]
    Ln {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image or .vhdx file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based, 0 for whole disk)")]
        partition: u32,
        #[arg(help = "Target that the symlink points to")]
        target: String,
        #[arg(help = "Absolute symlink path to create")]
        linkpath: String,
    },
    #[command(about = "Change file or directory permission mode")]
    Chmod {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image or .vhdx file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based, 0 for whole disk)")]
        partition: u32,
        #[arg(help = "Absolute path in the ext4 filesystem")]
        path: String,
        #[arg(help = "Octal permission mode (e.g., 0o644, 0o755)")]
        mode: String,
    },
    #[command(about = "High-speed recursive metadata/tree scanner for SniffEverything.exe")]
    TreeScan {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image or .vhdx file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based, 0 for whole disk)")]
        partition: u32,
        #[arg(default_value = "/", help = "Root path in ext4 to begin traversal")]
        path: String,
        #[arg(long, help = "Emit newline-delimited JSON (JSONL) instead of text")]
        json: bool,
        #[arg(long, help = "Maximum traversal depth (default: unlimited)")]
        max_depth: Option<u32>,
        #[arg(long, help = "Explicit timeout in milliseconds (default: unlimited)")]
        timeout_ms: Option<u64>,
        #[arg(long, help = "Explicit maximum entries to scan (default: unlimited)")]
        max_entries: Option<usize>,
        #[arg(long, help = "Emit per-directory and per-inode debug trace to stderr")]
        debug: bool,
    },
    #[command(about = "Display filesystem statistics (capacity, free space, block size, inodes)")]
    StatFs {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image or .vhdx file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based, 0 for whole disk)")]
        partition: u32,
        #[arg(long, help = "Emit JSON output")]
        json: bool,
    },
}

fn get_disk_path(drive: Option<u32>, image: Option<String>) -> Result<String, String> {
    match (drive, image) {
        (Some(d), None) => Ok(format!("\\\\.\\PhysicalDrive{}", d)),
        (None, Some(img)) => Ok(img),
        _ => Err("You must specify either a drive number (-d) or an image path (-i).".into()),
    }
}

struct BridgeContext {
    drive: Option<u32>,
    image: Option<String>,
    partition: u32,
}

impl BridgeContext {
    fn new(drive: Option<u32>, image: Option<String>, partition: u32) -> Self {
        Self { drive, image, partition }
    }

    fn execute<F>(&self, write_mode: bool, f: F)
    where
        F: FnOnce(&Ext4Bridge) -> Result<(), String>,
    {
        let disk_path = match get_disk_path(self.drive, self.image.clone()) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[FAIL] {}", e);
                std::process::exit(1);
            }
        };
        match Ext4Bridge::new(&disk_path, self.partition, write_mode) {
            Ok(bridge) => {
                if let Err(e) = f(&bridge) {
                    eprintln!("[FAIL] {}", e);
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("[FAIL] Initialization error: {:?}", e);
                std::process::exit(1);
            }
        }
    }
}

fn main() {
    // Pillar 10: Enforce OS DLL Search-Order Lockdown (Mitigate DLL side-loading)
    unsafe {
        windows_sys::Win32::System::LibraryLoader::SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32);
    }

    let args = CliArgs::parse();

    match args.command {
        Cli::Ls { drive, image, partition, path } => {
            let ctx = BridgeContext::new(drive, image, partition);
            ctx.execute(false, |bridge| {
                let mut dir = bridge.fs.open_dir(&path).map_err(|e| format!("Open directory error: {:?}", e))?;
                println!("[SUCCESS] List directory '{}':", path);
                while let Some(Ok(entry)) = dir.next_entry() {
                    println!("  - Inode {:8} | Type {:?} | {}", entry.inode(), entry.file_type(), entry.name());
                }
                Ok(())
            });
        }
        Cli::Cat { drive, image, partition, hex, path } => {
            let ctx = BridgeContext::new(drive, image, partition);
            ctx.execute(false, |bridge| {
                let mut file = bridge.fs.open(&path, OpenFlags::READ).map_err(|e| format!("Open file error: {:?}", e))?;
                let mut data = Vec::new();
                file.read_to_end(&mut data).map_err(|e| format!("Read data error: {:?}", e))?;
                if hex {
                    println!("[SUCCESS] Hex dump of '{}':", path);
                    for (i, byte) in data.iter().enumerate() {
                        print!("{:02X} ", byte);
                        if (i + 1) % 16 == 0 {
                            println!();
                        }
                    }
                    if data.len() % 16 != 0 {
                        println!();
                    }
                } else {
                    let content = String::from_utf8_lossy(&data);
                    println!("[SUCCESS] Read file '{}':\n{}", path, content);
                }
                Ok(())
            });
        }
        Cli::WriteText { drive, image, partition, path, data } => {
            let ctx = BridgeContext::new(drive, image, partition);
            ctx.execute(true, |bridge| {
                let mut file = bridge.fs.open(&path, OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE)
                    .map_err(|e| format!("Open file error: {:?}", e))?;
                file.write_all(data.as_bytes()).map_err(|e| format!("Write error: {:?}", e))?;
                println!("[SUCCESS] Text written to '{}'!", path);
                Ok(())
            });
        }
        Cli::WriteBin { drive, image, partition, path, from_file } => {
            let mut data = Vec::new();
            let read_res = if let Some(file_path) = from_file {
                std::fs::read(&file_path).map_err(|e| format!("Failed to read local file: {}", e))
            } else {
                std::io::stdin().read_to_end(&mut data).map_err(|e| format!("Failed to read from STDIN: {}", e)).map(|_| data)
            };
            let data = match read_res {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("[FAIL] {}", e);
                    std::process::exit(1);
                }
            };
            let ctx = BridgeContext::new(drive, image, partition);
            ctx.execute(true, |bridge| {
                let mut file = bridge.fs.open(&path, OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE)
                    .map_err(|e| format!("Open file error: {:?}", e))?;
                file.write_all(&data).map_err(|e| format!("Write error: {:?}", e))?;
                println!("[SUCCESS] {} bytes of binary data written to '{}'!", data.len(), path);
                Ok(())
            });
        }
        Cli::Cp { drive, image, partition, src, dest } => {
            let ctx = BridgeContext::new(drive, image, partition);
            ctx.execute(false, |bridge| {
                let mut file = bridge.fs.open(&src, OpenFlags::READ).map_err(|e| format!("Open file error: {:?}", e))?;
                let mut data = Vec::new();
                file.read_to_end(&mut data).map_err(|e| format!("Read data error: {:?}", e))?;
                std::fs::write(&dest, &data).map_err(|e| format!("Failed to write local file: {}", e))?;
                println!("[SUCCESS] Extracted '{}' to '{}' ({} bytes)", src, dest.display(), data.len());
                Ok(())
            });
        }
        Cli::Diff { drive, image, partition, ext4_path, local_path } => {
            let local_data = match std::fs::read(&local_path) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("[FAIL] Failed to read local file: {}", e);
                    std::process::exit(1);
                }
            };
            let ctx = BridgeContext::new(drive, image, partition);
            ctx.execute(false, |bridge| {
                let mut file = bridge.fs.open(&ext4_path, OpenFlags::READ).map_err(|e| format!("Open file error: {:?}", e))?;
                let mut ext4_data = Vec::new();
                file.read_to_end(&mut ext4_data).map_err(|e| format!("Read data error: {:?}", e))?;
                if local_data == ext4_data {
                    println!("[SUCCESS] Files match exactly ({} bytes).", ext4_data.len());
                } else {
                    println!("[FAIL] Files differ! (ext4: {} bytes, local: {} bytes)", ext4_data.len(), local_data.len());
                }
                Ok(())
            });
        }
        Cli::Stat { drive, image, partition, path } => {
            let ctx = BridgeContext::new(drive, image, partition);
            ctx.execute(false, |bridge| {
                let meta = bridge.fs.metadata(&path).map_err(|e| format!("Metadata error: {:?}", e))?;
                println!("[SUCCESS] Stat for '{}':", path);
                println!("  Size   : {} bytes", meta.size);
                println!("  Blocks : {}", meta.blocks);
                println!("  Type   : {:?}", meta.file_type);
                println!("  Mode   : {:#o}", meta.mode);
                println!("  UID    : {}", meta.uid);
                println!("  GID    : {}", meta.gid);
                Ok(())
            });
        }
        Cli::Rm { drive, image, partition, path } => {
            let ctx = BridgeContext::new(drive, image, partition);
            ctx.execute(true, |bridge| {
                if bridge.fs.is_dir(&path) {
                    bridge.fs.rmdir(&path).map_err(|e| format!("Rmdir error: {:?}", e))?;
                    println!("[SUCCESS] Removed directory '{}'", path);
                } else {
                    bridge.fs.remove(&path).map_err(|e| format!("Remove error: {:?}", e))?;
                    println!("[SUCCESS] Removed file '{}'", path);
                }
                Ok(())
            });
        }
        Cli::Mv { drive, image, partition, src, dest } => {
            let ctx = BridgeContext::new(drive, image, partition);
            ctx.execute(true, |bridge| {
                bridge.fs.rename(&src, &dest).map_err(|e| format!("Rename error: {:?}", e))?;
                println!("[SUCCESS] Moved/Renamed '{}' to '{}'", src, dest);
                Ok(())
            });
        }
        Cli::Mkdir { drive, image, partition, path } => {
            let ctx = BridgeContext::new(drive, image, partition);
            ctx.execute(true, |bridge| {
                bridge.fs.mkdir(&path, 0o777).map_err(|e| format!("Mkdir error: {:?}", e))?;
                println!("[SUCCESS] Created directory '{}'", path);
                Ok(())
            });
        }
        Cli::Ln { drive, image, partition, target, linkpath } => {
            let ctx = BridgeContext::new(drive, image, partition);
            ctx.execute(true, |bridge| {
                bridge.symlink_path(&target, &linkpath).map_err(|e| format!("Symlink error: {:?}", e))?;
                println!("[SUCCESS] Created symlink '{}' -> '{}'!", linkpath, target);
                Ok(())
            });
        }
        Cli::Chmod { drive, image, partition, path, mode } => {
            let parsed_mode = if mode.starts_with("0o") {
                u32::from_str_radix(&mode[2..], 8).unwrap_or(0o600)
            } else if mode.starts_with("0x") {
                u32::from_str_radix(&mode[2..], 16).unwrap_or(0o600)
            } else {
                u32::from_str_radix(&mode, 8).unwrap_or(u32::from_str_radix(&mode, 10).unwrap_or(0o600))
            };
            let ctx = BridgeContext::new(drive, image, partition);
            ctx.execute(true, |bridge| {
                bridge.chmod_path(&path, parsed_mode).map_err(|e| format!("Chmod error: {:?}", e))?;
                println!("[SUCCESS] Chmod '{}' to mode {:#o}!", path, parsed_mode);
                Ok(())
            });
        }
        Cli::TreeScan { drive, image, partition, path, json, max_depth, timeout_ms, max_entries, debug } => {
            let ctx = BridgeContext::new(drive, image, partition);
            ctx.execute(false, |bridge| {
                let start_time = std::time::Instant::now();
                let mut entries = Vec::new();
                bridge.tree_scan_visit(&path, max_depth, timeout_ms, max_entries, debug, &mut |entry| {
                    entries.push(entry);
                    0
                }).map_err(|e| format!("TreeScan error: {:?}", e))?;
                let elapsed = start_time.elapsed();

                if json {
                    for entry in &entries {
                        if let Ok(line) = serde_json::to_string(entry) {
                            println!("{}", line);
                        }
                    }
                } else {
                    println!("[SUCCESS] Scanned {} entries in {:.2}ms starting at '{}':", entries.len(), elapsed.as_secs_f64() * 1000.0, path);
                    for entry in entries.iter().take(50) {
                        let kind = if entry.is_dir { "DIR " } else { "FILE" };
                        println!("  {} | Inode {:8} | Size {:10} B | {}", kind, entry.inode, entry.size_bytes, entry.path);
                    }
                    if entries.len() > 50 {
                        println!("  ... and {} more entries (use --json for complete output)", entries.len() - 50);
                    }
                }
                Ok(())
            });
        }
        Cli::StatFs { drive, image, partition, json } => {
            let ctx = BridgeContext::new(drive, image, partition);
            ctx.execute(false, |bridge| {
                let stat = bridge.stat_fs().map_err(|e| format!("StatFs error: {:?}", e))?;
                if json {
                    let s = serde_json::to_string_pretty(&stat).unwrap_or_default();
                    println!("{}", s);
                } else {
                    println!("[SUCCESS] Filesystem Statistics:");
                    println!("  Volume Name  : {}", if stat.volume_name.is_empty() { "<unnamed>" } else { &stat.volume_name });
                    println!("  Total Space  : {} bytes ({:.2} GiB)", stat.total_bytes, stat.total_bytes as f64 / (1024.0 * 1024.0 * 1024.0));
                    println!("  Free Space   : {} bytes ({:.2} GiB)", stat.free_bytes, stat.free_bytes as f64 / (1024.0 * 1024.0 * 1024.0));
                    println!("  Used Space   : {} bytes ({:.2} GiB)", stat.used_bytes, stat.used_bytes as f64 / (1024.0 * 1024.0 * 1024.0));
                    println!("  Block Size   : {} bytes", stat.block_size);
                    println!("  Total Inodes : {}", stat.total_inodes);
                    println!("  Free Inodes  : {}", stat.free_inodes);
                }
                Ok(())
            });
        }
    }
}
