use up_rust::{UUri, UStatus, UListener, UMessage};
use up_transport_mqtt5::{Mqtt5Transport, Mqtt5TransportOptions, MqttClientOptions, TransportMode};
use std::sync::Arc;

struct DummyListener;
#[async_trait::async_trait]
impl UListener for DummyListener {
    async fn on_receive(&self, _: UMessage) {}
}

#[tokio::main]
async fn main() {
    let mut opts = MqttClientOptions::default();
    opts.broker_uri = "tcp://127.0.0.1:1883".to_string();
    opts.client_id = Some("test".to_string());
    let mut m5opts = Mqtt5TransportOptions::default();
    m5opts.mqtt_client_options = opts;
    m5opts.mode = TransportMode::OffVehicle;
    
    let tx = Mqtt5Transport::new(m5opts, "ecu-probe").await.unwrap();
    let source = UUri::try_from_parts("cloud.bridge", 0x2200, 1, 0x8000).unwrap();
    let res = up_rust::UTransport::register_listener(&tx, &source, None, Arc::new(DummyListener)).await;
    println!("Register result: {:?}", res);
}
