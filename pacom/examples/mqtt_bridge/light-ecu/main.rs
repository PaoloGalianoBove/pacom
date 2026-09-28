use kuksa_rust_sdk::kuksa::common::ClientTraitV2;
use kuksa_rust_sdk::kuksa::val::v2::KuksaClientV2;
use kuksa_rust_sdk::v2_proto::open_provider_stream_request::Action;
use kuksa_rust_sdk::v2_proto::signal_id::Signal::Path;
use kuksa_rust_sdk::v2_proto::{
    ProvideActuationRequest, SignalId,
};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let kuksa_uri = env::var("PACOM_KUKSA_URI")
        .unwrap_or_else(|_| "http://127.0.0.1:55555".to_string());
    
    let uri: http::Uri = kuksa_uri.parse().expect("Invalid Kuksa URI");
    let mut client = KuksaClientV2::new(uri);

    println!("[LIGHT ECU] Connecting to Kuksa at {}...", kuksa_uri);

    // Apriamo uno stream come provider
    let mut stream = client.open_provider_stream(None).await?;

    let signals = vec![
        "Vehicle.Body.Lights.Beam.Low.IsOn".to_string(),
        "Vehicle.Body.Lights.Beam.High.IsOn".to_string(),
    ];

    let actuator_identifiers: Vec<SignalId> = signals
        .iter()
        .map(|signal| SignalId {
            signal: Some(Path(signal.clone())),
        })
        .collect();

    // Inviamo la richiesta di registrarsi come fornitore di queste attuazioni
    let request = kuksa_rust_sdk::v2_proto::OpenProviderStreamRequest {
        action: Some(Action::ProvideActuationRequest(ProvideActuationRequest {
            actuator_identifiers,
        })),
    };

    stream
        .sender
        .send(request)
        .await
        .expect("Could not send ProvideActuationRequest");
    // Retrieve Metadata dynamically to avoid hardcoded IDs
    let mut id_to_path = std::collections::HashMap::new();
    for path in &signals {
        if let Ok(metadata_list) = client.list_metadata((path.clone(), String::new())).await {
            for metadata in metadata_list {
                let id = metadata.id as u32;
                id_to_path.insert(id, path.clone());
                println!("[LIGHT ECU] Mapped {} -> vss_id {}", path, id);
            }
        }
    }

    println!("[LIGHT ECU] Registered as Provider for Low/High beam. Listening for actuate requests...");

    // Rimaniamo in ascolto delle richieste dal databroker
    loop {
        match stream.receiver_stream.message().await {
            Ok(Some(response)) => {
                match response.action {
                    Some(kuksa_rust_sdk::v2_proto::open_provider_stream_response::Action::BatchActuateStreamRequest(batch_req)) => {
                        for req in batch_req.actuate_requests {
                            println!("[LIGHT ECU DEBUG] Raw request: {:?}", req);
                            let path = match req.signal_id.clone() {
                                Some(SignalId { signal: Some(Path(p)) }) => p,
                                Some(SignalId { signal: Some(kuksa_rust_sdk::v2_proto::signal_id::Signal::Id(i)) }) => {
                                    if let Some(resolved_path) = id_to_path.get(&(i as u32)) {
                                        resolved_path.clone()
                                    } else {
                                        format!("vss_id {}", i)
                                    }
                                },
                                _ => "Unknown".to_string(),
                            };

                            let val = match req.value {
                                Some(v) => match v.typed_value {
                                    Some(kuksa_rust_sdk::v2_proto::value::TypedValue::Bool(b)) => b,
                                    _ => false,
                                },
                                None => false,
                            };

                            println!("[LIGHT ECU - CAN BUS] Ricevuto actuate su {}: {}", path, val);
                            
                            // Fake CAN Frame Generation
                            if path.contains("Low.IsOn") {
                                let can_data = if val { "01" } else { "00" };
                                println!("   => Generazione frame CAN: can0 42A#{}", can_data);
                            } else if path.contains("High.IsOn") {
                                let can_data = if val { "01" } else { "00" };
                                println!("   => Generazione frame CAN: can0 42B#{}", can_data);
                            }
                        }
                    },
                    Some(other) => {
                        println!("[LIGHT ECU] Received other action from databroker: {:?}", other);
                    },
                    None => {
                        println!("[LIGHT ECU] Received response with no action from databroker.");
                    }
                }
            }
            Ok(None) => {
                println!("[LIGHT ECU] Stream closed by server.");
                break;
            }
            Err(e) => {
                eprintln!("[LIGHT ECU] Error reading from stream: {:?}", e);
                break;
            }
        }
    }

    Ok(())
}
