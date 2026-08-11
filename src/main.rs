use clap::{Parser, Subcommand};
use ext4_bridge::ext4::Ext4Bridge;
use std::path::PathBuf;
use std::io::Read;
use ext4_lwext4::types::OpenFlags;

#[derive(Parser)]
#[command(author, version, about = "ext4_lwext4-based physical disk and image bridge for Windows")]
struct CliArgs {
    #[command(subcommand)]
    command: Cli,
}

#[derive(Subcommand)]
enum Cli {
    Ls {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based)")]
        partition: u32,
        #[arg(help = "Absolute path in the ext4 filesystem")]
        path: String,
    },
    Cat {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based)")]
        partition: u32,
        #[arg(long, help = "Hex dump mode")]
        hex: bool,
        #[arg(help = "Absolute path in the ext4 filesystem")]
        path: String,
    },
    WriteText {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based)")]
        partition: u32,
        #[arg(help = "Absolute path in the ext4 filesystem")]
        path: String,
        #[arg(help = "Text content to write")]
        data: String,
    },
    WriteBin {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based)")]
        partition: u32,
        #[arg(help = "Absolute path in the ext4 filesystem")]
        path: String,
        #[arg(short, long, help = "Path to the local file to read binary data from (stdin if omitted)")]
        from_file: Option<PathBuf>,
    },
    Cp {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based)")]
        partition: u32,
        #[arg(help = "Source file path inside ext4")]
        src: String,
        #[arg(help = "Destination path on the local filesystem")]
        dest: PathBuf,
    },
    Diff {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based)")]
        partition: u32,
        #[arg(help = "Source file path inside ext4")]
        ext4_path: String,
        #[arg(help = "Local file path to compare with")]
        local_path: PathBuf,
    },
    Stat {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based)")]
        partition: u32,
        #[arg(help = "Absolute path in the ext4 filesystem")]
        path: String,
    },
    Rm {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based)")]
        partition: u32,
        #[arg(help = "Absolute path of file or directory in the ext4 filesystem")]
        path: String,
    },
    Mv {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based)")]
        partition: u32,
        #[arg(help = "Source absolute path inside ext4")]
        src: String,
        #[arg(help = "Destination absolute path inside ext4")]
        dest: String,
    },
    Mkdir {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based)")]
        partition: u32,
        #[arg(help = "Absolute directory path to create in the ext4 filesystem")]
        path: String,
    },
    Ln {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based)")]
        partition: u32,
        #[arg(help = "Target that the symlink points to")]
        target: String,
        #[arg(help = "Absolute symlink path to create in the ext4 filesystem")]
        linkpath: String,
    },
    Chmod {
        #[arg(short, long, help = "Physical drive number (e.g. 1 for \\\\.\\PhysicalDrive1)")]
        drive: Option<u32>,
        #[arg(short, long, help = "Path to the disk image file")]
        image: Option<String>,
        #[arg(short, long, default_value_t = 1, help = "Partition index (1-based)")]
        partition: u32,
        #[arg(help = "Absolute path in the ext4 filesystem")]
        path: String,
        #[arg(help = "Octal permission mode (e.g., 0o600, 0o755)")]
        mode: String,
    }
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
                println!("[FAIL] {}", e);
                return;
            }
        };
        match Ext4Bridge::new(&disk_path, self.partition, write_mode) {
            Ok(bridge) => {
                if let Err(e) = f(&bridge) {
                    println!("[FAIL] {}", e);
                }
            }
            Err(e) => println!("[FAIL] Initialization error: {:?}", e),
        }
    }
}

fn main() {
    // 아키텍처 명세 주석: ext4 브릿지 CLI 명령어 처리에 대한 중복 생명주기 관리 로직을 BridgeContext 구조체로 캡슐화하여 OOP 패턴으로 단일화하고 개별 쉘 스크립트 기반 수정 방식을 완전히 배제하여 코드 안전성을 극대화합니다.
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
                    println!("[FAIL] {}", e);
                    return;
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
                    println!("[FAIL] Failed to read local file: {}", e);
                    return;
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
    }
}
