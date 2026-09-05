use async_trait::async_trait;
use log::{info, trace};
use std::sync::Arc;
use up_rust::{UListener, UMessage, UStatus, UTransport, UUri};
use up_transport_mqtt5::Mqtt5Transport;
use up_transport_vsomeip::UPTransportVsomeip;

/// Pacom router routing messages between vSomeIP and MQTT transports.
pub struct PacomRouter {
    authority: String,
    vsomeip: Option<Arc<UPTransportVsomeip>>,
    mqtt: Option<Arc<Mqtt5Transport>>,
}

use crate::utils::{dbg_log, rpc_diag_enabled, rpc_diag_log, uri_dbg, verbose_debug_enabled};

fn is_local_only_publish(message: &UMessage) -> bool {
    message.attributes.sink.is_none()
}

fn is_wildcard_resource_id(resource_id: u32) -> bool {
    resource_id == 0xFFFF || resource_id == u32::MAX
}

fn configured_cloud_authority() -> Option<String> {
    std::env::var("PACOM_CLOUD_AUTHORITY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

impl PacomRouter {
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

    /// Returns true if a message targeting `uri` must be routed via the cross-domain
    /// transport (MQTT) rather than the local intra-vehicle transport (vSomeIP).
    ///
    /// Cross-domain routing is currently triggered for explicit cross-domain markers,
    /// MQTT wildcard UE-IDs, or when vSomeIP is unavailable on this node.
    pub fn is_cloud_bound(&self, uri: &UUri) -> bool {
        let target_auth = uri.authority_name();

        // Explicit cross-domain marker used by publish_to_authority():
        // //configured-cloud-authority/0/0/0.
        if uri.ue_id == 0
            && uri.ue_version_major == 0
            && uri.resource_id == 0
            && configured_cloud_authority().as_deref() == Some(&target_auth)
        {
            dbg_log(
                "Router",
                format!(
                    "is_cloud_bound=true reason=explicit_cross_domain_marker uri={}",
                    uri_dbg(uri)
                ),
            );
            return true;
        }

        // A wildcard or empty authority alone describes a generic filter, not a
        // cloud route. The cloud sink marker or configured authority must decide it.
        if configured_cloud_authority().as_deref() == Some(&target_auth)
            && !target_auth.is_empty()
            && target_auth != "*"
        {
            dbg_log(
                "Router",
                format!(
                    "is_cloud_bound=true reason=configured_cloud_authority uri={}",
                    uri_dbg(uri)
                ),
            );
            return true;
        }

        // Otherwise keep addressed traffic on vSomeIP even across different authorities.
        dbg_log(
            "Router",
            format!(
                "is_cloud_bound=false reason=local_or_generic_filter uri={}",
                uri_dbg(uri)
            ),
        );
        false
    }

    fn listener_cloud_path(&self, source_filter: &UUri, sink_filter: Option<&UUri>) -> bool {
        // If a local sink is explicitly provided, this listener is intended for local routing.
        if let Some(sink) = sink_filter {
            if !self.is_cloud_bound(sink) {
                dbg_log(
                    "Router",
                    format!(
                        "listener_cloud_path=false reason=explicit_local_sink source={} sink={}",
                        uri_dbg(source_filter),
                        uri_dbg(sink)
                    ),
                );
                return false;
            }
        }

        let decision = self.is_cloud_bound(source_filter)
            || sink_filter.map(|s| self.is_cloud_bound(s)).unwrap_or(false);
        dbg_log(
            "Router",
            format!(
                "listener_cloud_path={} source={} sink={}",
                decision,
                uri_dbg(source_filter),
                sink_filter
                    .map(uri_dbg)
                    .unwrap_or_else(|| "<none>".to_string())
            ),
        );
        decision
    }

    async fn register_cloud_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        let Some(mqtt_tx) = self.mqtt.as_ref() else {
            info!("[Router] Cloud listener registration skipped: MQTT transport not configured");
            return Err(UStatus::fail_with_code(
                up_rust::UCode::UNAVAILABLE,
                "MQTT transport not configured",
            ));
        };

        let sink_filter = sink_filter.ok_or_else(|| {
            UStatus::fail_with_code(
                up_rust::UCode::INVALID_ARGUMENT,
                "Cloud listener registration requires a sink filter",
            )
        })?;
        let mut retries = 50;
        let mut attempt = 1;
        loop {
            match mqtt_tx
                .register_listener(source_filter, Some(sink_filter), listener.clone())
                .await
            {
                Ok(_) => {
                    dbg_log(
                        "Router",
                        format!(
                            "register_cloud_listener(): mqtt registration succeeded attempts={} source={} sink={}",
                            attempt,
                            uri_dbg(source_filter),
                            uri_dbg(sink_filter)
                        ),
                    );
                    return Ok(());
                }
                Err(e)
                    if e.code.enum_value_or_default() == up_rust::UCode::UNAVAILABLE
                        && retries > 0 =>
                {
                    dbg_log(
                        "Router",
                        format!(
                            "register_cloud_listener(): mqtt unavailable retry attempt={} remaining={} source={} sink={} code={:?}",
                            attempt,
                            retries,
                            uri_dbg(source_filter),
                            uri_dbg(sink_filter),
                            e.code
                        ),
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    retries -= 1;
                    attempt += 1;
                }
                Err(e) => {
                    dbg_log(
                        "Router",
                        format!(
                            "register_cloud_listener(): mqtt registration failed attempt={} source={} sink={} code={:?} message={:?}",
                            attempt,
                            uri_dbg(source_filter),
                            uri_dbg(sink_filter),
                            e.code,
                            e.message
                        ),
                    );
                    return Err(e);
                }
            }
        }
    }

    async fn register_local_vsomeip_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        let Some(v) = self.vsomeip.as_ref() else {
            return Err(UStatus::fail_with_code(
                up_rust::UCode::UNAVAILABLE,
                "No vSomeIP transport available",
            ));
        };

        v.register_listener(source_filter, sink_filter, listener).await
    }

    async fn unregister_cloud_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        let Some(mqtt_tx) = self.mqtt.as_ref() else {
            return Err(UStatus::fail_with_code(
                up_rust::UCode::UNAVAILABLE,
                "MQTT transport not configured",
            ));
        };

        let sink_filter = sink_filter.ok_or_else(|| {
            UStatus::fail_with_code(
                up_rust::UCode::INVALID_ARGUMENT,
                "Cloud listener unregistration requires a sink filter",
            )
        })?;
        mqtt_tx
            .unregister_listener(source_filter, Some(sink_filter), listener)
            .await
    }

    async fn unregister_local_vsomeip_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        let Some(v) = self.vsomeip.as_ref() else {
            return Err(UStatus::fail_with_code(
                up_rust::UCode::UNAVAILABLE,
                "No vSomeIP transport available",
            ));
        };

        v.unregister_listener(source_filter, sink_filter, listener)
            .await
    }
}

