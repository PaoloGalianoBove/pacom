use std::sync::Arc;
use async_trait::async_trait;
use log::{info, trace};
use up_rust::{UListener, UMessage, UStatus, UTransport, UUri};
use up_transport_vsomeip::UPTransportVsomeip;
use up_transport_mqtt5::Mqtt5Transport;

/// uStreamer router routing messages between vSomeIP and MQTT transports.
pub struct UStreamerRouter {
    authority: String,
    vsomeip: Option<Arc<UPTransportVsomeip>>,
    mqtt: Option<Arc<Mqtt5Transport>>,
}

impl UStreamerRouter {
    pub fn new(
        authority: String,
        vsomeip: Option<Arc<UPTransportVsomeip>>,
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
        // If the target authority is different from our local authority, it goes to cloud.
        // Wildcard authority "*" is NOT exclusively cloud-bound.
        let target_auth = uri.authority_name();
        !target_auth.is_empty() && target_auth != self.authority && target_auth != "*"
    }
}

#[async_trait]
impl UTransport for UStreamerRouter {
    async fn send(&self, message: UMessage) -> Result<(), UStatus> {
        if message.attributes.sink.is_none() {
            // It's a Publish message. Broadcast to all transports!
            let mut success = false;
            let mut last_err = None;

            if let Some(ref mqtt_tx) = self.mqtt {
                trace!("[Router] Broadcasting Publish to MQTT 5 transport: {:?}", message);
                match mqtt_tx.send(message.clone()).await {
                    Ok(_) => success = true,
                    Err(e) => last_err = Some(e),
                }
            }

            if let Some(ref v) = self.vsomeip {
                trace!("[Router] Broadcasting Publish to local vSomeIP transport");
                match v.send(message).await {
                    Ok(_) => success = true,
                    Err(e) => last_err = Some(e),
                }
            }

            if success {
                return Ok(());
            } else if let Some(e) = last_err {
                return Err(e);
            } else {
                return Err(UStatus::fail_with_code(up_rust::UCode::UNAVAILABLE, "No transport available"));
            }
        }

        // For RPC messages (Request/Response), route based on the sink authority
        let is_cloud = self.is_cloud_bound(message.attributes.sink.as_ref().unwrap());

        if is_cloud || self.vsomeip.is_none() {
            if let Some(ref mqtt_tx) = self.mqtt {
                trace!("[Router] Routing RPC message to MQTT 5 transport: {:?}", message);
                mqtt_tx.send(message).await
            } else {
                info!("[Router] Warning: MQTT is not configured and vSomeIP is disabled");
                Err(UStatus::fail_with_code(up_rust::UCode::UNAVAILABLE, "No transport available"))
            }
        } else {
            trace!("[Router] Routing RPC message to local vSomeIP transport");
            if let Some(ref v) = self.vsomeip {
                v.send(message).await
            } else {
                Err(UStatus::fail_with_code(up_rust::UCode::UNAVAILABLE, "No transport available"))
            }
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

        if is_cloud || self.vsomeip.is_none() {
            if let Some(ref mqtt_tx) = self.mqtt {
                let default_sink = UUri::try_from_parts(&self.authority, 0xFFFF, 0xFF, 0xFFFF).unwrap();
                let effective_sink = Some(sink_filter.unwrap_or(&default_sink));
                let mut retries = 50;
                loop {
                    match mqtt_tx.register_listener(source_filter, effective_sink, listener.clone()).await {
                        Ok(_) => return Ok(()),
                        Err(e) if e.code.enum_value_or_default() == up_rust::UCode::UNAVAILABLE && retries > 0 => {
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            retries -= 1;
                        }
                        Err(e) => return Err(e),
                    }
                }
            } else {
                info!("[Router] Warning: MQTT is not configured and vSomeIP is disabled");
                Err(UStatus::fail_with_code(up_rust::UCode::UNAVAILABLE, "No transport available"))
            }
        } else {
            if let Some(ref v) = self.vsomeip {
                v.register_listener(source_filter, sink_filter, listener).await
            } else {
                Err(UStatus::fail_with_code(up_rust::UCode::UNAVAILABLE, "No transport available"))
            }
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

        if is_cloud || self.vsomeip.is_none() {
            if let Some(ref mqtt_tx) = self.mqtt {
                let default_sink = UUri::try_from_parts(&self.authority, 0xFFFF, 0xFF, 0xFFFF).unwrap();
                let effective_sink = Some(sink_filter.unwrap_or(&default_sink));
                let mut retries = 50;
                loop {
                    match mqtt_tx.unregister_listener(source_filter, effective_sink, listener.clone()).await {
                        Ok(_) => return Ok(()),
                        Err(e) if e.code.enum_value_or_default() == up_rust::UCode::UNAVAILABLE && retries > 0 => {
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            retries -= 1;
                        }
                        Err(e) => return Err(e),
                    }
                }
            } else {
                Err(UStatus::fail_with_code(up_rust::UCode::UNAVAILABLE, "No transport available"))
            }
        } else {
            if let Some(ref v) = self.vsomeip {
                v.unregister_listener(source_filter, sink_filter, listener).await
            } else {
                Err(UStatus::fail_with_code(up_rust::UCode::UNAVAILABLE, "No transport available"))
            }
        }
    }

    async fn receive(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
    ) -> Result<UMessage, UStatus> {
        if let Some(ref v) = self.vsomeip {
            v.receive(source_filter, sink_filter).await
        } else {
            Err(UStatus::fail_with_code(up_rust::UCode::UNAVAILABLE, "No transport available"))
        }
    }
}

impl up_rust::LocalUriProvider for UStreamerRouter {
    fn get_authority(&self) -> String {
        self.authority.clone()
    }

    fn get_resource_uri(&self, resource_id: u16) -> UUri {
        if let Some(ref v) = self.vsomeip {
            v.get_resource_uri(resource_id)
        } else {
            UUri::try_from_parts(&self.authority, 0, 0, resource_id).unwrap()
        }
    }

    fn get_source_uri(&self) -> UUri {
        if let Some(ref v) = self.vsomeip {
            v.get_source_uri()
        } else {
            UUri::try_from_parts(&self.authority, 0, 0, 0).unwrap()
        }
    }
}
