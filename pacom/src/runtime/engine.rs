use super::logical_registry::ManifestConfig;
use crate::error::PacomError;
use crate::transport::{mqtt, router::PacomRouter, vsomeip};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use up_rust::communication::{
    CallOptions, InMemoryRpcClient, InMemoryRpcServer, RequestHandler, RpcClient, RpcServer,
    ServiceInvocationError, UPayload,
};
use up_rust::{
    LocalUriProvider, UAttributes, UListener, UMessage, UMessageBuilder, UPayloadFormat, UStatus,
    UTransport, UUri,
};

use std::sync::Mutex;

const DISCOVERY_UE_ID: u16 = 0x0F00;
const DISCOVERY_RESOURCE_ID: u16 = 0x8F01;

use crate::utils::{dbg_log, uri_dbg, verbose_debug_enabled};

fn payload_preview(bytes: &[u8], max: usize) -> String {
    bytes
        .iter()
        .take(max)
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_cloud_topic(name: &str) -> bool {
    name.trim().starts_with("/cloud/")
}

pub(crate) fn cloud_authority_name() -> Result<String, PacomError> {
    std::env::var("PACOM_CLOUD_AUTHORITY")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| PacomError::Config("PACOM_CLOUD_AUTHORITY environment variable must be set for cloud operations".to_string()))
}

fn cloud_wildcard_source_uri(resource_id: u16) -> Result<UUri, PacomError> {
    UUri::try_from_parts("*", 0xFFFF, 1, resource_id)
        .map_err(|e| PacomError::Config(format!("Invalid cloud source URI: {e:?}")))
}

fn cloud_sink_marker_uri(authority: &str) -> Result<UUri, PacomError> {
    UUri::try_from_parts(authority, 0, 0, 0)
        .map_err(|e| PacomError::Config(format!("Invalid cloud sink URI: {e:?}")))
}

fn default_protocol_version() -> u16 {
    1
}

#[derive(Clone, Debug)]
struct ProviderInfo {
    authority: String,
    ue_id: u16,
    major_version: u8,
    resource_id: u16,
}

fn provider_dbg(p: &ProviderInfo) -> String {
    format!(
        "authority='{}' ue=0x{:04X} major={} resource=0x{:04X}",
        p.authority, p.ue_id, p.major_version, p.resource_id
    )
}

#[derive(Default)]
struct DiscoveryCache {
    rpc_providers: HashMap<String, ProviderInfo>,
    topic_publishers: HashMap<String, ProviderInfo>,
}

