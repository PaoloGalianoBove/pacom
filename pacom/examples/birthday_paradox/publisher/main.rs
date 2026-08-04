use pacom::{PacomRuntime, RuntimeConfig};
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("[Birthday Paradox] Starting application...");
    println!("[Birthday Paradox] This app provides 400 topics, which statistically guarantees at least one 15-bit FNV1a hash collision.");
    
    // Set environment variables for UE ID and Manifest Path
    unsafe {
        std::env::set_var("UP_UE_ID", "21761"); // 0x5501
        if std::env::var("PACOM_MANIFEST_PATH").is_err() {
            std::env::set_var("PACOM_MANIFEST_PATH", "examples/birthday_paradox/publisher/manifest.json");
        }
    }

    // Configurazione runtime
    let config = RuntimeConfig {
        authority: Some("ecu-birthday".to_string()),
        manifest_path: None,
        mqtt_config: None,
    };

    // Inizializza il runtime
    let runtime = PacomRuntime::new(config).await?;

    println!("[Birthday Paradox] PacomRuntime started successfully!");
    println!("[Birthday Paradox] Waiting 3 seconds to allow Subscriber to discover our service...");
    sleep(Duration::from_secs(3)).await;

    println!("[Birthday Paradox] If you see this message, the PACOM-SD Gossip Protocol successfully resolved all ID collisions dynamically!");

    // Keep the main thread alive
    // We loop 10 times to ensure vsomeip has enough time to establish routing.
    // Processing 400 multicast Discovery messages and 400 Subscribe requests takes a few seconds.
    // The first iterations will trigger the Discovery announcements and might drop payloads.
    // The subsequent iterations will successfully transmit data once routes are open.
    for iter in 1..=10 {
        sleep(Duration::from_secs(1)).await;
        for i in 0..400 {
            let topic_name = format!("/topic/sensor_{}", i);
            let payload = format!("Message {} for {}", iter, topic_name).into_bytes();
            let _ = runtime.publish_event(&topic_name, payload).await;
        }
    }

    println!("[Birthday Paradox] Finished publishing 4000 messages total.");
    println!("[Birthday Paradox] Exiting smoothly.");
    Ok(())
}
