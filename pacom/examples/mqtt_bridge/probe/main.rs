use pacom::{MqttConfig, PacomRuntime, RuntimeConfig};

const CLOUD_AUTHORITY: &str = "cloud.bridge";
const TOPIC_CLOUD_UPSTREAM: &str = "/cloud/upstream";
const TOPIC_CLOUD_COMMAND: &str = "/cloud/command";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_path = std::env::var("PACOM_MANIFEST_PATH").unwrap_or_else(|_| {
        format!(
            "{}/examples/mqtt_bridge/probe/manifest.json",
            env!("CARGO_MANIFEST_DIR")
        )
    });

    let broker_uri = std::env::var("PACOM_MQTT_BROKER_URI")
        .unwrap_or_else(|_| "mqtt://127.0.0.1:1883".to_string());

    unsafe {
        std::env::set_var("PACOM_CLOUD_UE_ID", "0x2200");
    }

    let runtime = PacomRuntime::new(RuntimeConfig {
        mqtt_config: Some(MqttConfig {
            broker_uri,
            client_id: "pacom-mqtt-probe".to_string(),
        }),
        manifest_path: Some(manifest_path),
    })
    .await?;

    runtime
        .subscribe_event_from(TOPIC_CLOUD_UPSTREAM, "ecu-hub", |payload| {
            let text = String::from_utf8_lossy(&payload);
            println!("[PROBE:UPSTREAM] {}", text);
        })
        .await?;

    runtime
        .subscribe_event_from(TOPIC_CLOUD_COMMAND, "ecu-hub", |payload| {
            let text = String::from_utf8_lossy(&payload);
            println!("[PROBE:COMMAND] {}", text);
        })
        .await?;

    println!("[PROBE] listening on MQTT URIs:");
    println!("[PROBE] upstream from ecu-hub");
    println!("[PROBE] command  from ecu-hub");

    std::thread::park();
    Ok(())
}