#[derive(Default)]
struct ProvidedCapabilities {
    rpc_services: HashSet<String>,
    published_topics: HashSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiscoveryEvent {
    kind: String,
    name: String,
    provider_ue_id: u16,
    major_version: u8,
    provider_authority: String,
    #[serde(default = "default_protocol_version")]
    protocol_version: u16,
    resource_id: u16,
}

/// A pending subscription waiting for the publisher to announce itself via Discovery.
struct PendingSubscription {
    listener: Arc<dyn UListener>,
    expected_resource_id: Option<Arc<std::sync::atomic::AtomicU16>>,
    source_authority: Option<String>,
}

struct DiscoveryListener {
    cache: Arc<RwLock<DiscoveryCache>>,
    router: Arc<PacomRouter>,
    pending_subs: Arc<Mutex<HashMap<String, Vec<PendingSubscription>>>>,
}

#[async_trait]
impl UListener for DiscoveryListener {
    async fn on_receive(&self, message: UMessage) {
        let source_uri_dbg = message
            .attributes
            .source
            .as_ref()
            .map(uri_dbg)
            .unwrap_or_else(|| "<none>".to_string());
        let sink_uri_dbg = message
            .attributes
            .sink
            .as_ref()
            .map(uri_dbg)
            .unwrap_or_else(|| "<none>".to_string());
        let source_authority = message
            .attributes
            .source
            .as_ref()
            .map(|s| s.authority_name().to_string())
            .unwrap_or_default();

        dbg_log("Runtime",format!(
            "DiscoveryListener on_receive: source={} sink={} payload_len={}",
            source_uri_dbg,
            sink_uri_dbg,
            message.payload.as_ref().map(|p| p.len()).unwrap_or(0)
        ));

        if let Some(payload) = message.payload {
            if let Ok(event) = serde_json::from_slice::<DiscoveryEvent>(&payload) {
                dbg_log("Runtime",format!(
                    "Discovery event received: kind='{}', name='{}', provider_ue=0x{:04x}, envelope_authority='{}', provider_authority='{}'",
                    event.kind,
                    event.name,
                    event.provider_ue_id,
                    source_authority,
                    event.provider_authority
                ));

                let effective_authority = if event.provider_authority.is_empty() {
                    source_authority.clone()
                } else {
                    event.provider_authority.clone()
                };

                let provider = ProviderInfo {
                    ue_id: event.provider_ue_id,
                    authority: effective_authority.clone(),
                    major_version: event.major_version,
                    resource_id: event.resource_id,
                };

                match event.kind.as_str() {
                    "rpc_provide" => {
                        if let Ok(mut cache) = self.cache.write() {
                            cache
                                .rpc_providers
                                .insert(event.name.clone(), provider.clone());
                            dbg_log("Runtime",format!(
                                "Discovery cache update: rpc_providers size={} latest name='{}' provider={}",
                                cache.rpc_providers.len(),
                                event.name,
                                provider_dbg(&provider)
                            ));
                        }
                    }
                    "topic_publish" => {
                        dbg_log("Runtime",format!(
                            "DiscoveryListener: received topic_publish name='{}' provider_ue=0x{:04X}",
                            event.name, event.provider_ue_id
                        ));
                        if let Ok(mut cache) = self.cache.write() {
                            cache
                                .topic_publishers
                                .insert(event.name.clone(), provider.clone());
                            dbg_log("Runtime",format!(
                                "Discovery cache update: topic_publishers size={} latest name='{}' provider={}",
                                cache.topic_publishers.len(),
                                event.name,
                                provider_dbg(&provider)
                            ));
                        }

                        // Reactively trigger any pending subscriptions for this topic.
                        // This ensures register_listener is called at the moment the publisher
                        // is first announced — before vSomeIP marks the service as "already available",
                        // allowing the subscription to be placed correctly.
                        let pending = {
                            if let Ok(mut map) = self.pending_subs.lock() {
                                let out = map.remove(&event.name);
                                dbg_log("Runtime",format!(
                                    "Pending map lookup: topic='{}' found={} remaining_topics={}",
                                    event.name,
                                    out.as_ref().map(|v| !v.is_empty()).unwrap_or(false),
                                    map.len()
                                ));
                                out
                            } else {
                                None
                            }
                        };

                        if let Some(subs) = pending {
                            dbg_log("Runtime",format!(
                                "Pending subscriptions found for '{}': count={}",
                                event.name,
                                subs.len()
                            ));
                            let mut still_pending: Vec<PendingSubscription> = Vec::new();

                            for sub in subs {
                                if let Some(ref wanted_authority) = sub.source_authority {
                                    if wanted_authority != &effective_authority {
                                        still_pending.push(sub);
                                        continue;
                                    }
                                }

                                if let Ok(uri) = UUri::try_from_parts(
                                    &effective_authority,
                                    provider.ue_id as u32,
                                    event.major_version,
                                    event.resource_id,
                                ) {
                                    if let Some(atomic_id) = &sub.expected_resource_id {
                                        atomic_id.store(event.resource_id, std::sync::atomic::Ordering::Relaxed);
                                    }
                                    dbg_log("Runtime",format!(
                                        "DiscoveryListener: activating pending listener topic='{}' uri='{}'",
                                        event.name,
                                        uri.to_uri(false)
                                    ));
                                    dbg_log("Runtime",format!(
                                        "Pending subscription activate: topic='{}' resource_id=0x{:04X} uri={}",
                                        event.name,
                                        event.resource_id,
                                        uri_dbg(&uri)
                                    ));
                                    match self
                                        .router
                                        .register_listener(&uri, None, sub.listener)
                                        .await
                                    {
                                        Ok(_) => dbg_log("Runtime",format!(
                                            "register_listener ok for '{}' on {}",
                                            event.name,
                                            uri.to_uri(false)
                                        )),
                                        Err(e) => dbg_log("Runtime",format!(
                                            "register_listener failed for '{}' on {}: code={:?}, message={:?}",
                                            event.name,
                                            uri.to_uri(false),
                                            e.code,
                                            e.message
                                        )),
                                    }
                                } else {
                                    dbg_log("Runtime",format!(
                                        "Pending subscription skipped: invalid URI build topic='{}' effective_authority='{}' ue=0x{:04X} major={} resource=0x{:04X}",
                                        event.name,
                                        effective_authority,
                                        provider.ue_id,
                                        event.major_version,
                                        event.resource_id
                                    ));
                                }
                            }

                            if !still_pending.is_empty() {
                                if let Ok(mut map) = self.pending_subs.lock() {
                                    map.entry(event.name.clone())
                                        .or_default()
                                        .extend(still_pending);
                                }
                            }
                        } else {
                            dbg_log("Runtime",format!(
                                "DiscoveryListener: no pending subscriptions for topic='{}'",
                                event.name
                            ));
                        }
                    }
                    _ => {
                        dbg_log("Runtime",format!(
                            "Discovery event ignored: unknown kind='{}' name='{}'",
                            event.kind, event.name
                        ));
                    }
                }
            } else {
                dbg_log("Runtime",format!(
                    "Discovery payload parse failed: payload_len={} preview={}...",
                    payload.len(),
                    payload_preview(&payload, 24)
                ));
            }
        } else {
            dbg_log("Runtime","DiscoveryListener received message without payload");
        }
    }
}

/// Optional MQTT 5 broker configuration for telemetry.
#[derive(Clone, Debug)]
pub struct MqttConfig {
    /// The URI of the MQTT broker (e.g., "mqtt://127.0.0.1:1883").
    pub broker_uri: String,
    /// The client ID to use for the MQTT connection.
    pub client_id: String,
}

/// Runtime configuration including optional cloud transport parameters.
#[derive(Clone, Debug, Default)]
pub struct RuntimeConfig {
    /// Optional MQTT 5 broker options for cloud-bound messaging
    pub mqtt_config: Option<MqttConfig>,
    /// Optional path to the application manifest JSON file.
    /// If `None`, falls back to `PACOM_MANIFEST_PATH` env var,
    /// then to `/etc/pacom/manifest.json`.
    pub manifest_path: Option<String>,
    /// Optional logical identity of this node/application.
    /// Falls back to `UP_AUTHORITY` env var, then `HOSTNAME`, then "local_ecu".
    pub authority: Option<String>,
}

/// Core runtime engine that binds logical operations to transports.
/// Dynamically coordinates vSomeIP (intra-vehicle) and MQTT (cloud) transports,
/// and fully encapsulates the lower-level communication abstractions.
pub struct RuntimeEngine {
    router: Arc<PacomRouter>,
    rpc_client: Arc<InMemoryRpcClient>,
    rpc_server: Arc<InMemoryRpcServer>,
    discovery_cache: Arc<RwLock<DiscoveryCache>>,
    provided_capabilities: Arc<RwLock<ProvidedCapabilities>>,
    manifest: ManifestConfig,
    local_ue_id: u16,
    /// Pending subscriptions that will be activated as soon as the publisher announces itself.
    pending_subscriptions: Arc<Mutex<HashMap<String, Vec<PendingSubscription>>>>,
    shutdown_tx: watch::Sender<bool>,
    discovery_task: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl RuntimeEngine {
    fn derive_topic_publish_ue_id(base_ue_id: u16) -> u16 {
        // Keep topic publish on a deterministic UE-ID distinct from the app UE-ID
        // to avoid SOME/IP service-offer collisions between RPC (major=1) and
        // publish paths (which may internally map with wildcard major semantics).
        let mut topic_ue = base_ue_id ^ 0x4000;
        if topic_ue == 0 || topic_ue == 0xFFFF {
            topic_ue ^= 0x2000;
        }
        topic_ue
    }

    /// Initializes the runtime engine by dynamically negotiating local vSomeIP roles,
    /// resolving the authority and application ID from the environment,
    /// and optionally connecting to the remote MQTT 5 broker.
    pub async fn new(config: RuntimeConfig) -> Result<Self, PacomError> {
        // 0. Load and validate the per-instance manifest
        let manifest = if let Some(ref path) = config.manifest_path {
            ManifestConfig::load(path)
        } else {
            ManifestConfig::load_from_env()
        };
        manifest.validate_no_collisions()?;

        // 1. Resolve authority/ECU name and application UE_ID dynamically from config or environment
        let authority = config
            .authority
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| resolve_authority());
        let ue_id = resolve_ue_id();
        dbg_log("Runtime",format!(
            "RuntimeEngine::new authority='{}' ue_id=0x{:04x} mqtt_enabled={} manifest_path={:?}",
            authority,
            ue_id,
            config.mqtt_config.is_some(),
            config.manifest_path
        ));
        dbg_log("Runtime",format!(
            "RuntimeEngine::new flags: PACOM_DISABLE_VSOMEIP={:?}",
            std::env::var("PACOM_DISABLE_VSOMEIP").ok()
        ));

        // 2. Set up the local vSomeIP transport (Router or Client) if not disabled
        let vsomeip_transport = if std::env::var("PACOM_DISABLE_VSOMEIP")
            .unwrap_or_else(|_| "false".to_string())
            != "true"
        {
            Some(vsomeip::setup_vsomeip_transport(ue_id, &authority).await?)
        } else {
            None
        };

        // 3. Set up the optional MQTT 5 transport
        let mqtt_transport = if let Some(mqtt_cfg) = config.mqtt_config {
            let mqtt =
                mqtt::setup_mqtt_transport(&mqtt_cfg.broker_uri, &mqtt_cfg.client_id, &authority)
                    .await?;
            Some(mqtt)
        } else {
            None
        };

        // 4. Wrap both inside the pacom router
        let has_vsomeip = vsomeip_transport.is_some();
        let router = Arc::new(PacomRouter::new(
            authority.clone(),
            vsomeip_transport,
            mqtt_transport,
        ));

        // 5. Initialize RPC client/server components over the selected router
        let rpc_client = Arc::new(
            InMemoryRpcClient::new(router.clone(), router.clone())
                .await
                .map_err(|e| {
                    PacomError::Config(format!("Failed to initialize InMemoryRpcClient: {e:?}"))
                })?,
        );
        let rpc_server = Arc::new(InMemoryRpcServer::new(router.clone(), router.clone()));

        let discovery_cache = Arc::new(RwLock::new(DiscoveryCache::default()));
        let provided_capabilities = Arc::new(RwLock::new(ProvidedCapabilities {
            rpc_services: manifest.rpc.provide.clone(),
            published_topics: manifest
                .topics
                .publish
                .iter()
                .filter(|topic| !is_cloud_topic(topic))
                .cloned()
                .collect(),
        }));

        let pending_subscriptions: Arc<Mutex<HashMap<String, Vec<PendingSubscription>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Register vSomeIP discovery channels whenever this node has vSomeIP enabled
        // and actually needs to discover remote peers (has subscriptions or consumed RPCs).
        // Pure providers with no consume/subscribe skip all 16 registrations.
        let has_any_subscriptions =
            !manifest.topics.subscribe.is_empty() || !manifest.rpc.consume.is_empty();
        let needs_vsomeip_discovery = has_vsomeip && has_any_subscriptions;

        let discovery_listener = Arc::new(DiscoveryListener {
            cache: discovery_cache.clone(),
            router: router.clone(),
            pending_subs: pending_subscriptions.clone(),
        });

        if needs_vsomeip_discovery {
            dbg_log("Runtime",format!(
                "Registering discovery listeners on {} channels",
                discovery_channel_count()
            ));
            // Subscribe to all 16 discovery channels to hear from any peer.
            for i in 0..discovery_channel_count() {
                let discovery_uri = UUri::try_from_parts(
                    "*",
                    (DISCOVERY_UE_ID + i) as u32,
                    1,
                    DISCOVERY_RESOURCE_ID,
                )
                .map_err(|e| PacomError::Config(format!("Failed to build discovery URI: {e:?}")))?;
                dbg_log("Runtime",format!(
                    "Register discovery listener channel={} uri={}",
                    i,
                    uri_dbg(&discovery_uri)
                ));
                router
                    .register_listener(&discovery_uri, None, discovery_listener.clone())
                    .await?;
            }
        }

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let discovery_task = Arc::new(Mutex::new(None));

        if has_vsomeip {
            let task = spawn_discovery_reannounce_task(
                router.clone(),
                provided_capabilities.clone(),
                ue_id,
                manifest.clone(),
                shutdown_rx,
            );
            if let Ok(mut guard) = discovery_task.lock() {
                *guard = Some(task);
            }
        } else {
            dbg_log("Runtime","Skipping discovery reannounce task because vSomeIP transport is disabled");
        }

        Ok(Self {
            router,
            rpc_client,
            rpc_server,
            discovery_cache,
            provided_capabilities,
            manifest,
            local_ue_id: ue_id,
            pending_subscriptions,
            shutdown_tx,
            discovery_task,
        })
    }

