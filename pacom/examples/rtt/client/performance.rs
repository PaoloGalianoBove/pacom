use std::fs::File;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use sysinfo::{ProcessesToUpdate, System};

#[derive(Debug, Clone, Copy, Default)]
pub struct ResourceSnapshot {
    pub proc_ram_mb: f64,
    pub proc_vsz_mb: f64,
    pub proc_cpu_pct: f32,
    pub sys_ram_pct: f64,
    pub sys_cpu_pct: f32,
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

pub struct PerformanceSampler {
    state: Arc<AtomicResourceSnapshot>,
}

impl PerformanceSampler {
    pub fn start() -> Self {
        let state = Arc::new(AtomicResourceSnapshot::default());
        let state_clone = state.clone();

        thread::spawn(move || {
            let mut sys = System::new();
            let pid = sysinfo::get_current_pid().unwrap();

            sys.refresh_memory();
            let total_mem = sys.total_memory() as f64;
            let sys_ram_pct = if total_mem > 0.0 {
                (sys.used_memory() as f64 / total_mem) * 100.0
            } else {
                0.0
            };

            loop {
                sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), false);

                let mut proc_ram_mb = 0.0;
                let mut proc_vsz_mb = 0.0;
                let mut proc_cpu_pct = 0.0f32;

                if let Some(proc) = sys.process(pid) {
                    proc_ram_mb = proc.memory() as f64 / (1024.0 * 1024.0);
                    proc_vsz_mb = proc.virtual_memory() as f64 / (1024.0 * 1024.0);
                    proc_cpu_pct = proc.cpu_usage();
                }

                state_clone
                    .proc_ram_mb
                    .store(proc_ram_mb.to_bits(), Ordering::Relaxed);
                state_clone
                    .proc_vsz_mb
                    .store(proc_vsz_mb.to_bits(), Ordering::Relaxed);
                state_clone
                    .proc_cpu_pct
                    .store(proc_cpu_pct.to_bits(), Ordering::Relaxed);
                state_clone
                    .sys_ram_pct
                    .store(sys_ram_pct.to_bits(), Ordering::Relaxed);
                state_clone
                    .sys_cpu_pct
                    .store(0.0f32.to_bits(), Ordering::Relaxed);

                thread::sleep(Duration::from_millis(200));
            }
        });

        Self { state }
    }

    pub fn snapshot(&self) -> ResourceSnapshot {
        ResourceSnapshot {
            proc_ram_mb: f64::from_bits(self.state.proc_ram_mb.load(Ordering::Relaxed)),
            proc_vsz_mb: f64::from_bits(self.state.proc_vsz_mb.load(Ordering::Relaxed)),
            proc_cpu_pct: f32::from_bits(self.state.proc_cpu_pct.load(Ordering::Relaxed)),
            sys_ram_pct: f64::from_bits(self.state.sys_ram_pct.load(Ordering::Relaxed)),
            sys_cpu_pct: f32::from_bits(self.state.sys_cpu_pct.load(Ordering::Relaxed)),
        }
    }
}

pub fn write_rtt_file(
    filename: &str,
    measurements: &[(usize, f64, String, ResourceSnapshot)],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = File::create(filename)?;

    writeln!(
        file,
        "iteration,rtt_ms,status,proc_ram_mb,proc_vsz_mb,proc_cpu_pct,sys_ram_pct,sys_cpu_pct"
    )?;

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
