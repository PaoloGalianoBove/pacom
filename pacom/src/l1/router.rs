use std::sync::Arc;
use async_trait::async_trait;
use log::{info, trace};
use up_rust::{UListener, UMessage, UStatus, UTransport, UUri};
use up_transport_vsomeip::UPTransportVsomeip;
use up_transport_mqtt5::Mqtt5Transport;

/// uStreamer router routing messages between vsomeip (in-vehicle) and mqtt5 (cloud) transports.
pub struct UStreamerRouter {
    authority: String,
    vsomeip: Arc<UPTransportVsomeip>,
    mqtt: Option<Arc<Mqtt5Transport>>,
}

impl UStreamerRouter {
    pub fn new(
        authority: String,
        vsomeip: Arc<UPTransportVsomeip>,
        mqtt: Option<Arc<Mqtt5Transport>>,
    ) -> Self {
        Self {
            authority,
            vsomeip,
            mqtt,
        }
    }

    /// Helper to identify if a target UUri is cloud-bound (requires MQTT) or local.
    fn is_cloud_bound(&self, uri: &UUri) -> bool {
        // If the target authority is different from our local authority, it goes to cloud
        let target_auth = uri.authority_name();
        !target_auth.is_empty() && target_auth != self.authority
    }
}

#[async_trait]
impl UTransport for UStreamerRouter {
    async fn send(&self, message: UMessage) -> Result<(), UStatus> {
        let is_cloud = if let Some(ref sink) = message.attributes.sink.as_ref() {
            self.is_cloud_bound(sink)
        } else {
            false
        };

        if is_cloud {
            if let Some(ref mqtt_tx) = self.mqtt {
                trace!("[Router] Routing cloud-bound message to MQTT 5 transport: {:?}", message);
                mqtt_tx.send(message).await
            } else {
                info!("[Router] Warning: Message is cloud-bound but MQTT is not configured");
                self.vsomeip.send(message).await
            }
        } else {
            trace!("[Router] Routing in-vehicle message to local vSomeIP transport");
            self.vsomeip.send(message).await
        }
    }

    async fn register_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        let is_cloud = self.is_cloud_bound(source_filter)
            || sink_filter.map(|s| self.is_cloud_bound(s)).unwrap_or(false);

        if is_cloud {
            if let Some(ref mqtt_tx) = self.mqtt {
                mqtt_tx.register_listener(source_filter, sink_filter, listener).await
            } else {
                info!("[Router] Warning: Topic requires MQTT registration but MQTT not configured");
                self.vsomeip.register_listener(source_filter, sink_filter, listener).await
            }
        } else {
            self.vsomeip.register_listener(source_filter, sink_filter, listener).await
        }
    }

    async fn unregister_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        let is_cloud = self.is_cloud_bound(source_filter)
            || sink_filter.map(|s| self.is_cloud_bound(s)).unwrap_or(false);

        if is_cloud {
            if let Some(ref mqtt_tx) = self.mqtt {
                mqtt_tx.unregister_listener(source_filter, sink_filter, listener).await
            } else {
                self.vsomeip.unregister_listener(source_filter, sink_filter, listener).await
            }
        } else {
            self.vsomeip.unregister_listener(source_filter, sink_filter, listener).await
        }
    }

    async fn receive(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
    ) -> Result<UMessage, UStatus> {
        // Receives from vsomeip since it's local
        self.vsomeip.receive(source_filter, sink_filter).await
    }
}

impl up_rust::LocalUriProvider for UStreamerRouter {
    fn get_authority(&self) -> String {
        self.authority.clone()
    }

    fn get_resource_uri(&self, resource_id: u16) -> UUri {
        self.vsomeip.get_resource_uri(resource_id)
    }

    fn get_source_uri(&self) -> UUri {
        self.vsomeip.get_source_uri()
    }
}