    pub async fn shutdown(&self) -> Result<(), PacomError> {
        let _ = self.shutdown_tx.send(true);
        let handle = if let Ok(mut guard) = self.discovery_task.lock() {
            guard.take()
        } else {
            None
        };

        if let Some(handle) = handle {
            let _ = handle.await;
        }
        Ok(())
    }

    async fn announce_discovery(&self, kind: &str, name: &str) -> Result<(), PacomError> {
        self.announce_discovery_with_provider(kind, name, self.local_ue_id)
            .await
    }

    async fn announce_discovery_with_provider(
        &self,
        kind: &str,
        name: &str,
        provider_ue_id: u16,
    ) -> Result<(), PacomError> {
        let authority = self.router.get_authority();
        let channel = (self.local_ue_id % discovery_channel_count()) as u16;

        let source = UUri::try_from_parts(
            &authority,
            (DISCOVERY_UE_ID + channel) as u32,
            1,
            DISCOVERY_RESOURCE_ID,
        )
        .map_err(|e| PacomError::Config(format!("Invalid discovery URI: {e:?}")))?;

        let resource_id = if kind == "rpc_provide" {
            self.manifest.method_id_for(name)
        } else {
            self.manifest.resource_id_for(name)
        };

        let event = DiscoveryEvent {
            kind: kind.to_string(),
            name: name.to_string(),
            provider_ue_id,
            major_version: 1, // Standard major version
            provider_authority: authority,
            protocol_version: default_protocol_version(),
            resource_id,
        };

        dbg_log("Runtime",format!(
            "announce_discovery kind='{}' name='{}' channel={} source={}",
            event.kind,
            event.name,
            channel,
            uri_dbg(&source)
        ));

        send_discovery_event(&self.router, source, event).await
    }

