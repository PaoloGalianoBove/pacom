use pacom::{PacomRuntime, RuntimeConfig};
use std::sync::{Arc, RwLock};
use tokio::io::{self, AsyncBufReadExt, BufReader};

const RPC_SET_LIGHTS: &str = "/rpc/lights/set";
const TOPIC_STATUS: &str = "/status/lights";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_path = std::env::var("PACOM_MANIFEST_PATH").unwrap_or_else(|_| {
        format!(
            "{}/examples/mqtt_bridge/light-dashboard/manifest.json",
            env!("CARGO_MANIFEST_DIR")
        )
    });
    let authority = std::env::var("UP_AUTHORITY")
        .ok()
        .filter(|value| !value.trim().is_empty());

    println!("[DASHBOARD] Inizializzazione runtime (solo locale SOME/IP)...");
    let runtime = Arc::new(
        PacomRuntime::new(RuntimeConfig {
            mqtt_config: None,
            manifest_path: Some(manifest_path),
            authority,
        })
        .await?,
    );

    let current_status = Arc::new(RwLock::new("Off".to_string()));

    // La subscribe allo stato e obbligatoria: serve a riflettere anche
    // i comandi provenienti dal cloud e non solo le risposte RPC locali.
    let current_status_clone = current_status.clone();
    runtime
        .subscribe_event(TOPIC_STATUS, move |payload| {
            let status = String::from_utf8_lossy(&payload).into_owned();
            if let Ok(mut lock) = current_status_clone.write() {
                *lock = status.clone();
            }
            println!(
                "\n[DASHBOARD - RICEVUTO AGGIORNAMENTO] Stato luci cambiato in: {}",
                status
            );
            print_menu(&status);
        })
        .await?;
    println!("[DASHBOARD] Subscribe stato locale abilitata.");

    // Stampa iniziale del menu
    {
        let status = current_status.read().unwrap().clone();
        print_menu(&status);
    }

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
            println!(
                "[DASHBOARD] Invio comando RPC '{}' a light_switch...",
                cmd_str
            );
            match runtime
                .invoke_rpc_method(RPC_SET_LIGHTS, cmd_str.as_bytes().to_vec())
                .await
            {
                Ok(resp) => {
                    let confirmed_status = String::from_utf8_lossy(&resp).into_owned();
                    println!(
                        "[DASHBOARD] Risposta RPC da light_switch: Stato impostato a '{}'",
                        confirmed_status
                    );
                    if let Ok(mut lock) = current_status.write() {
                        *lock = confirmed_status.clone();
                    }
                    print_menu(&confirmed_status);
                }
                Err(e) => {
                    eprintln!(
                        "[DASHBOARD - ERRORE RPC] Impossibile inviare comando: {}",
                        e
                    );
                }
            }
        } else {
            println!("Opzione non valida. Inserisci 0, 1 o 2 (o '/quit' per uscire).");
        }
    }

    Ok(())
}

fn print_menu(status: &str) {
    println!("\n====================================");
    println!("current status: {}", status);
    println!("0. Lights off");
    println!("1. Low beam");
    println!("2. High Beam");
    println!("====================================");
    print!("Scegli un'opzione (0-2 o '/quit'): ");
    use std::io::Write;
    let _ = std::io::stdout().flush();
}
