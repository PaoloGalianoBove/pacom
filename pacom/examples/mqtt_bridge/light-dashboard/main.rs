use pacom::{PacomRuntime, RuntimeConfig};
use tokio::io::{self, AsyncBufReadExt, BufReader};

const TOPIC_UP: &str = "/bridge/up";
const TOPIC_DOWN: &str = "/bridge/down";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_path = std::env::var("PACOM_MANIFEST_PATH").unwrap_or_else(|_| {
        format!(
            "{}/examples/mqtt_bridge/edge/manifest.json",
            env!("CARGO_MANIFEST_DIR")
        )
    });

    let runtime = PacomRuntime::new(RuntimeConfig { 
        mqtt_config: None,
        manifest_path: Some(manifest_path),
    }).await?;

    runtime
        .subscribe_event(TOPIC_DOWN, |payload| {
            let msg = String::from_utf8_lossy(&payload);
            println!("[EDGE] received from hub: {}", msg);
        })
        .await?;

    println!("[EDGE] interactive mode");
    println!("[EDGE] type text and press Enter to send via SOME/IP to hub");
    println!("[EDGE] type '/quit' to exit");

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
            .publish_event(TOPIC_UP, text.as_bytes().to_vec())
            .await?;
        println!("[EDGE] sent to hub: {}", text);
    }

    Ok(())
}