    /// Publishes raw bytes payload to a logical topic name.
    ///
    /// Topic names are resolved through the per-instance manifest.
    pub async fn publish(&self, topic_name: &str, payload: Vec<u8>) -> Result<(), PacomError> {
        if !self.manifest.is_topic_published(topic_name) {
            return Err(PacomError::ManifestViolation {
                operation: "topics.publish".to_string(),
                name: topic_name.to_string(),
            });
        }

        {
            let mut caps = self.provided_capabilities.write().map_err(|_| {
                PacomError::Config("Failed to update provided capabilities".to_string())
            })?;
            if !is_cloud_topic(topic_name) {
                caps.published_topics.insert(topic_name.to_string());
            }
        }

        if is_cloud_topic(topic_name) {
            let cloud_authority = cloud_authority_name()?;
            dbg_log("Runtime",format!(
                "publish topic='{}' routed as cloud-bound authority='{}'",
                topic_name, cloud_authority
            ));
            self.publish_to_authority(topic_name, &cloud_authority, payload)
                .await
        } else {
            let resource_id = self.manifest.resource_id_for(topic_name);

            let local_authority = self.router.get_authority();
            let local_ue_id = self.local_ue_id;
            let topic_publish_ue_id = Self::derive_topic_publish_ue_id(local_ue_id);

            let uri = UUri::try_from_parts(&local_authority, topic_publish_ue_id as u32, 1, resource_id)
                .map_err(|e| PacomError::Config(format!("Invalid topic URI: {e:?}")))?;

            let msg = UMessageBuilder::publish(uri)
                .build_with_payload(payload, UPayloadFormat::UPAYLOAD_FORMAT_RAW)
                .map_err(|e| PacomError::Config(format!("Failed to build message: {e:?}")))?;

            dbg_log("Runtime",format!(
                "publish topic='{}' uri='{}' payload_len={} topic_publish_ue=0x{:04X} app_ue=0x{:04X}",
                topic_name,
                msg.attributes
                    .source
                    .as_ref()
                    .map(|u| u.to_uri(false))
                    .unwrap_or_else(|| "<none>".to_string()),
                msg.payload.as_ref().map(|p| p.len()).unwrap_or(0),
                topic_publish_ue_id,
                local_ue_id
            ));

            self.announce_discovery_with_provider("topic_publish", topic_name, topic_publish_ue_id)
                .await?;
            self.router.send(msg).await?;
            Ok(())
        }
    }

