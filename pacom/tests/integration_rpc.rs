use std::process::Command;
use std::time::Duration;
use tokio::time::sleep;

/// Integration test that spans a full RPC roundtrip.
/// Requires vSomeIP routing manager to be active, or will fail/timeout.
/// Ignored by default in CI, run with `cargo test -- --ignored`.
#[tokio::test]
#[ignore]
async fn test_rpc_echo_roundtrip() {
    // 1. Start Server as a separate OS process
    let mut server_process = Command::new("cargo")
        .args(["run", "--example", "rtt_server"])
        .env("UP_AUTHORITY", "test-ecu")
        .env("UP_UE_ID", "0x5555")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("Failed to start server process");

    // Allow server to start and register to vSomeIP
    sleep(Duration::from_secs(3)).await;

    // 2. Start Client as a separate OS process
    // We override NUM_REQUESTS to 10 for a fast test if the client supported it,
    // but the example is hardcoded. It will run 10k iterations very fast locally.
    let client_output = Command::new("cargo")
        .args(["run", "--example", "rtt_client"])
        .env("UP_AUTHORITY", "test-ecu")
        .env("UP_UE_ID", "0x6666")
        .output()
        .expect("Failed to run client process");

    // 3. Cleanup
    let _ = server_process.kill();
    let _ = server_process.wait();

    let stdout = String::from_utf8_lossy(&client_output.stdout);
    let stderr = String::from_utf8_lossy(&client_output.stderr);

    // Ensure the client executed successfully and completed the benchmark
    assert!(
        client_output.status.success(),
        "Client failed! Stdout:\n{}\nStderr:\n{}",
        stdout,
        stderr
    );
    assert!(
        stdout.contains("RTT measurements written to rtt_measurements.csv")
            || stdout.contains("RTT="),
        "Benchmark did not complete successfully"
    );
}
