mod app_client;
mod performance;

use app_client::RttClientApp;
use performance::{PerformanceSampler, RttMeasurementWriter};

const NUM_REQUESTS: usize = 10_000;
const WARMUP_REQUESTS: usize = 10;
const DEFAULT_RTT_OUTPUT_FILE: &str = "rtt_measurements.csv";
const RPC_METHOD: &str = "/rpc/rtt/echo";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_path = std::env::var("PACOM_MANIFEST_PATH").unwrap_or_else(|_| {
        format!(
            "{}/examples/rtt/client/manifest.json",
            env!("CARGO_MANIFEST_DIR")
        )
    });

    println!(
        "Starting pacom RPC Client (benchmark: {} iterations)...",
        NUM_REQUESTS
    );
    let client = RttClientApp::new(manifest_path).await?;
    let sampler = PerformanceSampler::start();

    println!("[CLIENT] Warming up ({} requests)...", WARMUP_REQUESTS);
    client.warm_up(RPC_METHOD, WARMUP_REQUESTS).await;
    println!("[CLIENT] Warm-up complete. Starting benchmark...");

    let output_file = std::env::var("PACOM_RTT_OUTPUT_PATH")
        .unwrap_or_else(|_| DEFAULT_RTT_OUTPUT_FILE.to_string());
    let mut measurement_writer = RttMeasurementWriter::create(&output_file)?;

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
                measurement_writer.write(i, rtt_ms, "ok", snapshot)?;
            }
            Err(e) => {
                eprintln!("[CLIENT] iteration {}: invoke error: {}", i, e);
                measurement_writer.write(
                    i,
                    rtt_ms,
                    &format!("invoke_error: {}", e),
                    snapshot,
                )?;
            }
        }
    }

    measurement_writer.finish()?;

    Ok(())
}