    /// Publishes a logical event topic targeting a specific authority.
    ///
    /// If the authority differs from local authority, routing is delegated to the
    /// cloud transport by the router.
    pub async fn publish_to_authority(
        &self,
        topic_name: &str,
        authority: &str,
        payload: Vec<u8>,
    ) -> Result<(), PacomError> {
        if !self.manifest.is_topic_published(topic_name) {
            return Err(PacomError::ManifestViolation {
                operation: "topics.publish".to_string(),
                name: topic_name.to_string(),
            });
        }

        let resource_id = self.manifest.resource_id_for(topic_name);
        let local_authority = self.router.get_authority();
        let local_ue_id = self.local_ue_id;

        // The source is US (local ECU)
        let source_uri = UUri::try_from_parts(&local_authority, local_ue_id as u32, 1, resource_id)
            .map_err(|e| PacomError::Config(format!("Invalid topic URI: {e:?}")))?;

        // The sink is the TARGET authority (e.g., cloud.bridge)
        // up-transport-mqtt5 (OffVehicle) REQUIRES a sink URI to route messages across domains.
        // Therefore, we use a Notification message instead of a pure Publish.
        let sink_uri = UUri::try_from_parts(authority, 0, 0, 0)
            .map_err(|e| PacomError::Config(format!("Invalid sink URI: {e:?}")))?;

        let msg = UMessageBuilder::notification(source_uri, sink_uri)
            .build_with_payload(payload, UPayloadFormat::UPAYLOAD_FORMAT_RAW)
            .map_err(|e| PacomError::Config(format!("Failed to build message: {e:?}")))?;

        dbg_log("Runtime",format!(
            "publish_to_authority topic='{}' target='{}' payload_len={}",
            topic_name,
            authority,
            msg.payload.as_ref().map(|p| p.len()).unwrap_or(0)
        ));

        self.router.send(msg).await?;
        Ok(())
    }

    /// Subscribes a closure callback to receive raw bytes from a logical topic key.
    ///
    /// Non-blocking: the callback is registered immediately and will be activated
    /// either now (if the publisher was already discovered) or reactively as soon as
    /// the publisher announces itself via the discovery mechanism.
    pub async fn subscribe<F>(&self, topic_name: &str, callback: F) -> Result<(), PacomError>
    where
        F: Fn(Vec<u8>) + Send + Sync + 'static,
    {
        if !self.manifest.is_topic_subscribed(topic_name) {
            return Err(PacomError::ManifestViolation {
                operation: "topics.subscribe".to_string(),
                name: topic_name.to_string(),
            });
        }

        let resource_id = self.manifest.resource_id_for(topic_name);
        let expected_resource_id = Arc::new(std::sync::atomic::AtomicU16::new(
            if is_cloud_topic(topic_name) { resource_id } else { 0 }
        ));
        
        let listener: Arc<dyn UListener> = Arc::new(ClosureListener {
            expected_resource_id: expected_resource_id.clone(),
            callback: Box::new(callback),
        });

        if is_cloud_topic(topic_name) {
            let cloud_authority = cloud_authority_name()?;
            let source_filter = cloud_wildcard_source_uri(resource_id)?;
            let sink_filter = cloud_sink_marker_uri(&cloud_authority)?;

            dbg_log("Runtime",format!(
                "subscribe topic='{}' resolved as cloud listener source='{}' sink='{}'",
                topic_name,
                source_filter.to_uri(false),
                sink_filter.to_uri(false)
            ));

            self.router
                .register_listener(&source_filter, Some(&sink_filter), listener)
                .await?;
            return Ok(());
        }

        dbg_log("Runtime",format!(
            "subscribe topic='{}' resource_id=0x{:04X}",
            topic_name,
            resource_id
        ));

        // Check if we already know who publishes this topic from a prior discovery event.
        let maybe_info = self
            .discovery_cache
            .read()
            .ok()
            .and_then(|c| c.topic_publishers.get(topic_name).cloned());

        if let Some(info) = maybe_info {
            // Publisher already known: register immediately.
            let uri = UUri::try_from_parts(
                &info.authority,
                info.ue_id as u32,
                info.major_version,
                info.resource_id,
            )
            .map_err(|e| PacomError::Config(format!("Invalid subscribe topic URI: {e:?}")))?;
            dbg_log("Runtime",format!(
                "subscribe: provider already known topic='{}' register_listener uri='{}'",
                topic_name,
                uri.to_uri(false)
            ));
            dbg_log("Runtime",format!(
                "subscribe immediate topic='{}' using provider authority='{}' ue=0x{:04x} uri='{}'",
                topic_name,
                info.authority,
                info.ue_id,
                uri.to_uri(false)
            ));
            dbg_log("Runtime",format!(
                "subscribe immediate provider={}",
                provider_dbg(&info)
            ));
            self.router.register_listener(&uri, None, listener).await?;
        } else {
            // Publisher not yet known: enqueue as pending.
            dbg_log("Runtime",format!(
                "subscribe: provider not yet known topic='{}' enqueue pending",
                topic_name
            ));
            // DiscoveryListener will trigger the registration when the publisher announces.
            if let Ok(mut map) = self.pending_subscriptions.lock() {
                map.entry(topic_name.to_string())
                    .or_default()
                    .push(PendingSubscription {
                        listener,
                        expected_resource_id: Some(expected_resource_id),
                        source_authority: None,
                    });
                dbg_log("Runtime",format!("subscribe pending topic='{}'", topic_name));
                let total_pending = map.values().map(|v| v.len()).sum::<usize>();
                dbg_log("Runtime",format!(
                    "subscribe pending stats: pending_topics={} total_pending_subscriptions={}",
                    map.len(),
                    total_pending
                ));
            }
        }
        Ok(())
    }