#[async_trait]
impl UTransport for PacomRouter {
    async fn send(&self, message: UMessage) -> Result<(), UStatus> {
        if verbose_debug_enabled() {
            let source = message
                .attributes
                .source
                .as_ref()
                .map(|u| u.to_uri(false))
                .unwrap_or_else(|| "<none>".to_string());
            let sink = message
                .attributes
                .sink
                .as_ref()
                .map(|u| u.to_uri(false))
                .unwrap_or_else(|| "<none>".to_string());
            dbg_log(
                "Router",
                format!(
                    "send(): source={}, sink={}, has_payload={}",
                    source,
                    sink,
                    message.payload.is_some()
                ),
            );
        }

        if is_local_only_publish(&message) {
            // A Publish without a sink is a local event. Do not mirror it to
            // MQTT implicitly; missing vSomeIP must remain an explicit error.
            let mut success = false;
            let mut last_err = None;

            if let Some(ref v) = self.vsomeip {
                trace!("[Router] Broadcasting Publish to local vSomeIP transport");
                let mut vsomeip_msg = message;
                if let Some(source) = vsomeip_msg.attributes.source.as_ref().cloned() {
                    dbg_log(
                        "Router",
                        format!(
                            "PUBLISH_PATH raw source={} major={} resource={} wildcard_resource={}",
                            uri_dbg(&source),
                            source.ue_version_major,
                            source.resource_id,
                            is_wildcard_resource_id(source.resource_id)
                        ),
                    );

                    // Keep the complete uProtocol source URI. The vSomeIP
                    // transport maps only its numeric fields to SOME/IP.
                } else {
                    dbg_log(
                        "Router",
                        "PUBLISH_PATH source_missing; using original publish message on vSomeIP",
                    );
                }

                let final_source = vsomeip_msg
                    .attributes
                    .source
                    .as_ref()
                    .map(uri_dbg)
                    .unwrap_or_else(|| "<none>".to_string());
                dbg_log(
                    "Router",
                    format!(
                        "PUBLISH_PATH final_vsmsg source={} sink=<none> payload_len={}",
                        final_source,
                        vsomeip_msg.payload.as_ref().map(|p| p.len()).unwrap_or(0)
                    ),
                );

                match v.send(vsomeip_msg).await {
                    Ok(_) => {
                        success = true;
                        dbg_log("Router", "VSOMEIP_SEND_RESULT result=ok");
                    }
                    Err(e) => {
                        dbg_log(
                            "Router",
                            format!(
                                "VSOMEIP_SEND_RESULT result=err code={:?} message={:?}",
                                e.code, e.message
                            ),
                        );
                        if !success {
                            last_err = Some(e);
                        }
                    }
                }
            }

            if success {
                dbg_log("Router", "send(): local publish succeeded on vSomeIP");
                return Ok(());
            } else if let Some(e) = last_err {
                dbg_log(
                    "Router",
                    format!("send(): publish broadcast failed with code={:?}", e.code),
                );
                return Err(e);
            } else {
                return Err(UStatus::fail_with_code(
                    up_rust::UCode::UNAVAILABLE,
                    "No transport available",
                ));
            }
        }

        // Addressed message (Notification/RPC): route based on the sink authority.
        let sink = message.attributes.sink.as_ref().unwrap();
        let is_cloud = self.is_cloud_bound(sink);
        dbg_log(
            "Router",
            format!(
                "send(): addressed message cloud_bound={} for sink={}",
                is_cloud,
                sink.to_uri(false)
            ),
        );

        if is_cloud {
            if let Some(ref mqtt_tx) = self.mqtt {
                trace!("[Router] Routing cloud message to MQTT 5 transport");
                let out = mqtt_tx.send(message).await;
                match &out {
                    Ok(_) => dbg_log("Router", "send(): addressed->mqtt result=ok"),
                    Err(e) => dbg_log(
                        "Router",
                        format!(
                            "send(): addressed->mqtt result=err code={:?} message={:?}",
                            e.code, e.message
                        ),
                    ),
                }
                out
            } else {
                Err(UStatus::fail_with_code(
                    up_rust::UCode::UNAVAILABLE,
                    "Cloud-bound message cannot be sent: MQTT transport not configured",
                ))
            }
        } else {
            if let Some(ref v) = self.vsomeip {
                trace!("[Router] Routing message to local vSomeIP transport");
                let mut vsomeip_msg = message;

                let pre_source = vsomeip_msg
                    .attributes
                    .source
                    .as_ref()
                    .map(uri_dbg)
                    .unwrap_or_else(|| "<none>".to_string());
                let pre_sink = vsomeip_msg
                    .attributes
                    .sink
                    .as_ref()
                    .map(uri_dbg)
                    .unwrap_or_else(|| "<none>".to_string());
                rpc_diag_log(format!(
                    "addressed_local pre_send source={} sink={} payload_len={}",
                    pre_source,
                    pre_sink,
                    vsomeip_msg.payload.as_ref().map(|p| p.len()).unwrap_or(0)
                ));

                if let Some(attrs) = vsomeip_msg.attributes.as_mut() {
                    // Keep source and sink authorities intact. The vSomeIP
                    // backend ignores authority when building the SOME/IP IDs.

                    if let Some(sink) = attrs.sink.as_ref().cloned() {
                        let is_rpc_response_like = sink.resource_id == 0;
                        if is_rpc_response_like {
                            rpc_diag_log(format!(
                                "addressed_local response_sink_preserved sink={}",
                                uri_dbg(&sink)
                            ));
                        } else {
                            // Keep the complete sink URI for uProtocol-level
                            // identity while forwarding it to vSomeIP.
                        }
                    }
                }

                let post_source = vsomeip_msg
                    .attributes
                    .source
                    .as_ref()
                    .map(uri_dbg)
                    .unwrap_or_else(|| "<none>".to_string());
                let post_sink = vsomeip_msg
                    .attributes
                    .sink
                    .as_ref()
                    .map(uri_dbg)
                    .unwrap_or_else(|| "<none>".to_string());
                rpc_diag_log(format!(
                    "addressed_local post_rewrite source={} sink={}",
                    post_source, post_sink
                ));

                if rpc_diag_enabled() {
                    if let Some(attrs) = vsomeip_msg.attributes.as_ref() {
                        if let (Some(src), Some(snk)) = (attrs.source.as_ref(), attrs.sink.as_ref())
                        {
                            // RPC responses normally target the caller's reply sink (resource_id=0).
                            // If source and sink UE coincide, we may be routing the reply to self.
                            if snk.resource_id == 0 && src.ue_id == snk.ue_id {
                                rpc_diag_log(format!(
                                    "WARN addressed_local potential_self_routed_response source={} sink={}",
                                    uri_dbg(src),
                                    uri_dbg(snk)
                                ));
                            }
                        }
                    }
                }

                let out = v.send(vsomeip_msg).await;
                match &out {
                    Ok(_) => {
                        dbg_log("Router", "send(): addressed->vsomeip result=ok");
                        rpc_diag_log("addressed_local send_result=ok");
                    }
                    Err(e) => {
                        dbg_log(
                            "Router",
                            format!(
                                "send(): addressed->vsomeip result=err code={:?} message={:?}",
                                e.code, e.message
                            ),
                        );
                        rpc_diag_log(format!(
                            "addressed_local send_result=err code={:?} message={:?}",
                            e.code, e.message
                        ));
                    }
                }
                out
            } else {
                Err(UStatus::fail_with_code(
                    up_rust::UCode::UNAVAILABLE,
                    "No transport available",
                ))
            }
        }
    }

