use pacom::{MqttConfig, PacomError, PacomRuntime, RuntimeConfig};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::Mutex as TokioMutex;
use tokio::time::sleep;
use up_rust::UCode;

const RPC_SET_LIGHTS: &str = "/rpc/lights/set";
const TOPIC_STATUS: &str = "/status/lights";
const TOPIC_CLOUD_TELEMETRY: &str = "/cloud/telemetry";
const TOPIC_CLOUD_COMMAND: &str = "/cloud/command";
const CLOUD_AUTHORITY: &str = "cloud.bridge";

fn rpc_post_publish_delay_ms() -> u64 {
    std::env::var("PACOM_RPC_POST_PUBLISH_DELAY_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(80)
}

fn publish_status_on_hmi_rpc() -> bool {
    std::env::var("PACOM_PUBLISH_STATUS_ON_HMI_RPC")
        .ok()
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes" || v == "on"
        })
        .unwrap_or(false)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_path = std::env::var("PACOM_MANIFEST_PATH").unwrap_or_else(|_| {
        format!(
            "{}/examples/mqtt_bridge/light-switch/manifest.json",
            env!("CARGO_MANIFEST_DIR")
        )
    });

    let broker_uri = std::env::var("PACOM_MQTT_BROKER_URI")
        .unwrap_or_else(|_| "mqtt://127.0.0.1:1883".to_string());

    println!(
        "[LIGHT_SWITCH] Tentativo di connessione al broker MQTT in {}...",
        broker_uri
    );

    // Inizializzazione con fallback grazioso su fallimento connessione MQTT
    let runtime = match PacomRuntime::new(RuntimeConfig {
        mqtt_config: Some(MqttConfig {
            broker_uri: broker_uri.clone(),
            client_id: "light-switch-core".to_string(),
        }),
        manifest_path: Some(manifest_path.clone()),
    })
    .await
    {
        Ok(rt) => {
            println!("[LIGHT_SWITCH] Connessione a MQTT riuscita. Nodi veicolo e cloud pronti.");
            Arc::new(rt)
        }
        Err(e) => {
            println!(
                "[WARNING - MQTT] Broker MQTT non raggiungibile ({e}). Avvio in modalità solo-veicolo locale (SOME/IP)..."
            );
            let rt = PacomRuntime::new(RuntimeConfig {
                mqtt_config: None,
                manifest_path: Some(manifest_path),
            })
            .await?;
            Arc::new(rt)
        }
    };

    // Stato protetto da RwLock per la concorrenza tra i thread dei vari trasporti
    let current_state = Arc::new(RwLock::new("Off".to_string()));
    let last_someip_time = Arc::new(RwLock::new(None::<Instant>));
    let publish_gate = Arc::new(TokioMutex::new(()));

    // 1. Registrazione RPC locale (SOME/IP)
    let current_state_clone = current_state.clone();
    let last_someip_time_clone = last_someip_time.clone();
    let runtime_clone = runtime.clone();
    let publish_gate_clone = publish_gate.clone();
    runtime
        .register_rpc_method(RPC_SET_LIGHTS, move |payload| {
            let current_state_clone = current_state_clone.clone();
            let last_someip_time_clone = last_someip_time_clone.clone();
            let runtime_clone = runtime_clone.clone();
            let publish_gate_clone = publish_gate_clone.clone();
            async move {
                let cmd = String::from_utf8_lossy(&payload).into_owned();
                println!("[HMI ➔ SOME/IP] Ricevuto comando RPC: '{}'", cmd);

                // Aggiorna lo stato e segna il timestamp dell'ultimo comando locale
                let mut state_changed = true;
                if let Ok(mut state) = current_state_clone.write() {
                    state_changed = *state != cmd;
                    *state = cmd.clone();
                }
                if let Ok(mut time) = last_someip_time_clone.write() {
                    *time = Some(Instant::now());
                }

                // Priorita alla risposta RPC: differiamo leggermente la fan-out publish
                // per ridurre la contesa di trasporto durante il path request/response.
                let rt = runtime_clone.clone();
                let cmd_pub = cmd.clone();
                let publish_delay = rpc_post_publish_delay_ms();
                let publish_local_status = publish_status_on_hmi_rpc();
                let publish_gate = publish_gate_clone.clone();
                tokio::spawn(async move {
                    let _publish_guard = publish_gate.lock().await;
                    if !state_changed {
                        return;
                    }
                    if publish_delay > 0 {
                        sleep(Duration::from_millis(publish_delay)).await;
                    }
                    if publish_local_status {
                        let _ = rt
                            .publish_event(TOPIC_STATUS, cmd_pub.as_bytes().to_vec())
                            .await;
                    }
                    let _ = rt
                        .publish_event_to(
                            TOPIC_CLOUD_TELEMETRY,
                            CLOUD_AUTHORITY,
                            cmd_pub.as_bytes().to_vec(),
                        )
                        .await;
                });

                println!(
                    "[FEEDBACK] Risposta RPC inviata. Stato impostato a: '{}'",
                    cmd
                );
                cmd.into_bytes()
            }
        })
        .await?;

    // 2. Sottoscrizione comandi Cloud (MQTT)
    let current_state_clone = current_state.clone();
    let last_someip_time_clone = last_someip_time.clone();
    let runtime_clone = runtime.clone();
    let publish_gate_clone = publish_gate.clone();
    let cloud_sub_result = runtime.subscribe_event_from(TOPIC_CLOUD_COMMAND, CLOUD_AUTHORITY, move |payload| {
        let cmd = String::from_utf8_lossy(&payload).into_owned();
        println!("[CLOUD ➔ MQTT] Ricevuto comando dal Cloud: '{}'", cmd);
        let publish_gate_clone = publish_gate_clone.clone();

        // Controllo della Race Condition (finestra di 1.5 secondi)
        let now = Instant::now();
        let mut is_collision = false;

        if let Ok(time_lock) = last_someip_time_clone.read() {
            if let Some(last_time) = *time_lock {
                if now.duration_since(last_time) < Duration::from_millis(1500) {
                    is_collision = true;
                }
            }
        }

        if is_collision {
            let current = current_state_clone.read().unwrap().clone();
            println!("⚠️ [COLLISIONE RILEVATA] Scarto comando Cloud '{}'. Precedenza a HMI (stato attuale: '{}').", cmd, current);

            // Invia telemetria per risincronizzare il cloud allo stato locale
            let rt = runtime_clone.clone();
            tokio::spawn(async move {
                let _publish_guard = publish_gate_clone.lock().await;
                let _ = rt.publish_event_to(TOPIC_CLOUD_TELEMETRY, CLOUD_AUTHORITY, current.as_bytes().to_vec()).await;
            });
        } else {
            // Nessuna collisione, applica il comando
            let mut state_changed = true;
            if let Ok(mut state) = current_state_clone.write() {
                state_changed = *state != cmd;
                *state = cmd.clone();
            }
            println!("[CLOUD ➔ MQTT] Comando Cloud '{}' applicato con successo.", cmd);

            let rt = runtime_clone.clone();
            let cmd_pub = cmd.clone();
            tokio::spawn(async move {
                let _publish_guard = publish_gate_clone.lock().await;
                if !state_changed {
                    return;
                }
                let _ = rt.publish_event(TOPIC_STATUS, cmd_pub.as_bytes().to_vec()).await;
                let _ = rt.publish_event_to(TOPIC_CLOUD_TELEMETRY, CLOUD_AUTHORITY, cmd_pub.as_bytes().to_vec()).await;
            });
        }
    }).await;

    match cloud_sub_result {
        Ok(()) => {}
        Err(PacomError::Transport(status))
            if status.code.enum_value_or_default() == UCode::UNAVAILABLE =>
        {
            println!(
                "[WARNING - MQTT] Subscribe cloud non disponibile ({}). Continuo in modalita solo locale.",
                status.message.clone().unwrap_or_else(|| "UNAVAILABLE".to_string())
            );
        }
        Err(e) => return Err(e.into()),
    }

    // Pubblica lo stato iniziale per registrarlo nel Service Discovery ed evitare deadlock di avvio
    let _ = runtime.publish_event(TOPIC_STATUS, b"Off".to_vec()).await;

    println!("[LIGHT_SWITCH] In attesa di comandi HMI (RPC) o Cloud (MQTT)...");

    // Tieni in vita il thread principale
    std::thread::park();
    Ok(())
}
