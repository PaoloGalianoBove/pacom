use pacom::{MqttConfig, PacomError, PacomRuntime, RuntimeConfig};
use std::sync::Arc;
use up_rust::UCode;

const RPC_SET_LIGHTS: &str = "/rpc/lights/set";
const TOPIC_STATUS: &str = "/status/lights";
const TOPIC_CLOUD_TELEMETRY: &str = "/cloud/telemetry";
const TOPIC_CLOUD_COMMAND: &str = "/cloud/command";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_path = std::env::var("PACOM_MANIFEST_PATH").unwrap_or_else(|_| {
        format!(
            "{}/examples/mqtt_bridge/light-switch/manifest.json",
            env!("CARGO_MANIFEST_DIR")
        )
    });
    let authority = std::env::var("UP_AUTHORITY")
        .ok()
        .filter(|value| !value.trim().is_empty());

    let broker_uri = std::env::var("PACOM_MQTT_BROKER_URI")
        .unwrap_or_else(|_| "mqtt://127.0.0.1:1883".to_string());

    println!(
        "[LIGHT_SWITCH] Attempting to connect to MQTT broker at {}...",
        broker_uri
    );

    // Inizializzazione con fallback grazioso su fallimento connessione MQTT
    let runtime = match PacomRuntime::new(RuntimeConfig {
        mqtt_config: Some(MqttConfig {
            broker_uri: broker_uri.clone(),
            client_id: "light-switch-core".to_string(),
        }),
        manifest_path: Some(manifest_path.clone()),
        authority: authority.clone(),
    })
    .await
    {
        Ok(rt) => {
            println!("[LIGHT_SWITCH] Successfully connected to MQTT. Vehicle and cloud nodes ready.");
            Arc::new(rt)
        }
        Err(e) => {
            println!(
                "[WARNING - MQTT] Unreachable MQTT broker ({e}). Starting in local vehicle-only mode (SOME/IP)..."
            );
            let rt = PacomRuntime::new(RuntimeConfig {
                mqtt_config: None,
                manifest_path: Some(manifest_path),
                authority,
            })
            .await?;
            Arc::new(rt)
        }
    };

    // 1. Registrazione RPC locale (SOME/IP)
    let runtime_clone = runtime.clone();
    runtime
        .register_rpc_method(RPC_SET_LIGHTS, move |payload| {
            let runtime_clone = runtime_clone.clone();
            async move {
                let cmd = String::from_utf8_lossy(&payload).into_owned();
                println!("[HMI ➔ SOME/IP] Received RPC command: '{}'", cmd);

                let _ = runtime_clone
                    .publish_event(TOPIC_CLOUD_TELEMETRY, cmd.as_bytes().to_vec())
                    .await;

                println!(
                    "[FEEDBACK] Risposta RPC inviata. Stato impostato a: '{}'",
                    cmd
                );
                cmd.into_bytes()
            }
        })
        .await?;

    // 2. Sottoscrizione comandi Cloud (MQTT)
    let runtime_clone = runtime.clone();
    let cloud_sub_result = runtime
        .subscribe_event(TOPIC_CLOUD_COMMAND, move |payload| {
            let runtime_clone = runtime_clone.clone();
            async move {
                let cmd = String::from_utf8_lossy(&payload).into_owned();
                println!("[CLOUD ➔ MQTT] Received command from Cloud: '{}'", cmd);
                println!(
                    "[CLOUD ➔ MQTT] Cloud command '{}' applied successfully.",
                    cmd
                );

                let rt = runtime_clone.clone();
                let _ = rt
                    .publish_event(TOPIC_STATUS, cmd.clone().as_bytes().to_vec())
                    .await;
                let _ = rt
                    .publish_event(TOPIC_CLOUD_TELEMETRY, cmd.clone().as_bytes().to_vec())
                    .await;
            }
        })
        .await;

    match cloud_sub_result {
        Ok(()) => {}
        Err(PacomError::Transport(status))
            if status.code.enum_value_or_default() == UCode::UNAVAILABLE =>
        {
            println!(
                "[WARNING - MQTT] Cloud subscription unavailable ({}). Proceeding in local-only mode.",
                status
                    .message
                    .clone()
                    .unwrap_or_else(|| "UNAVAILABLE".to_string())
            );
        }
        Err(e) => return Err(e.into()),
    }

    // Pubblica lo stato iniziale per registrarlo nel Service Discovery.
    let _ = runtime.publish_event(TOPIC_STATUS, b"Off".to_vec()).await;

    println!("[LIGHT_SWITCH] Waiting for HMI (RPC) or Cloud (MQTT) commands...");

    // Tieni in vita il thread principale
    std::thread::park();
    Ok(())
}