    async fn register_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        dbg_log(
            "Router",
            format!(
                "register_listener(): source_filter={}, sink_filter={}",
                source_filter.to_uri(false),
                sink_filter
                    .map(|u| u.to_uri(false))
                    .unwrap_or_else(|| "<none>".to_string())
            ),
        );

        let is_cloud = self.listener_cloud_path(source_filter, sink_filter);
        dbg_log(
            "Router",
            format!("register_listener(): cloud_path={}", is_cloud),
        );

        if is_cloud {
            self.register_cloud_listener(source_filter, sink_filter, listener)
                .await
        } else {
            self.register_local_vsomeip_listener(source_filter, sink_filter, listener)
                .await
        }
    }

    async fn unregister_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        dbg_log(
            "Router",
            format!(
                "unregister_listener(): source_filter={}, sink_filter={}",
                source_filter.to_uri(false),
                sink_filter
                    .map(|u| u.to_uri(false))
                    .unwrap_or_else(|| "<none>".to_string())
            ),
        );

        let is_cloud = self.listener_cloud_path(source_filter, sink_filter);

        if is_cloud {
            self.unregister_cloud_listener(source_filter, sink_filter, listener)
                .await
        } else {
            self.unregister_local_vsomeip_listener(source_filter, sink_filter, listener)
                .await
        }
    }

    async fn receive(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
    ) -> Result<UMessage, UStatus> {
        dbg_log(
            "Router",
            format!(
                "receive(): source_filter={} sink_filter={}",
                uri_dbg(source_filter),
                sink_filter
                    .map(uri_dbg)
                    .unwrap_or_else(|| "<none>".to_string())
            ),
        );
        let is_cloud = self.listener_cloud_path(source_filter, sink_filter);
        if is_cloud {
            if let Some(ref mqtt_tx) = self.mqtt {
                mqtt_tx.receive(source_filter, sink_filter).await
            } else {
                Err(UStatus::fail_with_code(
                    up_rust::UCode::UNAVAILABLE,
                    "Cloud-bound receive requires MQTT transport",
                ))
            }
        } else if let Some(ref v) = self.vsomeip {
            let out = v.receive(source_filter, sink_filter).await;
            match &out {
                Ok(msg) => {
                    let src = msg
                        .attributes
                        .source
                        .as_ref()
                        .map(uri_dbg)
                        .unwrap_or_else(|| "<none>".to_string());
                    let sink = msg
                        .attributes
                        .sink
                        .as_ref()
                        .map(uri_dbg)
                        .unwrap_or_else(|| "<none>".to_string());
                    dbg_log(
                        "Router",
                        format!(
                            "receive(): message received source={} sink={} payload_len={}",
                            src,
                            sink,
                            msg.payload.as_ref().map(|p| p.len()).unwrap_or(0)
                        ),
                    );
                }
                Err(e) => dbg_log(
                    "Router",
                    format!(
                        "receive(): failed code={:?} message={:?}",
                        e.code, e.message
                    ),
                ),
            }
            out
        } else {
            Err(UStatus::fail_with_code(
                up_rust::UCode::UNAVAILABLE,
                "Local receive requires vSomeIP transport",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use up_rust::{UMessageBuilder, UPayloadFormat};

    #[test]
    fn local_publish_without_sink_is_local_only() {
        let source = UUri::try_from_parts("ecu-a", 0x1234, 1, 0x9449).unwrap();
        let msg = UMessageBuilder::publish(source)
            .build_with_payload(vec![1, 2, 3], UPayloadFormat::UPAYLOAD_FORMAT_RAW)
            .unwrap();

        assert!(is_local_only_publish(&msg));
    }

    #[test]
    fn notification_with_sink_is_not_local_only_publish() {
        let source = UUri::try_from_parts("ecu-a", 0x1234, 1, 0x2222).unwrap();
        let sink = UUri::try_from_parts("cloud.bridge", 0, 0, 0).unwrap();
        let msg = UMessageBuilder::notification(source, sink)
            .build_with_payload(vec![1, 2, 3], UPayloadFormat::UPAYLOAD_FORMAT_RAW)
            .unwrap();

        assert!(!is_local_only_publish(&msg));
    }
}

impl up_rust::LocalUriProvider for PacomRouter {
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
