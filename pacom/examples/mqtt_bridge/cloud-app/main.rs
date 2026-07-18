use pacom::{MqttConfig, PacomRuntime, RuntimeConfig};
use tokio::io::{self, AsyncBufReadExt, BufReader};

const CLOUD_AUTHORITY: &str = "cloud.bridge";
const TOPIC_CLOUD_COMMAND: &str = "/cloud/command";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_path = std::env::var("PACOM_MANIFEST_PATH").unwrap_or_else(|_| {
        format!(
            "{}/examples/mqtt_bridge/sender/manifest.json",
            env!("CARGO_MANIFEST_DIR")
        )
    });

    let broker_uri = std::env::var("PACOM_MQTT_BROKER_URI")
        .unwrap_or_else(|_| "mqtt://127.0.0.1:1883".to_string());

    let runtime = PacomRuntime::new(RuntimeConfig {
        mqtt_config: Some(MqttConfig {
            broker_uri,
            client_id: "pacom-mqtt-sender".to_string(),
        }),
        manifest_path: Some(manifest_path),
    })
    .await?;

    println!("[SENDER] interactive MQTT command sender");
    println!("[SENDER] target: {}/{}", CLOUD_AUTHORITY, TOPIC_CLOUD_COMMAND);
    println!("[SENDER] type text + Enter, '/quit' to stop");

    let stdin = BufReader::new(io::stdin());
    let mut lines = stdin.lines();

    while let Some(line) = lines.next_line().await? {
        let text = line.trim();
        if text.eq_ignore_ascii_case("/quit") {
            break;
        }
        if text.is_empty() {
            continue;
        }

        runtime
            .publish_event_to(
                TOPIC_CLOUD_COMMAND,
                "ecu-hub", // Send TO the vehicle hub
                text.as_bytes().to_vec(),
            )
            .await?;
        println!("[SENDER] sent command: {}", text);
    }

    Ok(())
}
