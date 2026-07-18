use pacom::{MqttConfig, PacomRuntime, RuntimeConfig};
use tokio::io::{self, AsyncBufReadExt, BufReader};
use std::sync::Arc;

const TOPIC_CLOUD_TELEMETRY: &str = "/cloud/telemetry";
const TOPIC_CLOUD_COMMAND: &str = "/cloud/command";
const SWITCH_AUTHORITY: &str = "ecu-switch";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_path = std::env::var("PACOM_MANIFEST_PATH").unwrap_or_else(|_| {
        format!(
            "{}/examples/mqtt_bridge/cloud-app/manifest.json",
            env!("CARGO_MANIFEST_DIR")
        )
    });

    let broker_uri = std::env::var("PACOM_MQTT_BROKER_URI")
        .unwrap_or_else(|_| "mqtt://127.0.0.1:1883".to_string());

    println!("[CLOUD_APP] Inizializzazione runtime (connessione a MQTT)...");
    let runtime = Arc::new(
        PacomRuntime::new(RuntimeConfig {
            mqtt_config: Some(MqttConfig {
                broker_uri: broker_uri.clone(),
                client_id: "cloud-app-client".to_string(),
            }),
            manifest_path: Some(manifest_path),
        })
        .await?,
    );

    // Sottoscrizione alle telemetrie ricevute da ecu-switch
    runtime.subscribe_event_from(TOPIC_CLOUD_TELEMETRY, SWITCH_AUTHORITY, |payload| {
        let status = String::from_utf8_lossy(&payload);
        println!("\n[CLOUD - TELEMETRIA] Stato luci veicolo: {}", status);
        print_menu();
    }).await?;

    print_menu();

    let stdin = BufReader::new(io::stdin());
    let mut lines = stdin.lines();

    while let Some(line) = lines.next_line().await? {
        let text = line.trim();
        if text.eq_ignore_ascii_case("/quit") {
            break;
        }

        let cmd = match text {
            "0" => Some("Off"),
            "1" => Some("Low Beam"),
            "2" => Some("High Beam"),
            _ => None,
        };

        if let Some(cmd_str) = cmd {
            println!("[CLOUD] Invio comando '{}' via MQTT a ecu-switch...", cmd_str);
            if let Err(e) = runtime
                .publish_event_to(TOPIC_CLOUD_COMMAND, SWITCH_AUTHORITY, cmd_str.as_bytes().to_vec())
                .await
            {
                eprintln!("[CLOUD - ERRORE MQTT] Invio comando fallito: {}", e);
            }
        } else {
            println!("Opzione non valida. Inserisci 0, 1 o 2 (o '/quit' per uscire).");
        }
    }

    Ok(())
}

fn print_menu() {
    println!("\n====================================");
    println!("[CLOUD] Invio comandi MQTT:");
    println!("0. Lights off");
    println!("1. Low beam");
    println!("2. High Beam");
    println!("====================================");
    print!("Scegli un'opzione (0-2 o '/quit'): ");
    use std::io::Write;
    let _ = std::io::stdout().flush();
}
