use pacom::{MqttConfig, PacomError, PacomRuntime, RuntimeConfig};
use std::sync::Arc;
use tokio::sync::Mutex;
use up_rust::UCode;

use kuksa_rust_sdk::kuksa::common::ClientTraitV2;
use kuksa_rust_sdk::kuksa::val::v2::KuksaClientV2;
use kuksa_rust_sdk::v2_proto::Value;
use kuksa_rust_sdk::v2_proto::value::TypedValue;

const RPC_SET_LIGHTS: &str = "/rpc/lights/set";
const TOPIC_STATUS: &str = "/status/lights";
const TOPIC_CLOUD_TELEMETRY: &str = "/cloud/telemetry";
const TOPIC_CLOUD_COMMAND: &str = "/cloud/command";
const AMBIENT_COLOR_SIGNAL: &str =
    "Vehicle.Cabin.Light.AmbientLight.Row1.DriverSide.Color";

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

    let kuksa_uri = std::env::var("PACOM_KUKSA_URI")
        .unwrap_or_else(|_| "http://127.0.0.1:55555".to_string());
    
    // Parse l'URI in modo sicuro (kuksa-rust-sdk lo usa internamente)
    let uri: http::Uri = kuksa_uri.parse().expect("Invalid Kuksa URI");
    let kuksa_client = Arc::new(Mutex::new(KuksaClientV2::new(uri)));

    // 1. Registrazione RPC locale (SOME/IP)
    let runtime_clone = runtime.clone();
    let kuksa_clone_rpc = kuksa_client.clone();
    runtime
        .register_rpc_method(RPC_SET_LIGHTS, move |payload| {
            let runtime_clone = runtime_clone.clone();
            let kuksa_clone = kuksa_clone_rpc.clone();
            async move {
                let cmd = String::from_utf8_lossy(&payload).into_owned();
                println!("[HMI ➔ SOME/IP] Received RPC command: '{}'", cmd);

                // Attua sui segnali VSS
                actuate_lights(kuksa_clone, &cmd).await;

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
    let kuksa_clone_mqtt = kuksa_client.clone();
    let cloud_sub_result = runtime
        .subscribe_event(TOPIC_CLOUD_COMMAND, move |payload| {
            let runtime_clone = runtime_clone.clone();
            let kuksa_clone = kuksa_clone_mqtt.clone();
            async move {
                let cmd = String::from_utf8_lossy(&payload).into_owned();
                println!("[CLOUD ➔ MQTT] Received command from Cloud: '{}'", cmd);
                
                // Attua sui segnali VSS
                actuate_lights(kuksa_clone, &cmd).await;

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
    let _ = runtime.publish_event(TOPIC_STATUS, b"All Off".to_vec()).await;

    println!("[LIGHT_SWITCH] Waiting for HMI (RPC) or Cloud (MQTT) commands...");

    // Tieni in vita il thread principale
    std::thread::park();
    Ok(())
}

async fn actuate_lights(kuksa_client: Arc<Mutex<KuksaClientV2>>, cmd: &str) {
    let Some(hex) = cmd.strip_prefix("RGB:") else {
        // Low/high beam commands are intentionally disabled: this deployment
        // maps only the ambient RGB actuator to CAN.
        // "Low Beam On" / "Low Beam Off" / "High Beam On" / "High Beam Off"
        // are no longer sent to KUKSA.
        eprintln!("[KUKSA] Unsupported command: {}", cmd);
        return;
    };
    let Some(color) = normalize_rgb_hex(hex) else {
        eprintln!("[KUKSA] Invalid RGB command. Expected RGB followed by six hex digits.");
        return;
    };

    let mut client = kuksa_client.lock().await;
    let _ = client
        .actuate(
            AMBIENT_COLOR_SIGNAL.to_string(),
            Value {
                typed_value: Some(TypedValue::String(color.clone())),
            },
        )
        .await
        .map_err(|e| eprintln!("[KUKSA] Error actuating ambient RGB color: {:?}", e));
    println!("[KUKSA] Actuated ambient RGB color to {}", color);
}

fn normalize_rgb_hex(input: &str) -> Option<String> {
    let hex = input.trim();
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("#{}", hex.to_ascii_uppercase()))
}
