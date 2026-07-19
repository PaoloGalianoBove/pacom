use pacom::{PacomRuntime, RuntimeConfig};

const RPC_METHOD: &str = "/rpc/rtt/echo";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting pacom RPC Server...");

    let manifest_path = std::env::var("PACOM_MANIFEST_PATH").unwrap_or_else(|_| {
        format!(
            "{}/examples/rtt/server/manifest.json",
            env!("CARGO_MANIFEST_DIR")
        )
    });

    let client = PacomRuntime::new(RuntimeConfig {
        mqtt_config: None,
        manifest_path: Some(manifest_path),
    })
    .await?;

    // Register asynchronous RPC handler directly on the runtime using the logical service name
    client
        .register_rpc_method(RPC_METHOD, |request_bytes| async move {
            let msg = String::from_utf8_lossy(&request_bytes).into_owned();
            println!("[SERVER] Received RPC request: {}", msg);
            format!("Echo: {}", msg).into_bytes()
        })
        .await?;

    println!(
        "[SERVER] Endpoint registered for '{}'. Listening...",
        RPC_METHOD
    );

    // Keep alive
    std::thread::park();
    Ok(())
}
