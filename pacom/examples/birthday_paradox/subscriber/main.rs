use pacom::{PacomRuntime, RuntimeConfig};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("[Birthday Paradox Subscriber] Starting application...");
    
    unsafe {
        std::env::set_var("UP_UE_ID", "21762"); // 0x5502
        if std::env::var("PACOM_MANIFEST_PATH").is_err() {
            std::env::set_var("PACOM_MANIFEST_PATH", "examples/birthday_paradox/subscriber/manifest.json");
        }
    }

    let config = RuntimeConfig {
        authority: Some("ecu-birthday-sub".to_string()),
        manifest_path: None,
        mqtt_config: None,
    };

    let runtime = PacomRuntime::new(config).await?;
    println!("[Birthday Paradox Subscriber] PacomRuntime started successfully!");

    let received_count = Arc::new(AtomicUsize::new(0));

    // Subscribe to all 400 topics
    for i in 0..400 {
        let topic_name = format!("/topic/sensor_{}", i);
        let counter = received_count.clone();
        
        runtime.subscribe_event(&topic_name, move |_payload| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::Relaxed);
            }
        }).await?;
    }

    println!("[Birthday Paradox Subscriber] Subscribed to 400 topics. Waiting for messages...");

    // Wait and report
    let mut last_count = 0;
    for iter in 1..=10 {
        sleep(Duration::from_secs(1)).await;
        let current = received_count.load(Ordering::Relaxed);
        println!("[Birthday Paradox Subscriber] Iteration {}: Received {} messages so far (+{}).", iter, current, current - last_count);
        last_count = current;
    }

    println!("[Birthday Paradox Subscriber] Final count: {} messages received.", received_count.load(Ordering::Relaxed));
    println!("[Birthday Paradox Subscriber] Exiting smoothly.");
    Ok(())
}
