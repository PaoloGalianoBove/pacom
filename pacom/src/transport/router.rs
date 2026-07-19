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

fn verbose_debug_enabled() -> bool {
    std::env::var("PACOM_DEBUG_VERBOSE")
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes" || v == "on"
        })
        .unwrap_or(false)
}

fn dbg_log(msg: impl AsRef<str>) {
    if verbose_debug_enabled() {
        println!("[PACOM-DBG][Router] {}", msg.as_ref());
    }
}

fn rpc_diag_enabled() -> bool {
    std::env::var("PACOM_RPC_DIAGNOSTICS")
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes" || v == "on"
        })
        .unwrap_or(false)
}

fn rpc_diag_log(msg: impl AsRef<str>) {
    if rpc_diag_enabled() {
        println!("[PACOM-RPC-DIAG][Router] {}", msg.as_ref());
    }
}

fn uri_dbg(uri: &UUri) -> String {
    format!(
        "{} [auth='{}', ue=0x{:08X}, ver={}, res=0x{:08X}]",
        uri.to_uri(false),
        uri.authority_name(),
        uri.ue_id,
        uri.ue_version_major,
        uri.resource_id
    )
}

fn is_mqtt_wildcard_ue_id(ue_id: u32) -> bool {
    ue_id == 0xFFFF || ue_id == u32::MAX
}

fn is_wildcard_resource_id(resource_id: u32) -> bool {
    resource_id == 0xFFFF || resource_id == u32::MAX
}

fn is_wildcard_major_version(major: u32) -> bool {
    major == 0xFF
}

fn is_wildcard_source_filter(uri: &UUri) -> bool {
    let auth = uri.authority_name();
    (auth.is_empty() || auth == "*") && (is_mqtt_wildcard_ue_id(uri.ue_id) || uri.ue_id == 0)
}

fn is_local_only_publish(message: &UMessage) -> bool {
    message.attributes.sink.is_none()
}

