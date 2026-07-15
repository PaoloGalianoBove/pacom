mod app_client;
mod performance;

use app_client::RttClientApp;
use performance::{PerformanceSampler, write_rtt_file};

const NUM_REQUESTS: usize = 10_000;
const WARMUP_REQUESTS: usize = 10;
const RTT_OUTPUT_FILE: &str = "rtt_measurements.csv";
const RPC_METHOD: &str = "/rpc/rtt/echo";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_path = std::env::var("PACOM_MANIFEST_PATH").unwrap_or_else(|_| {
        format!(
            "{}/examples/rtt/client/manifest.json",
            env!("CARGO_MANIFEST_DIR")
        )
    });

    println!("Starting pacom RPC Client (benchmark: {} iterations)...", NUM_REQUESTS);
    let client = RttClientApp::new(manifest_path).await?;
    let sampler = PerformanceSampler::start();

    println!("[CLIENT] Warming up ({} requests)...", WARMUP_REQUESTS);
    client.warm_up(RPC_METHOD, WARMUP_REQUESTS).await;
    println!("[CLIENT] Warm-up complete. Starting benchmark...");

    let mut rtt_measurements = Vec::with_capacity(NUM_REQUESTS);

    for i in 0..NUM_REQUESTS {
        let snapshot = sampler.snapshot();
        let msg = format!("ping-{}", i % 4);
        let (rtt_ms, invoke_result) = client.invoke_timed(RPC_METHOD, msg.into_bytes()).await;

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

    write_rtt_file(RTT_OUTPUT_FILE, &rtt_measurements)?;

    Ok(())
}
