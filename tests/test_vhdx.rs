// [EN] Integration tests and benchmarks for VHDX parsing, BAT translation, and tree scanning with variance telemetry.
// [KO] VHDX 파싱, BAT 블록 변환, 트리 스캔 및 16회 반복 분산 통계 벤치마크 통합 테스트.

use ext4_bridge::ext4::Ext4Bridge;
use ext4_bridge::vhdx::VhdxReader;
use std::path::Path;
use std::time::Instant;

#[test]
fn test_vhdx_detector_and_real_docker_inspection() {
    let docker_vhdx_path = r"C:\Program Files\Docker\Docker\resources\wsl\ext4.vhdx";
    if !Path::new(docker_vhdx_path).exists() {
        eprintln!("[SKIP] Docker ext4.vhdx not present on this machine, skipping live VHDX test.");
        return;
    }

    // 1. Signature detection
    assert!(VhdxReader::is_vhdx_file(docker_vhdx_path), "Should detect valid VHDX signature");

    // 2. Open via VhdxReader
    let mut reader = VhdxReader::open(docker_vhdx_path, false).expect("VhdxReader::open failed");
    assert!(reader.virtual_disk_size() > 0, "Virtual disk size must be > 0");
    assert!(reader.bat.len() > 0, "BAT table must contain entries");
    assert!(reader.sector_size() == 512 || reader.sector_size() == 4096);

    // 3. Test raw VHDX virtual read at superblock offset 1024
    let mut sb = [0u8; 1024];
    reader.read_at(1024, &mut sb).expect("Failed to read virtual offset 1024");
    let magic = u16::from_le_bytes([sb[56], sb[57]]);
    assert_eq!(magic, 0xEF53, "Virtual offset 1024 must contain ext4 magic 0xEF53");

    // 4. Open via Ext4Bridge
    let bridge = Ext4Bridge::open_fs(docker_vhdx_path, 0, false).expect("Ext4Bridge::open_fs failed on VHDX");

    // 5. Query filesystem statistics
    let stats = bridge.stat_fs().expect("stat_fs failed on VHDX");
    assert!(stats.total_bytes >= 100 * 1024 * 1024, "Total bytes should be ~140MB");
    assert!(stats.free_bytes > 0);
    assert!(stats.used_bytes > 0);

    // 6. Tree scan depth 0
    let entries = bridge.tree_scan("/", Some(0)).expect("tree_scan failed");
    assert!(entries.len() >= 10, "Root directory must contain at least 10 entries");
    assert!(entries.iter().any(|e| e.path == "/bin"));
    assert!(entries.iter().any(|e| e.path == "/etc"));
    assert!(entries.iter().any(|e| e.path == "/var"));

    // 7. Measure benchmark metrics (mean and variance over 16 runs)
    const TRIALS: usize = 16;
    let mut latencies_us = Vec::with_capacity(TRIALS);

    for _ in 0..TRIALS {
        let t0 = Instant::now();
        let _ = bridge.tree_scan("/", Some(0)).unwrap();
        let elapsed = t0.elapsed().as_micros() as f64;
        latencies_us.push(elapsed);
    }

    let mean_us: f64 = latencies_us.iter().sum::<f64>() / (TRIALS as f64);
    let variance_us2: f64 = latencies_us.iter().map(|&x| (x - mean_us).powi(2)).sum::<f64>() / ((TRIALS - 1) as f64);

    println!(
        "\n=== VHDX Tree-Scan Benchmark (16 Trials) ===\n  Mean Scan Latency : {:.2} µs ({:.3} ms)\n  Latency Variance  : {:.2} µs²\n  Sample Latencies  : {:?} µs\n",
        mean_us,
        mean_us / 1000.0,
        variance_us2,
        latencies_us
    );

    assert!(mean_us < 50000.0, "Mean scan latency must be < 50ms");
}
