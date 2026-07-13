use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Instant, Duration};
use std::fs::File;
use std::io::Write;
use std::thread;
use sysinfo::{System, ProcessesToUpdate};
use pacom::{PlatformClient, SdkConfig};

// ── Identical constants to light-switch-client ──────────────────────────────
const NUM_REQUESTS: usize = 10_000;
const WARMUP_REQUESTS: usize = 10;
const RTT_OUTPUT_FILE: &str = "rtt_measurements.csv";

// ── Identical resource-snapshot types to light-switch-client ────────────────
#[derive(Debug, Clone, Copy, Default)]
struct ResourceSnapshot {
    proc_ram_mb: f64,
    proc_vsz_mb: f64,
    proc_cpu_pct: f32,
    sys_ram_pct: f64,
    sys_cpu_pct: f32,
}

struct AtomicResourceSnapshot {
    proc_ram_mb: AtomicU64,
    proc_vsz_mb: AtomicU64,
    proc_cpu_pct: AtomicU32,
    sys_ram_pct: AtomicU64,
    sys_cpu_pct: AtomicU32,
}

impl Default for AtomicResourceSnapshot {
    fn default() -> Self {
        Self {
            proc_ram_mb: AtomicU64::new(0f64.to_bits()),
            proc_vsz_mb: AtomicU64::new(0f64.to_bits()),
            proc_cpu_pct: AtomicU32::new(0f32.to_bits()),
            sys_ram_pct: AtomicU64::new(0f64.to_bits()),
            sys_cpu_pct: AtomicU32::new(0f32.to_bits()),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ── Background resource-monitoring thread (identical to light-switch) ───
    let resource_snapshot = Arc::new(AtomicResourceSnapshot::default());
    let snapshot_clone = resource_snapshot.clone();

    thread::spawn(move || {
        let mut sys = System::new();
        let pid = sysinfo::get_current_pid().unwrap();

        // Query global memory once at startup for baseline calculation
        sys.refresh_memory();
        let total_mem = sys.total_memory() as f64;
        let sys_ram_pct = if total_mem > 0.0 {
            (sys.used_memory() as f64 / total_mem) * 100.0
        } else {
            0.0
        };

        loop {
            // ONLY refresh the process itself to completely avoid system file I/O blocking
            sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), false);

            let mut proc_ram_mb = 0.0;
            let mut proc_vsz_mb = 0.0;
            let mut proc_cpu_pct = 0.0f32;

            if let Some(proc) = sys.process(pid) {
                proc_ram_mb = proc.memory() as f64 / (1024.0 * 1024.0);
                proc_vsz_mb = proc.virtual_memory() as f64 / (1024.0 * 1024.0);
                proc_cpu_pct = proc.cpu_usage();
            }

            // Lock-free store with relaxed ordering (zero block/contention with main thread!)
            snapshot_clone.proc_ram_mb.store(proc_ram_mb.to_bits(), Ordering::Relaxed);
            snapshot_clone.proc_vsz_mb.store(proc_vsz_mb.to_bits(), Ordering::Relaxed);
            snapshot_clone.proc_cpu_pct.store(proc_cpu_pct.to_bits(), Ordering::Relaxed);
            snapshot_clone.sys_ram_pct.store(sys_ram_pct.to_bits(), Ordering::Relaxed);
            snapshot_clone.sys_cpu_pct.store(0.0f32.to_bits(), Ordering::Relaxed);

            thread::sleep(Duration::from_millis(200));
        }
    });

    println!("Starting pacom RPC Client (benchmark: {} iterations)...", NUM_REQUESTS);
    let client = PlatformClient::new(SdkConfig { mqtt_config: None }).await?;
    println!("[CLIENT] PlatformClient created.");

    // ── Warm-up (identical iteration count to light-switch) ─────────────────
    println!("[CLIENT] Warming up ({} requests)...", WARMUP_REQUESTS);
    for _ in 0..WARMUP_REQUESTS {
        let _ = client.call_rpc("light-switch", b"warmup".to_vec()).await;
    }
    println!("[CLIENT] Warm-up complete. Starting benchmark...");

    // ── Measurement loop (identical structure to light-switch) ───────────────
    let mut rtt_measurements: Vec<(usize, f64, String, ResourceSnapshot)> = Vec::with_capacity(NUM_REQUESTS);

    for i in 0..NUM_REQUESTS {
        // Lock-free load with relaxed ordering (never blocks the hot loop!)
        let snapshot = ResourceSnapshot {
            proc_ram_mb: f64::from_bits(resource_snapshot.proc_ram_mb.load(Ordering::Relaxed)),
            proc_vsz_mb: f64::from_bits(resource_snapshot.proc_vsz_mb.load(Ordering::Relaxed)),
            proc_cpu_pct: f32::from_bits(resource_snapshot.proc_cpu_pct.load(Ordering::Relaxed)),
            sys_ram_pct: f64::from_bits(resource_snapshot.sys_ram_pct.load(Ordering::Relaxed)),
            sys_cpu_pct: f32::from_bits(resource_snapshot.sys_cpu_pct.load(Ordering::Relaxed)),
        };

        // Cycle through payloads to avoid caching artifacts (like light-switch cycles LightStatus)
        let msg = format!("ping-{}", i % 4);
        let req_bytes = msg.into_bytes();

        // Measure RTT
        let start = Instant::now();
        let invoke_result = client.call_rpc("light-switch", req_bytes).await;
        let rtt_ms = start.elapsed().as_secs_f64() * 1000.0;

        match invoke_result {
            Ok(_) => {
                if i % 1000 == 0 {
                    println!(
                        "[CLIENT] iteration {}: RTT={:.3}ms, CPU={:.1}%",
                        i, rtt_ms, snapshot.proc_cpu_pct
                    );
                }
                rtt_measurements.push((i, rtt_ms, "ok".to_string(), snapshot));
            }
            Err(e) => {
                eprintln!("[CLIENT] iteration {}: invoke error: {}", i, e);
                rtt_measurements.push((i, rtt_ms, format!("invoke_error: {}", e), snapshot));
            }
        }
    }

    // ── Write CSV (identical format to light-switch) ─────────────────────────
    write_rtt_file(RTT_OUTPUT_FILE, &rtt_measurements)?;

    Ok(())
}

/// Writes RTT measurements to CSV with identical schema to light-switch-client.
fn write_rtt_file(
    filename: &str,
    measurements: &[(usize, f64, String, ResourceSnapshot)],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = File::create(filename)?;

    // Identical header to light-switch rtt_measurements.csv
    writeln!(file, "iteration,rtt_ms,status,proc_ram_mb,proc_vsz_mb,proc_cpu_pct,sys_ram_pct,sys_cpu_pct")?;

    for (iter, rtt_ms, status, snapshot) in measurements {
        writeln!(
            file,
            "{},{:.3},{},{:.3},{:.3},{:.1},{:.1},{:.1}",
            iter,
            rtt_ms,
            status,
            snapshot.proc_ram_mb,
            snapshot.proc_vsz_mb,
            snapshot.proc_cpu_pct,
            snapshot.sys_ram_pct,
            snapshot.sys_cpu_pct
        )?;
    }

    println!("[CLIENT] RTT measurements written to {}", filename);
    Ok(())
}
