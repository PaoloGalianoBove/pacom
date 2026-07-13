use pacom::{PlatformClient, SdkConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting pacom RPC Server...");

    let client = PlatformClient::new(SdkConfig { mqtt_config: None }).await?;
    
    // Register asynchronous RPC handler directly on PlatformClient using the logical service name
    client.register_rpc_method("light-switch", |request_bytes| async move {
        let msg = String::from_utf8_lossy(&request_bytes).into_owned();
        println!("[SERVER] Received RPC request: {}", msg);
        format!("Echo: {}", msg).into_bytes()
    }).await?;
    
    println!("[SERVER] Endpoint registered for 'light-switch'. Listening...");

    // Keep alive
    std::thread::park();
    Ok(())
}
