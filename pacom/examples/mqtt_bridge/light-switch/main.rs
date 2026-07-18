use std::sync::Arc;

use pacom::{MqttConfig, PacomRuntime, RuntimeConfig};

const TOPIC_UP: &str = "/bridge/up";
const TOPIC_DOWN: &str = "/bridge/down";
const TOPIC_CLOUD_UPSTREAM: &str = "/cloud/upstream";
const TOPIC_CLOUD_COMMAND: &str = "/cloud/command";
const CLOUD_AUTHORITY: &str = "cloud.bridge";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_path = std::env::var("PACOM_MANIFEST_PATH").unwrap_or_else(|_| {
        format!(
            "{}/examples/mqtt_bridge/hub/manifest.json",
            env!("CARGO_MANIFEST_DIR")
        )
    });

    let broker_uri = std::env::var("PACOM_MQTT_BROKER_URI")
        .unwrap_or_else(|_| "mqtt://127.0.0.1:1883".to_string());

    let runtime = Arc::new(
        PacomRuntime::new(RuntimeConfig {
            mqtt_config: Some(MqttConfig {
                broker_uri: broker_uri.clone(),
                client_id: "pacom-hub-runtime".to_string(),
            }),
            manifest_path: Some(manifest_path),
        })
        .await?,
    );

    unsafe {
        std::env::set_var("PACOM_CLOUD_UE_ID", "0x2200");
    }

    let runtime_for_upstream = runtime.clone();
    runtime
        .subscribe_event(TOPIC_UP, move |payload| {
            let runtime = runtime_for_upstream.clone();
            tokio::spawn(async move {
                let text = String::from_utf8_lossy(&payload);
                println!("[HUB] SOME/IP received from edge: {}", text);
                if let Err(e) = runtime
                    .publish_event_to(TOPIC_CLOUD_UPSTREAM, CLOUD_AUTHORITY, payload)
                    .await
                {
                    eprintln!("[HUB] failed to forward SOME/IP -> MQTT: {}", e);
                }
            });
        })
        .await?;

    let runtime_for_cmd = runtime.clone();
    runtime
        .subscribe_event_from(TOPIC_CLOUD_COMMAND, CLOUD_AUTHORITY, move |payload| {
            let runtime = runtime_for_cmd.clone();
            tokio::spawn(async move {
                let text = String::from_utf8_lossy(&payload);
                println!("[HUB] MQTT command received: {}", text);
                if let Err(e) = runtime.publish_event(TOPIC_DOWN, payload).await {
                    eprintln!("[HUB] failed to forward MQTT -> SOME/IP: {}", e);
                }
            });
        })
        .await?;

    runtime
        .publish_event(TOPIC_DOWN, b"hub-online".to_vec())
        .await?;

    println!("[HUB] bridge online");
    println!("[HUB] SOME/IP '{}' -> MQTT '{}/{}'", TOPIC_UP, CLOUD_AUTHORITY, TOPIC_CLOUD_UPSTREAM);
    println!("[HUB] MQTT '{}/{}' -> SOME/IP '{}'", CLOUD_AUTHORITY, TOPIC_CLOUD_COMMAND, TOPIC_DOWN);
    println!("[HUB] waiting...");

    std::thread::park();
    Ok(())
}