    /// Subscribes to a logical event topic from a specific authority.
    pub async fn subscribe_from_authority<F>(
        &self,
        topic_name: &str,
        authority: &str,
        callback: F,
    ) -> Result<(), PacomError>
    where
        F: Fn(Vec<u8>) + Send + Sync + 'static,
    {
        if !self.manifest.is_topic_subscribed(topic_name) {
            return Err(PacomError::ManifestViolation {
                operation: "topics.subscribe".to_string(),
                name: topic_name.to_string(),
            });
        }

        let resource_id = self.manifest.resource_id_for(topic_name);
        let expected_resource_id = Arc::new(std::sync::atomic::AtomicU16::new(
            if self.router.is_cloud_authority(authority) || is_cloud_topic(topic_name) { resource_id } else { 0 }
        ));

        let listener: Arc<dyn UListener> = Arc::new(ClosureListener {
            expected_resource_id: expected_resource_id.clone(),
            callback: Box::new(callback),
        });

        let is_cloud = self.router.is_cloud_authority(authority) || is_cloud_topic(topic_name);
        dbg_log("Runtime",format!(
            "subscribe_from_authority topic='{}' target_authority='{}' resource_id=0x{:04X} cloud_bound={}",
            topic_name,
            authority,
            resource_id,
            is_cloud
        ));

        if is_cloud {
            // For cross-domain subscriptions the MQTT transport routes by authority and
            // topic string, not by vSomeIP service ID. Use 0xFFFF as a neutral wildcard.
            let uri = UUri::try_from_parts(authority, 0xFFFF, 1, resource_id)
                .map_err(|e| PacomError::Config(format!("Invalid subscribe URI: {e:?}")))?;
            self.router.register_listener(&uri, None, listener).await?;
        } else {


            // For local (vSomeIP) subscriptions, use the same reactive mechanism as subscribe():
            // register immediately if we know the publisher, else enqueue as pending.
            let maybe_info = self
                .discovery_cache
                .read()
                .ok()
                .and_then(|c| c.topic_publishers.get(topic_name).cloned());

            if let Some(info) = maybe_info {
                let uri = UUri::try_from_parts(
                    &info.authority,
                    info.ue_id as u32,
                    info.major_version,
                    resource_id,
                )
                .map_err(|e| PacomError::Config(format!("Invalid subscribe URI: {e:?}")))?;
                dbg_log("Runtime",format!(
                    "subscribe_from_authority immediate local provider={} uri={}",
                    provider_dbg(&info),
                    uri_dbg(&uri)
                ));
                self.router.register_listener(&uri, None, listener).await?;
            } else {
                if let Ok(mut map) = self.pending_subscriptions.lock() {
                    map.entry(topic_name.to_string())
                        .or_default()
                        .push(PendingSubscription {
                            listener,
                            expected_resource_id: Some(expected_resource_id),
                            source_authority: Some(authority.to_string()),
                        });
                    let total_pending = map.values().map(|v| v.len()).sum::<usize>();
                    dbg_log("Runtime",format!(
                        "subscribe_from_authority pending stats: pending_topics={} total_pending_subscriptions={}",
                        map.len(),
                        total_pending
                    ));
                }
            }
        }
        Ok(())
    }