fn normalize_uri_for_vsomeip(uri: &UUri) -> UUri {
    UUri::try_from_parts(
        "",
        uri.ue_id,
        uri.ue_version_major as u8,
        uri.resource_id as u16,
    )
    .unwrap_or_else(|_| uri.clone())
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
    /// The rule is general: any destination with an authority different from the local
    /// node's authority is considered cross-domain. If the local node itself has no
    /// vSomeIP transport (e.g. a pure cloud gateway), all traffic is cross-domain.
    pub fn is_cloud_bound(&self, uri: &UUri) -> bool {
        let target_auth = uri.authority_name();
        // Empty or wildcard authority means local (broadcast on vSomeIP)
        if target_auth.is_empty() || target_auth == "*" {
            dbg_log(format!(
                "is_cloud_bound=false reason=empty_or_wildcard_authority uri={}",
                uri_dbg(uri)
            ));
            return false;
        }
        // If we have no vSomeIP, every message is cloud-bound (MQTT-only node)
        if self.vsomeip.is_none() {
            dbg_log(format!(
                "is_cloud_bound=true reason=no_vsomeip_transport uri={}",
                uri_dbg(uri)
            ));
            return true;
        }

        // Explicit cross-domain marker used by publish_to_authority(): //authority/0/0/0
        if uri.ue_id == 0 && uri.ue_version_major == 0 && uri.resource_id == 0 {
            dbg_log(format!(
                "is_cloud_bound=true reason=explicit_cross_domain_marker uri={}",
                uri_dbg(uri)
            ));
            return true;
        }

        // MQTT authority-level wildcard subscriptions can be represented either as
        // 16-bit wildcard (0xFFFF) or full-width wildcard (0xFFFFFFFF).
        if is_mqtt_wildcard_ue_id(uri.ue_id) {
            dbg_log(format!(
                "is_cloud_bound=true reason=mqtt_wildcard_ue_id uri={}",
                uri_dbg(uri)
            ));
            return true;
        }

        // Otherwise keep addressed traffic on vSomeIP even across different authorities.
        dbg_log(format!(
            "is_cloud_bound=false reason=default_local_vsomeip uri={}",
            uri_dbg(uri)
        ));
        false
    }

    fn listener_cloud_path(&self, source_filter: &UUri, sink_filter: Option<&UUri>) -> bool {
        // If a local sink is explicitly provided, this listener is intended for local routing.
        if let Some(sink) = sink_filter {
            if !self.is_cloud_bound(sink) {
                dbg_log(format!(
                    "listener_cloud_path=false reason=explicit_local_sink source={} sink={}",
                    uri_dbg(source_filter),
                    uri_dbg(sink)
                ));
                return false;
            }
        }

        let decision = self.is_cloud_bound(source_filter)
            || sink_filter.map(|s| self.is_cloud_bound(s)).unwrap_or(false);
        dbg_log(format!(
            "listener_cloud_path={} source={} sink={}",
            decision,
            uri_dbg(source_filter),
            sink_filter
                .map(uri_dbg)
                .unwrap_or_else(|| "<none>".to_string())
        ));
        decision
    }

    fn normalized_local_source_filter(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
    ) -> UUri {
        let Some(sink) = sink_filter else {
            return source_filter.clone();
        };

        // When source is a wildcard and sink targets a local endpoint, force a
        // deterministic local source filter to avoid wildcard-derived SOME/IP
        // offers like instance/version=255 that can conflict with the real offer.
        if is_wildcard_source_filter(source_filter) && !self.is_cloud_bound(sink) {
            let resource = if is_wildcard_resource_id(source_filter.resource_id) {
                sink.resource_id as u16
            } else {
                source_filter.resource_id as u16
            };

            if let Ok(uri) =
                UUri::try_from_parts("", sink.ue_id, sink.ue_version_major as u8, resource)
            {
                dbg_log(format!(
                    "normalized_local_source_filter: source={} sink={} normalized={}",
                    uri_dbg(source_filter),
                    uri_dbg(sink),
                    uri_dbg(&uri)
                ));
                return uri;
            }
        }

        source_filter.clone()
    }

    fn should_skip_local_vsomeip_catchall(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
    ) -> bool {
        let Some(sink) = sink_filter else {
            return false;
        };

        // This pattern is used by higher layers as a generic "catch-all" listener,
        // but on SOME/IP it may translate into wildcard service offers that conflict
        // with concrete service registrations (observed as ...255... conflicts).
        is_wildcard_source_filter(source_filter)
            && !self.is_cloud_bound(sink)
            && is_wildcard_resource_id(sink.resource_id)
    }

    fn should_skip_local_vsomeip_candidate(
        &self,
        candidate: &UUri,
        sink_filter: Option<&UUri>,
    ) -> bool {
        let Some(sink) = sink_filter else {
            return false;
        };

        !self.is_cloud_bound(sink)
            && (is_mqtt_wildcard_ue_id(candidate.ue_id)
                || is_wildcard_major_version(candidate.ue_version_major)
                || is_wildcard_resource_id(candidate.resource_id))
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
            dbg_log(format!(
                "send(): source={}, sink={}, has_payload={}",
                source,
                sink,
                message.payload.is_some()
            ));
        }

        if is_local_only_publish(&message) {
            // It's a Publish message. Broadcast to all available transports.
            let mut success = false;
            let mut last_err = None;

            if let Some(ref mqtt_tx) = self.mqtt {
                // Off-Vehicle MQTT routing requires a sink URI. Sink-less Publish is local
                // fan-out semantics, so we intentionally skip MQTT for this branch.
                let _ = mqtt_tx;
                dbg_log("send(): publish->mqtt skipped (sink missing; local publish only)");
            }

            if let Some(ref v) = self.vsomeip {
                trace!("[Router] Broadcasting Publish to local vSomeIP transport");
                let mut vsomeip_msg = message;
                if let Some(source) = vsomeip_msg.attributes.source.as_ref().cloned() {
                    dbg_log(format!(
                        "PUBLISH_PATH raw source={} major={} resource={} wildcard_major={} wildcard_resource={}",
                        uri_dbg(&source),
                        source.ue_version_major,
                        source.resource_id,
                        is_wildcard_major_version(source.ue_version_major),
                        is_wildcard_resource_id(source.resource_id)
                    ));

                    // Keep publish semantics unchanged. Normalize only authority for local vSomeIP.
                    let normalized_source = UUri::try_from_parts(
                        "",
                        source.ue_id,
                        source.ue_version_major as u8,
                        source.resource_id as u16,
                    )
                    .unwrap_or_else(|_| source.clone());

                    if normalized_source.to_uri(false) != source.to_uri(false) {
                        dbg_log(format!(
                            "VSOMEIP_REWRITE source={} normalized_source={}",
                            uri_dbg(&source),
                            uri_dbg(&normalized_source)
                        ));
                    }

                    if let Some(attrs) = vsomeip_msg.attributes.as_mut() {
                        attrs.source = Some(normalized_source).into();
                    }
                } else {
                    dbg_log(
                        "PUBLISH_PATH source_missing; using original publish message on vSomeIP",
                    );
                }

                let final_source = vsomeip_msg
                    .attributes
                    .source
                    .as_ref()
                    .map(uri_dbg)
                    .unwrap_or_else(|| "<none>".to_string());
                dbg_log(format!(
                    "PUBLISH_PATH final_vsmsg source={} sink=<none> payload_len={}",
                    final_source,
                    vsomeip_msg.payload.as_ref().map(|p| p.len()).unwrap_or(0)
                ));

                match v.send(vsomeip_msg).await {
                    Ok(_) => {
                        success = true;
                        dbg_log("VSOMEIP_SEND_RESULT result=ok");
                    }
                    Err(e) => {
                        dbg_log(format!(
                            "VSOMEIP_SEND_RESULT result=err code={:?} message={:?}",
                            e.code, e.message
                        ));
                        if !success {
                            last_err = Some(e);
                        }
                    }
                }
            }

            if success {
                dbg_log("send(): publish broadcast succeeded on at least one transport");
                return Ok(());
            } else if let Some(e) = last_err {
                dbg_log(format!(
                    "send(): publish broadcast failed with code={:?}",
                    e.code
                ));
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
        dbg_log(format!(
            "send(): addressed message cloud_bound={} for sink={}",
            is_cloud,
            sink.to_uri(false)
        ));

        if is_cloud {
            if let Some(ref mqtt_tx) = self.mqtt {
                trace!("[Router] Routing cloud message to MQTT 5 transport");
                let out = mqtt_tx.send(message).await;
                match &out {
                    Ok(_) => dbg_log("send(): addressed->mqtt result=ok"),
                    Err(e) => dbg_log(format!(
                        "send(): addressed->mqtt result=err code={:?} message={:?}",
                        e.code, e.message
                    )),
                }
                out
            } else {
                // MQTT not configured: skip cloud-bound send gracefully.
                info!("[Router] Cloud send skipped: MQTT transport not configured");
                Ok(())
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
                    // For RPC requests on local vSomeIP, normalize only source authority.
                    // Keep UE/version/resource untouched so caller identity is preserved.
                    if let (Some(source), Some(sink)) =
                        (attrs.source.as_ref().cloned(), attrs.sink.as_ref().cloned())
                    {
                        let is_rpc_request_like = source.resource_id == 0 && sink.resource_id != 0;
                        if is_rpc_request_like {
                            if let Ok(normalized_source) = UUri::try_from_parts(
                                "",
                                source.ue_id,
                                source.ue_version_major as u8,
                                source.resource_id as u16,
                            ) {
                                if normalized_source.to_uri(false) != source.to_uri(false) {
                                    rpc_diag_log(format!(
                                        "addressed_local request_source_normalized old={} new={}",
                                        uri_dbg(&source),
                                        uri_dbg(&normalized_source)
                                    ));
                                }
                                attrs.source = Some(normalized_source).into();
                            }
                        }
                    }

                    if let Some(sink) = attrs.sink.as_ref().cloned() {
                        let is_rpc_response_like = sink.resource_id == 0;
                        if is_rpc_response_like {
                            rpc_diag_log(format!(
                                "addressed_local response_sink_preserved sink={}",
                                uri_dbg(&sink)
                            ));
                        } else {
                            let normalized = normalize_uri_for_vsomeip(&sink);
                            if normalized.to_uri(false) != sink.to_uri(false) {
                                dbg_log(format!(
                                    "VSOMEIP_REWRITE addressed sink={} normalized_sink={}",
                                    uri_dbg(&sink),
                                    uri_dbg(&normalized)
                                ));
                            }
                            attrs.sink = Some(normalized).into();
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
                        if let (Some(src), Some(snk)) = (attrs.source.as_ref(), attrs.sink.as_ref()) {
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
                        dbg_log("send(): addressed->vsomeip result=ok");
                        rpc_diag_log("addressed_local send_result=ok");
                    }
                    Err(e) => {
                        dbg_log(format!(
                            "send(): addressed->vsomeip result=err code={:?} message={:?}",
                            e.code, e.message
                        ));
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
        dbg_log(format!(
            "register_listener(): source_filter={}, sink_filter={}",
            source_filter.to_uri(false),
            sink_filter
                .map(|u| u.to_uri(false))
                .unwrap_or_else(|| "<none>".to_string())
        ));

        let is_cloud = self.listener_cloud_path(source_filter, sink_filter);
        dbg_log(format!("register_listener(): cloud_path={}", is_cloud));

        let mut success = false;
        let mut last_err = None;

        if is_cloud {
            if let Some(ref mqtt_tx) = self.mqtt {
                let default_sink =
                    UUri::try_from_parts(&self.authority, 0xFFFF, 0xFF, 0xFFFF).unwrap();
                let effective_sink = Some(sink_filter.unwrap_or(&default_sink));
                let mut retries = 50;
                let mut attempt = 1;
                loop {
                    match mqtt_tx
                        .register_listener(source_filter, effective_sink, listener.clone())
                        .await
                    {
                        Ok(_) => {
                            success = true;
                            dbg_log(format!(
                                "register_listener(): mqtt registration succeeded attempts={} source={} sink={}",
                                attempt,
                                uri_dbg(source_filter),
                                effective_sink
                                    .map(uri_dbg)
                                    .unwrap_or_else(|| "<none>".to_string())
                            ));
                            break;
                        }
                        Err(e)
                            if e.code.enum_value_or_default() == up_rust::UCode::UNAVAILABLE
                                && retries > 0 =>
                        {
                            dbg_log(format!(
                                "register_listener(): mqtt unavailable retry attempt={} remaining={} source={} sink={} code={:?}",
                                attempt,
                                retries,
                                uri_dbg(source_filter),
                                effective_sink
                                    .map(uri_dbg)
                                    .unwrap_or_else(|| "<none>".to_string()),
                                e.code
                            ));
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            retries -= 1;
                            attempt += 1;
                        }
                        Err(e) => {
                            dbg_log(format!(
                                "register_listener(): mqtt registration failed attempt={} source={} sink={} code={:?} message={:?}",
                                attempt,
                                uri_dbg(source_filter),
                                effective_sink
                                    .map(uri_dbg)
                                    .unwrap_or_else(|| "<none>".to_string()),
                                e.code,
                                e.message
                            ));
                            last_err = Some(e);
                            break;
                        }
                    }
                }
            } else {
                info!(
                    "[Router] Cloud listener registration skipped: MQTT transport not configured"
                );
            }
        }

        // Register on local vSomeIP only for non-cloud filters.
        if !is_cloud {
            if let Some(ref v) = self.vsomeip {
                if let Some(sink) = sink_filter {
                    if sink.resource_id == 0 {
                        match v
                            .register_listener(source_filter, sink_filter, listener.clone())
                            .await
                        {
                            Ok(_) => {
                                dbg_log(format!(
                                    "register_listener(): vsomeip RPC response listener registered source={} sink={}",
                                    uri_dbg(source_filter),
                                    uri_dbg(sink)
                                ));
                                return Ok(());
                            }
                            Err(e) => {
                                dbg_log(format!(
                                    "register_listener(): vsomeip RPC response listener registration failed: {:?}",
                                    e
                                ));
                                return Err(e);
                            }
                        }
                    }
                }
                if self.should_skip_local_vsomeip_catchall(source_filter, sink_filter) {
                    dbg_log(format!(
                        "register_listener(): skipping local vSomeIP catch-all source={} sink={}",
                        uri_dbg(source_filter),
                        sink_filter
                            .map(uri_dbg)
                            .unwrap_or_else(|| "<none>".to_string())
                    ));
                    success = true;
                } else {
                    let normalized_source =
                        self.normalized_local_source_filter(source_filter, sink_filter);
                    let entity_id = normalized_source.ue_id;
                    let version_major = normalized_source.ue_version_major as u8;
                    let resource_id = normalized_source.resource_id;

                    let filter_local = UUri::try_from_parts(
                        &self.authority,
                        entity_id,
                        version_major,
                        resource_id as u16,
                    )
                    .unwrap_or_else(|_| normalized_source.clone());
                    let filter_empty =
                        UUri::try_from_parts("", entity_id, version_major, resource_id as u16)
                            .unwrap_or_else(|_| normalized_source.clone());

                    // vSomeIP messages do not carry authorities. For remote authorities, prefer empty/local filters first.
                    let source_auth = normalized_source.authority_name();
                    let mut candidates: Vec<UUri> = if !source_auth.is_empty()
                        && source_auth != "*"
                        && source_auth != self.authority
                    {
                        vec![
                            filter_empty.clone(),
                            filter_local.clone(),
                            normalized_source.clone(),
                        ]
                    } else {
                        vec![
                            normalized_source.clone(),
                            filter_local.clone(),
                            filter_empty.clone(),
                        ]
                    };

                    // Deduplicate candidate filters by their URI representation.
                    candidates.dedup_by(|a, b| a.to_uri(false) == b.to_uri(false));
                    let candidate_list = candidates
                        .iter()
                        .map(uri_dbg)
                        .collect::<Vec<_>>()
                        .join(" | ");
                    dbg_log(format!(
                        "register_listener(): vsomeip candidates={}",
                        candidate_list
                    ));

                    for (idx, candidate) in candidates.into_iter().enumerate() {
                        if self.should_skip_local_vsomeip_candidate(&candidate, sink_filter) {
                            dbg_log(format!(
                                "register_listener(): skipping wildcard local vSomeIP candidate candidate_index={} filter={} sink={}",
                                idx,
                                uri_dbg(&candidate),
                                sink_filter
                                    .map(uri_dbg)
                                    .unwrap_or_else(|| "<none>".to_string())
                            ));
                            continue;
                        }

                        match v
                            .register_listener(&candidate, sink_filter, listener.clone())
                            .await
                        {
                            Ok(_) => {
                                success = true;
                                dbg_log(format!(
                                    "register_listener(): vSomeIP listener registration succeeded candidate_index={} filter={} sink={}",
                                    idx,
                                    uri_dbg(&candidate),
                                    sink_filter
                                        .map(uri_dbg)
                                        .unwrap_or_else(|| "<none>".to_string())
                                ));
                                break;
                            }
                            Err(e) => {
                                dbg_log(format!(
                                    "register_listener(): vSomeIP candidate failed candidate_index={} filter={} sink={} code={:?} message={:?}",
                                    idx,
                                    uri_dbg(&candidate),
                                    sink_filter
                                        .map(uri_dbg)
                                        .unwrap_or_else(|| "<none>".to_string()),
                                    e.code,
                                    e.message
                                ));
                                last_err = Some(e);
                            }
                        }
                    }

                    if !success {
                        dbg_log(
                            "register_listener(): vSomeIP listener registration failed on all filter variants",
                        );
                    }
                }
            }
        }

        if success {
            Ok(())
        } else {
            Err(last_err.unwrap_or_else(|| {
                UStatus::fail_with_code(up_rust::UCode::UNAVAILABLE, "No transport available")
            }))
        }
    }

    async fn unregister_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        dbg_log(format!(
            "unregister_listener(): source_filter={}, sink_filter={}",
            source_filter.to_uri(false),
            sink_filter
                .map(|u| u.to_uri(false))
                .unwrap_or_else(|| "<none>".to_string())
        ));

        let is_cloud = self.listener_cloud_path(source_filter, sink_filter);

        let mut success = false;
        let mut last_err = None;

        if is_cloud {
            if let Some(ref mqtt_tx) = self.mqtt {
                let default_sink =
                    UUri::try_from_parts(&self.authority, 0xFFFF, 0xFF, 0xFFFF).unwrap();
                let effective_sink = Some(sink_filter.unwrap_or(&default_sink));
                match mqtt_tx
                    .unregister_listener(source_filter, effective_sink, listener.clone())
                    .await
                {
                    Ok(_) => {
                        success = true;
                        dbg_log(format!(
                            "unregister_listener(): mqtt unregister succeeded source={} sink={}",
                            uri_dbg(source_filter),
                            effective_sink
                                .map(uri_dbg)
                                .unwrap_or_else(|| "<none>".to_string())
                        ));
                    }
                    Err(e) => {
                        dbg_log(format!(
                            "unregister_listener(): mqtt unregister failed source={} sink={} code={:?} message={:?}",
                            uri_dbg(source_filter),
                            effective_sink
                                .map(uri_dbg)
                                .unwrap_or_else(|| "<none>".to_string()),
                            e.code,
                            e.message
                        ));
                        last_err = Some(e)
                    }
                }
            }
        }

        if !is_cloud {
            if let Some(ref v) = self.vsomeip {
                if let Some(sink) = sink_filter {
                    if sink.resource_id == 0 {
                        match v
                            .unregister_listener(source_filter, sink_filter, listener.clone())
                            .await
                        {
                            Ok(_) => {
                                dbg_log(format!(
                                    "unregister_listener(): vsomeip RPC response listener unregistered source={} sink={}",
                                    uri_dbg(source_filter),
                                    uri_dbg(sink)
                                ));
                                return Ok(());
                            }
                            Err(e) => {
                                dbg_log(format!(
                                    "unregister_listener(): vsomeip RPC response listener unregistration failed: {:?}",
                                    e
                                ));
                                return Err(e);
                            }
                        }
                    }
                }
                if self.should_skip_local_vsomeip_catchall(source_filter, sink_filter) {
                    dbg_log(format!(
                        "unregister_listener(): skipping local vSomeIP catch-all source={} sink={}",
                        uri_dbg(source_filter),
                        sink_filter
                            .map(uri_dbg)
                            .unwrap_or_else(|| "<none>".to_string())
                    ));
                    success = true;
                } else {
                    let normalized_source =
                        self.normalized_local_source_filter(source_filter, sink_filter);
                    let entity_id = normalized_source.ue_id;
                    let version_major = normalized_source.ue_version_major as u8;
                    let resource_id = normalized_source.resource_id;

                    let filter_local = UUri::try_from_parts(
                        &self.authority,
                        entity_id,
                        version_major,
                        resource_id as u16,
                    )
                    .unwrap_or_else(|_| normalized_source.clone());
                    let filter_empty =
                        UUri::try_from_parts("", entity_id, version_major, resource_id as u16)
                            .unwrap_or_else(|_| normalized_source.clone());

                    let source_auth = normalized_source.authority_name();
                    let mut candidates: Vec<UUri> = if !source_auth.is_empty()
                        && source_auth != "*"
                        && source_auth != self.authority
                    {
                        vec![
                            filter_empty.clone(),
                            filter_local.clone(),
                            normalized_source.clone(),
                        ]
                    } else {
                        vec![
                            normalized_source.clone(),
                            filter_local.clone(),
                            filter_empty.clone(),
                        ]
                    };
                    candidates.dedup_by(|a, b| a.to_uri(false) == b.to_uri(false));
                    let candidate_list = candidates
                        .iter()
                        .map(uri_dbg)
                        .collect::<Vec<_>>()
                        .join(" | ");
                    dbg_log(format!(
                        "unregister_listener(): vsomeip candidates={}",
                        candidate_list
                    ));

                    for (idx, candidate) in candidates.into_iter().enumerate() {
                        if self.should_skip_local_vsomeip_candidate(&candidate, sink_filter) {
                            dbg_log(format!(
                                "unregister_listener(): skipping wildcard local vSomeIP candidate candidate_index={} filter={} sink={}",
                                idx,
                                uri_dbg(&candidate),
                                sink_filter
                                    .map(uri_dbg)
                                    .unwrap_or_else(|| "<none>".to_string())
                            ));
                            continue;
                        }

                        match v
                            .unregister_listener(&candidate, sink_filter, listener.clone())
                            .await
                        {
                            Ok(_) => {
                                success = true;
                                dbg_log(format!(
                                    "unregister_listener(): vSomeIP unregister succeeded candidate_index={} filter={} sink={}",
                                    idx,
                                    uri_dbg(&candidate),
                                    sink_filter
                                        .map(uri_dbg)
                                        .unwrap_or_else(|| "<none>".to_string())
                                ));
                                break;
                            }
                            Err(e) => {
                                dbg_log(format!(
                                    "unregister_listener(): vSomeIP candidate failed candidate_index={} filter={} sink={} code={:?} message={:?}",
                                    idx,
                                    uri_dbg(&candidate),
                                    sink_filter
                                        .map(uri_dbg)
                                        .unwrap_or_else(|| "<none>".to_string()),
                                    e.code,
                                    e.message
                                ));
                                last_err = Some(e);
                            }
                        }
                    }
                }
            }
        }

        if success {
            Ok(())
        } else {
            Err(last_err.unwrap_or_else(|| {
                UStatus::fail_with_code(up_rust::UCode::UNAVAILABLE, "No transport available")
            }))
        }
    }

    async fn receive(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
    ) -> Result<UMessage, UStatus> {
        dbg_log(format!(
            "receive(): source_filter={} sink_filter={}",
            uri_dbg(source_filter),
            sink_filter
                .map(uri_dbg)
                .unwrap_or_else(|| "<none>".to_string())
        ));
        if let Some(ref v) = self.vsomeip {
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
                    dbg_log(format!(
                        "receive(): message received source={} sink={} payload_len={}",
                        src,
                        sink,
                        msg.payload.as_ref().map(|p| p.len()).unwrap_or(0)
                    ));
                }
                Err(e) => dbg_log(format!(
                    "receive(): failed code={:?} message={:?}",
                    e.code, e.message
                )),
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
