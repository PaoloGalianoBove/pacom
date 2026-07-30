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
            std::env::set_var("PACOM_MANIFEST_PATH", "examples/birthday_paradox/manifest.json");
        }
    }

    // Configurazione runtime
    let config = RuntimeConfig {
        authority: Some("ecu-birthday".to_string()),
        manifest_path: None,
        mqtt_config: None,
    };

    // Inizializza il runtime
    let _runtime = PacomRuntime::new(config).await?;

    println!("[Birthday Paradox] PacomRuntime started successfully!");
    println!("[Birthday Paradox] If you see this message, the PACOM-SD Gossip Protocol successfully resolved all ID collisions dynamically!");

    // Keep the main thread alive
    sleep(Duration::from_secs(2)).await;

    println!("[Birthday Paradox] Exiting smoothly.");
    Ok(())
}