    /// Invokes an RPC method identified by a logical service name.
    pub async fn call_rpc(
        &self,
        service_name: &str,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, PacomError> {
        if !self.manifest.is_rpc_consumed(service_name) {
            return Err(PacomError::ManifestViolation {
                operation: "rpc.consume".to_string(),
                name: service_name.to_string(),
            });
        }

        let info = self.resolve_rpc_provider_with_retry(service_name).await?;

        let method_uri = UUri::try_from_parts(
            &info.authority,
            info.ue_id as u32,
            info.major_version,
            info.resource_id,
        )
        .map_err(|e| PacomError::Config(format!("Invalid method URI: {e:?}")))?;

        let payload_obj = UPayload::new(payload, UPayloadFormat::UPAYLOAD_FORMAT_RAW);
        let call_options = CallOptions::for_rpc_request(rpc_timeout_ms(), None, None, None);

        let response = self
            .rpc_client
            .invoke_method(method_uri, call_options, Some(payload_obj))
            .await
            .map_err(|e| PacomError::RpcError(format!("RPC invocation failed: {e:?}")))?;

        match response {
            Some(p) => Ok(p.payload().to_vec()),
            None => Err(PacomError::EmptyResponse),
        }
    }

    /// Registers an asynchronous handler for an RPC method served by this process.
    pub async fn register_rpc_method<F, Fut>(
        &self,
        service_name: &str,
        handler: F,
    ) -> Result<(), PacomError>
    where
        F: Fn(Vec<u8>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Vec<u8>> + Send + 'static,
    {
        if !self.manifest.is_rpc_provided(service_name) {
            return Err(PacomError::ManifestViolation {
                operation: "rpc.provide".to_string(),
                name: service_name.to_string(),
            });
        }

        let method_id = self.manifest.method_id_for(service_name);

        let wrapper = Arc::new(ClosureHandler { handler });
        self.rpc_server
            .register_endpoint(None, method_id, wrapper)
            .await
            .map_err(|e| PacomError::Config(format!("Failed to register RPC: {e:?}")))?;

        {
            let mut caps = self.provided_capabilities.write().map_err(|_| {
                PacomError::Config("Failed to update provided capabilities".to_string())
            })?;
            caps.rpc_services.insert(service_name.to_string());
        }

        self.announce_discovery("rpc_provide", service_name).await
    }

    async fn resolve_rpc_provider_with_retry(
        &self,
        service_name: &str,
    ) -> Result<ProviderInfo, PacomError> {
        if let Some(info) = self.lookup_rpc_provider(service_name)? {
            dbg_log("Runtime",format!(
                "resolve_rpc_provider_with_retry immediate hit service='{}' provider={}",
                service_name,
                provider_dbg(&info)
            ));
            return Ok(info);
        }

        let timeout = discovery_wait_timeout();
        let poll = discovery_poll_interval();
        let deadline = Instant::now() + timeout;
        let mut attempts: u64 = 0;
        dbg_log("Runtime",format!(
            "resolve_rpc_provider_with_retry waiting service='{}' timeout_ms={} poll_ms={}",
            service_name,
            timeout.as_millis(),
            poll.as_millis()
        ));

        while Instant::now() < deadline {
            sleep(poll).await;
            attempts += 1;
            if let Some(info) = self.lookup_rpc_provider(service_name)? {
                dbg_log("Runtime",format!(
                    "resolve_rpc_provider_with_retry resolved service='{}' attempts={} provider={}",
                    service_name,
                    attempts,
                    provider_dbg(&info)
                ));
                return Ok(info);
            }
            if attempts % 20 == 0 {
                dbg_log("Runtime",format!(
                    "resolve_rpc_provider_with_retry still waiting service='{}' attempts={} elapsed_ms={}",
                    service_name,
                    attempts,
                    (Instant::now() + Duration::from_millis(0))
                        .saturating_duration_since(deadline - timeout)
                        .as_millis()
                ));
            }
        }

        Err(PacomError::DiscoveryTimeout {
            name: service_name.to_string(),
            timeout_ms: timeout.as_millis() as u64,
        })
    }

    fn lookup_rpc_provider(&self, service_name: &str) -> Result<Option<ProviderInfo>, PacomError> {
        let cache = self
            .discovery_cache
            .read()
            .map_err(|_| PacomError::Config("Failed to read discovery cache".to_string()))?;
        Ok(cache.rpc_providers.get(service_name).cloned())
    }
}

fn discovery_wait_timeout() -> Duration {
    std::env::var("PACOM_DISCOVERY_MAX_WAIT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_millis(180_000))
}

fn rpc_timeout_ms() -> u32 {
    std::env::var("PACOM_RPC_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(5_000)
}

fn discovery_channel_count() -> u16 {
    std::env::var("PACOM_DISCOVERY_CHANNELS")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .filter(|v| *v > 0 && *v <= 64)
        .unwrap_or(16)
}

fn discovery_poll_interval() -> Duration {
    std::env::var("PACOM_DISCOVERY_POLL_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_millis(250))
}

fn discovery_reannounce_interval() -> Duration {
    std::env::var("PACOM_DISCOVERY_REANNOUNCE_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(5))
}

fn spawn_discovery_reannounce_task(
    router: Arc<PacomRouter>,
    provided_capabilities: Arc<RwLock<ProvidedCapabilities>>,
    local_ue_id: u16,
    manifest: ManifestConfig,
    mut shutdown_rx: watch::Receiver<bool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let interval = discovery_reannounce_interval();
        dbg_log("Runtime",format!(
            "discovery reannounce task started interval_secs={}",
            interval.as_secs()
        ));
        loop {
            tokio::select! {
                _ = sleep(interval) => {}
                changed = shutdown_rx.changed() => {
                    if changed.is_ok() && *shutdown_rx.borrow() {
                        dbg_log("Runtime","discovery reannounce task stopped by shutdown signal");
                        break;
                    }
                }
            }

            let (services, topics) = match provided_capabilities.read() {
                Ok(caps) => (
                    caps.rpc_services.iter().cloned().collect::<Vec<_>>(),
                    caps.published_topics.iter().cloned().collect::<Vec<_>>(),
                ),
                Err(_) => continue,
            };

            let topic_publish_ue_id = RuntimeEngine::derive_topic_publish_ue_id(local_ue_id);
            let channel = (local_ue_id % discovery_channel_count()) as u16;
            dbg_log("Runtime",format!(
                "discovery reannounce tick: rpc_count={} topic_count={} channel={}",
                services.len(),
                topics.len(),
                channel
            ));

            for service in services {
                let authority = router.get_authority();
                if let Ok(source) = UUri::try_from_parts(
                    &authority,
                    (DISCOVERY_UE_ID + channel) as u32,
                    1,
                    DISCOVERY_RESOURCE_ID,
                ) {
                    let event = DiscoveryEvent {
                        kind: "rpc_provide".to_string(),
                        name: service.clone(),
                        provider_ue_id: local_ue_id,
                        major_version: 1,
                        provider_authority: authority,
                        protocol_version: default_protocol_version(),
                        resource_id: manifest.method_id_for(&service),
                    };
                    let _ = send_discovery_event(&router, source, event).await;
                }
            }

            for topic in topics {
                let authority = router.get_authority();
                if let Ok(source) = UUri::try_from_parts(
                    &authority,
                    (DISCOVERY_UE_ID + channel) as u32,
                    1,
                    DISCOVERY_RESOURCE_ID,
                ) {
                    let event = DiscoveryEvent {
                        kind: "topic_publish".to_string(),
                        name: topic.clone(),
                        provider_ue_id: topic_publish_ue_id,
                        major_version: 1,
                        provider_authority: authority,
                        protocol_version: default_protocol_version(),
                        resource_id: manifest.resource_id_for(&topic),
                    };
                    let _ = send_discovery_event(&router, source, event).await;
                }
            }
        }
    })
}

async fn send_discovery_event(
    router: &Arc<PacomRouter>,
    source: UUri,
    event: DiscoveryEvent,
) -> Result<(), PacomError> {
    let payload = serde_json::to_vec(&event)
        .map_err(|e| PacomError::Config(format!("Failed to serialize discovery event: {e}")))?;

    let msg = UMessageBuilder::publish(source)
        .build_with_payload(payload, UPayloadFormat::UPAYLOAD_FORMAT_RAW)
        .map_err(|e| PacomError::Config(format!("Failed to build discovery message: {e:?}")))?;

    dbg_log("Runtime",format!(
        "send_discovery_event kind='{}' name='{}' source={} payload_len={}",
        event.kind,
        event.name,
        msg.attributes
            .source
            .as_ref()
            .map(uri_dbg)
            .unwrap_or_else(|| "<none>".to_string()),
        msg.payload.as_ref().map(|p| p.len()).unwrap_or(0)
    ));

    match router.send(msg).await {
        Ok(_) => dbg_log("Runtime","send_discovery_event transport send result=ok"),
        Err(e) => {
            dbg_log("Runtime",format!(
                "send_discovery_event transport send result=err code={:?} message={:?}",
                e.code, e.message
            ));
            return Err(e.into());
        }
    }
    Ok(())
}

/// Resolve the node/ECU authority name from environment.
fn resolve_authority() -> String {
    if let Ok(authority) = std::env::var("UP_AUTHORITY") {
        if !authority.trim().is_empty() {
            return authority;
        }
    }

    if let Ok(hostname) = std::env::var("HOSTNAME") {
        if !hostname.trim().is_empty() {
            return hostname;
        }
    }

    "local_ecu".to_string()
}

/// Resolve the application UE identifier from environment,
/// or derive a stable default from the executable name.
fn resolve_ue_id() -> u16 {
    std::env::var("UP_UE_ID")
        .ok()
        .and_then(|val| {
            if val.starts_with("0x") {
                u16::from_str_radix(&val[2..], 16).ok()
            } else {
                val.parse::<u16>().ok()
            }
        })
        .unwrap_or_else(|| {
            // Determine a deterministic ID based on the current executable's name
            let exe_name = std::env::current_exe()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "default_app".to_string());

            // FNV-1a hashing to convert the string to a u16
            let mut hash = 0x811c9dc5u32;
            for byte in exe_name.bytes() {
                hash ^= byte as u32;
                hash = hash.wrapping_mul(0x01000193);
            }
            let val = (hash ^ (hash >> 16)) as u16;

            // Restrict to user-application ID range (>= 0x1000) to avoid system conflicts
            if val < 0x1000 { val + 0x1000 } else { val }
        })
}

/// Internal adapter from uProtocol listener callbacks to byte closures.
struct ClosureListener {
    expected_resource_id: Arc<std::sync::atomic::AtomicU16>,
    callback: Box<dyn Fn(Vec<u8>) + Send + Sync + 'static>,
}

#[async_trait]
impl UListener for ClosureListener {
    async fn on_receive(&self, message: UMessage) {
        if verbose_debug_enabled() {
            let source_uri = message
                .attributes
                .source
                .as_ref()
                .map(|u| u.to_uri(false))
                .unwrap_or_else(|| "<none>".to_string());
            let sink_uri = message
                .attributes
                .sink
                .as_ref()
                .map(|u| u.to_uri(false))
                .unwrap_or_else(|| "<none>".to_string());
            let expected = self.expected_resource_id.load(std::sync::atomic::Ordering::Relaxed);
            dbg_log("Runtime",format!(
                "ClosureListener received message: expected_resource_id={}, source_uri={}, sink_uri={}, payload_len={}",
                expected,
                source_uri,
                sink_uri,
                message.payload.as_ref().map(|p| p.len()).unwrap_or(0)
            ));
        }

        if let Some(attributes) = message.attributes.into_option() {
            if let Some(source) = attributes.source.into_option() {
                let expected = self.expected_resource_id.load(std::sync::atomic::Ordering::Relaxed);
                if expected != 0 && source.resource_id != expected as u32 {
                    dbg_log("Runtime",format!(
                        "ClosureListener dropped message: expected_resource_id={}, got_resource_id={}",
                        expected, source.resource_id
                    ));
                    return; // Ignore messages intended for other topics (MQTT broadcast workaround)
                }
            } else {
                dbg_log("Runtime","ClosureListener: message has no source attribute");
            }
        } else {
            dbg_log("Runtime","ClosureListener: message has no attributes");
        }
        if let Some(payload) = message.payload {
            let expected = self.expected_resource_id.load(std::sync::atomic::Ordering::Relaxed);
            dbg_log("Runtime",format!(
                "ClosureListener delivering payload: resource_id={}, payload_len={}",
                expected,
                payload.len()
            ));
            (self.callback)(payload.to_vec());
        } else {
            dbg_log("Runtime","ClosureListener: message has no payload");
        }
    }
}

/// Internal adapter from uProtocol request handlers to byte closures.
struct ClosureHandler<F> {
    handler: F,
}

#[async_trait]
impl<F, Fut> RequestHandler for ClosureHandler<F>
where
    F: Fn(Vec<u8>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Vec<u8>> + Send + 'static,
{
    async fn handle_request(
        &self,
        _method_id: u16,
        _attributes: &UAttributes,
        request_payload: Option<UPayload>,
    ) -> Result<Option<UPayload>, ServiceInvocationError> {
        let req_bytes = request_payload
            .map(|p| p.payload().to_vec())
            .unwrap_or_default();

        let resp_bytes = (self.handler)(req_bytes).await;

        let resp_payload = UPayload::new(resp_bytes, UPayloadFormat::UPAYLOAD_FORMAT_RAW);
        Ok(Some(resp_payload))
    }
}

#[async_trait]
impl UTransport for RuntimeEngine {
    async fn send(&self, message: UMessage) -> Result<(), UStatus> {
        self.router.send(message).await
    }

    async fn register_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        self.router
            .register_listener(source_filter, sink_filter, listener)
            .await
    }

    async fn unregister_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        self.router
            .unregister_listener(source_filter, sink_filter, listener)
            .await
    }

    async fn receive(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
    ) -> Result<UMessage, UStatus> {
        self.router.receive(source_filter, sink_filter).await
    }
}

impl LocalUriProvider for RuntimeEngine {
    fn get_authority(&self) -> String {
        self.router.get_authority()
    }

    fn get_resource_uri(&self, resource_id: u16) -> UUri {
        self.router.get_resource_uri(resource_id)
    }

    fn get_source_uri(&self) -> UUri {
        self.router.get_source_uri()
    }
}
