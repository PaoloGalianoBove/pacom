use std::sync::Arc;
use up_rust::{UCode, UStatus};
use up_transport_mqtt5::{Mqtt5Transport, Mqtt5TransportOptions, MqttClientOptions, TransportMode};

/// Configures and instantiates the official uProtocol MQTT 5 transport.
pub async fn setup_mqtt_transport(
    broker_uri: &str,
    client_id: &str,
    authority: &str,
) -> Result<Arc<Mqtt5Transport>, UStatus> {
    // Define options for MQTT client connection
    let mut client_options = MqttClientOptions::default();
    client_options.broker_uri = broker_uri.to_string();
    client_options.client_id = Some(client_id.to_string());

    // Configure transport options for OffVehicle mode (telemetry/cloud)
    let options = Mqtt5TransportOptions {
        max_filters: 10_000,
        max_listeners_per_filter: 100,
        mode: TransportMode::OffVehicle,
        mqtt_client_options: client_options,
    };

    // Instantiate Mqtt5Transport using the async constructor
    let transport = Mqtt5Transport::new(options, authority).await.map_err(|e| {
        UStatus::fail_with_code(
            UCode::INTERNAL,
            format!("Failed to initialize Mqtt5Transport: {e:?}"),
        )
    })?;

    // We MUST explicitly call connect() to establish the connection,
    // otherwise the transport remains UNAVAILABLE indefinitely.
    transport.connect().await.map_err(|e| {
        UStatus::fail_with_code(
            UCode::INTERNAL,
            format!("Failed to connect Mqtt5Transport: {e:?}"),
        )
    })?;

    Ok(Arc::new(transport))
}
