use std::time::Instant;

use pacom::{PacomRuntime, RuntimeConfig};

pub struct RttClientApp {
    runtime: PacomRuntime,
}

impl RttClientApp {
    pub async fn new(manifest_path: String) -> Result<Self, Box<dyn std::error::Error>> {
        let authority = std::env::var("UP_AUTHORITY")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let runtime = PacomRuntime::new(RuntimeConfig {
            mqtt_config: None,
            manifest_path: Some(manifest_path),
            authority,
        })
        .await?;
        println!("[CLIENT] PacomRuntime created.");
        Ok(Self { runtime })
    }

    pub async fn warm_up(&self, method: &str, count: usize) {
        for _ in 0..count {
            let _ = self.runtime.invoke_rpc_method(method, b"warmup".to_vec()).await;
        }
    }

    pub async fn invoke_timed(
        &self,
        method: &str,
        payload: Vec<u8>,
    ) -> (f64, Result<Vec<u8>, pacom::PacomError>) {
        let start = Instant::now();
        let result = self.runtime.invoke_rpc_method(method, payload).await;
        let rtt_ms = start.elapsed().as_secs_f64() * 1000.0;
        (rtt_ms, result)
    }
}
