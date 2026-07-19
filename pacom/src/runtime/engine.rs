use super::logical_registry::ManifestConfig;
use crate::error::PacomError;
use crate::transport::{mqtt, router::PacomRouter, vsomeip};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::RwLock;
use std::time::{Duration, Instant};
use tokio::time::sleep;
use up_rust::communication::{
    CallOptions, InMemoryRpcClient, InMemoryRpcServer, RequestHandler, RpcClient, RpcServer,
    ServiceInvocationError, UPayload,
};
use up_rust::{
    LocalUriProvider, UAttributes, UListener, UMessage, UMessageBuilder, UPayloadFormat, UStatus,
    UTransport, UUri, UUID,
};

use std::sync::Mutex;

const DISCOVERY_UE_ID: u16 = 0x0F00;
const DISCOVERY_RESOURCE_ID: u16 = 0x8F01;
static RPC_DIAG_SEQ: AtomicU64 = AtomicU64::new(1);

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
        println!("[PACOM-DBG][Runtime] {}", msg.as_ref());
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

fn next_rpc_diag_id() -> u64 {
    RPC_DIAG_SEQ.fetch_add(1, Ordering::Relaxed)
}

fn rpc_diag_log(msg: impl AsRef<str>) {
    if rpc_diag_enabled() {
        println!("[PACOM-RPC-DIAG] {}", msg.as_ref());
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

fn provider_dbg(p: &ProviderInfo) -> String {
    format!(
        "authority='{}' ue=0x{:04X} major={}",
        p.authority, p.ue_id, p.major_version
    )
}

fn payload_preview(bytes: &[u8], max: usize) -> String {
    bytes
        .iter()
        .take(max)
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

fn wildcard_local_subscribe_enabled() -> bool {
    std::env::var("PACOM_ENABLE_LOCAL_WILDCARD_SUBSCRIBE")
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes" || v == "on"
        })
        .unwrap_or(false)
}

#[derive(Clone, Debug)]
struct ProviderInfo {
    ue_id: u16,
    authority: String,
    major_version: u8,
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
}

/// A pending subscription waiting for the publisher to announce itself via Discovery.
struct PendingSubscription {
    listener: Arc<dyn UListener>,
    resource_id: u16,
}

struct DiscoveryListener {
    cache: Arc<RwLock<DiscoveryCache>>,
    router: Arc<PacomRouter>,
    pending_subs: Arc<Mutex<HashMap<String, Vec<PendingSubscription>>>>,
}

#[derive(Clone)]
struct RpcDedupState {
    enabled: bool,
    ttl: Duration,
    max_entries: usize,
    cache: Arc<Mutex<RpcDedupCache>>,
}

impl RpcDedupState {
    fn new() -> Self {
        Self {
            enabled: rpc_dedup_enabled(),
            ttl: rpc_dedup_ttl(),
            max_entries: rpc_dedup_max_entries(),
            cache: Arc::new(Mutex::new(RpcDedupCache::default())),
        }
    }
}

#[derive(Default)]
struct RpcDedupCache {
    entries: HashMap<String, RpcDedupEntry>,
}

struct RpcDedupEntry {
    response: Vec<u8>,
    expires_at: Instant,
}

impl RpcDedupCache {
    fn get_valid_response(&mut self, key: &str, now: Instant) -> Option<Vec<u8>> {
        self.purge_expired(now);
        if let Some(entry) = self.entries.get(key) {
            return Some(entry.response.clone());
        }
        None
    }

    fn insert_response(
        &mut self,
        key: String,
        response: Vec<u8>,
        now: Instant,
        ttl: Duration,
        max_entries: usize,
    ) {
        self.purge_expired(now);

        if self.entries.len() >= max_entries {
            if let Some(first_key) = self.entries.keys().next().cloned() {
                self.entries.remove(&first_key);
            }
        }

        self.entries.insert(
            key,
            RpcDedupEntry {
                response,
                expires_at: now + ttl,
            },
        );
    }

    fn purge_expired(&mut self, now: Instant) {
        self.entries.retain(|_, entry| entry.expires_at > now);
    }
}

fn payload_signature(payload: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    payload.hash(&mut hasher);
    hasher.finish()
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

        dbg_log(format!(
            "DiscoveryListener on_receive: source={} sink={} payload_len={}",
            source_uri_dbg,
            sink_uri_dbg,
            message.payload.as_ref().map(|p| p.len()).unwrap_or(0)
        ));

        if let Some(payload) = message.payload {
            if let Ok(event) = serde_json::from_slice::<DiscoveryEvent>(&payload) {
                dbg_log(format!(
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
                };

                match event.kind.as_str() {
                    "rpc_provide" => {
                        if let Ok(mut cache) = self.cache.write() {
                            cache
                                .rpc_providers
                                .insert(event.name.clone(), provider.clone());
                            dbg_log(format!(
                                "Discovery cache update: rpc_providers size={} latest name='{}' provider={}",
                                cache.rpc_providers.len(),
                                event.name,
                                provider_dbg(&provider)
                            ));
                        }
                    }
                    "topic_publish" => {
                        dbg_log(format!(
                            "DiscoveryListener: received topic_publish name='{}' provider_ue=0x{:04X}",
                            event.name, event.provider_ue_id
                        ));
                        if let Ok(mut cache) = self.cache.write() {
                            cache
                                .topic_publishers
                                .insert(event.name.clone(), provider.clone());
                            dbg_log(format!(
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
                                dbg_log(format!(
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
                            dbg_log(format!(
                                "Pending subscriptions found for '{}': count={}",
                                event.name,
                                subs.len()
                            ));
                            for sub in subs {
                                if let Ok(uri) = UUri::try_from_parts(
                                    &effective_authority,
                                    provider.ue_id as u32,
                                    event.major_version,
                                    sub.resource_id,
                                ) {
                                    dbg_log(format!(
                                        "DiscoveryListener: activating pending listener topic='{}' uri='{}'",
                                        event.name,
                                        uri.to_uri(false)
                                    ));
                                    dbg_log(format!(
                                        "Pending subscription activate: topic='{}' resource_id=0x{:04X} uri={}",
                                        event.name,
                                        sub.resource_id,
                                        uri_dbg(&uri)
                                    ));
                                    match self
                                        .router
                                        .register_listener(&uri, None, sub.listener)
                                        .await
                                    {
                                        Ok(_) => dbg_log(format!(
                                            "register_listener ok for '{}' on {}",
                                            event.name,
                                            uri.to_uri(false)
                                        )),
                                        Err(e) => dbg_log(format!(
                                            "register_listener failed for '{}' on {}: code={:?}, message={:?}",
                                            event.name,
                                            uri.to_uri(false),
                                            e.code,
                                            e.message
                                        )),
                                    }
                                } else {
                                    dbg_log(format!(
                                        "Pending subscription skipped: invalid URI build topic='{}' effective_authority='{}' ue=0x{:04X} major={} resource=0x{:04X}",
                                        event.name,
                                        effective_authority,
                                        provider.ue_id,
                                        event.major_version,
                                        sub.resource_id
                                    ));
                                }
                            }
                        } else {
                            dbg_log(format!(
                                "DiscoveryListener: no pending subscriptions for topic='{}'",
                                event.name
                            ));
                        }
                    }
                    _ => {
                        dbg_log(format!(
                            "Discovery event ignored: unknown kind='{}' name='{}'",
                            event.kind, event.name
                        ));
                    }
                }
            } else {
                dbg_log(format!(
                    "Discovery payload parse failed: payload_len={} preview={}...",
                    payload.len(),
                    payload_preview(&payload, 24)
                ));
            }
        } else {
            dbg_log("DiscoveryListener received message without payload");
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
    /// Pending subscriptions that will be activated as soon as the publisher announces itself.
    pending_subscriptions: Arc<Mutex<HashMap<String, Vec<PendingSubscription>>>>,
    rpc_dedup: Arc<RpcDedupState>,
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

        // 1. Resolve authority/ECU name and application UE_ID dynamically from environment
        let authority = resolve_authority();
        let ue_id = resolve_ue_id();
        dbg_log(format!(
            "RuntimeEngine::new authority='{}' ue_id=0x{:04x} mqtt_enabled={} manifest_path={:?}",
            authority,
            ue_id,
            config.mqtt_config.is_some(),
            config.manifest_path
        ));
        dbg_log(format!(
            "RuntimeEngine::new flags: PACOM_DISABLE_VSOMEIP={:?} PACOM_ENABLE_LOCAL_WILDCARD_SUBSCRIBE={}",
            std::env::var("PACOM_DISABLE_VSOMEIP").ok(),
            wildcard_local_subscribe_enabled()
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
            published_topics: manifest.topics.publish.clone(),
        }));

        let pending_subscriptions: Arc<Mutex<HashMap<String, Vec<PendingSubscription>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let rpc_dedup = Arc::new(RpcDedupState::new());
        dbg_log(format!(
            "RuntimeEngine::new rpc_dedup enabled={} ttl_ms={} max_entries={}",
            rpc_dedup.enabled,
            rpc_dedup.ttl.as_millis(),
            rpc_dedup.max_entries
        ));

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
            dbg_log("Registering discovery listeners on 16 channels");
            // Subscribe to all 16 discovery channels to hear from any peer.
            for i in 0..16 {
                let discovery_uri = UUri::try_from_parts(
                    "*",
                    (DISCOVERY_UE_ID + i) as u32,
                    1,
                    DISCOVERY_RESOURCE_ID,
                )
                .map_err(|e| PacomError::Config(format!("Failed to build discovery URI: {e:?}")))?;
                dbg_log(format!(
                    "Register discovery listener channel={} uri={}",
                    i,
                    uri_dbg(&discovery_uri)
                ));
                router
                    .register_listener(&discovery_uri, None, discovery_listener.clone())
                    .await?;
            }
        }

        if has_vsomeip {
            spawn_discovery_reannounce_task(router.clone(), provided_capabilities.clone());
        } else {
            dbg_log("Skipping discovery reannounce task because vSomeIP transport is disabled");
        }

        Ok(Self {
            router,
            rpc_client,
            rpc_server,
            discovery_cache,
            provided_capabilities,
            manifest,
            pending_subscriptions,
            rpc_dedup,
        })
    }

    async fn announce_discovery(&self, kind: &str, name: &str) -> Result<(), PacomError> {
        let local_ue_id = resolve_ue_id();
        self.announce_discovery_with_provider(kind, name, local_ue_id)
            .await
    }

    async fn announce_discovery_with_provider(
        &self,
        kind: &str,
        name: &str,
        provider_ue_id: u16,
    ) -> Result<(), PacomError> {
        let authority = self.router.get_authority();
        let local_ue_id = resolve_ue_id();
        let channel = (local_ue_id % 16) as u16;

        let source = UUri::try_from_parts(
            &authority,
            (DISCOVERY_UE_ID + channel) as u32,
            1,
            DISCOVERY_RESOURCE_ID,
        )
        .map_err(|e| PacomError::Config(format!("Invalid discovery URI: {e:?}")))?;

        let event = DiscoveryEvent {
            kind: kind.to_string(),
            name: name.to_string(),
            provider_ue_id,
            major_version: 1, // Standard major version
            provider_authority: authority,
        };

        dbg_log(format!(
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
            caps.published_topics.insert(topic_name.to_string());
        }

        let resource_id = self.manifest.resource_id_for(topic_name);

        let local_authority = self.router.get_authority();
        let local_ue_id = resolve_ue_id();
        let topic_publish_ue_id = Self::derive_topic_publish_ue_id(local_ue_id);

        let uri =
            UUri::try_from_parts(&local_authority, topic_publish_ue_id as u32, 1, resource_id)
                .map_err(|e| PacomError::Config(format!("Invalid topic URI: {e:?}")))?;

        let msg = UMessageBuilder::publish(uri)
            .build_with_payload(payload, UPayloadFormat::UPAYLOAD_FORMAT_RAW)
            .map_err(|e| PacomError::Config(format!("Failed to build message: {e:?}")))?;

        dbg_log(format!(
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

        self.router.send(msg).await?;
        self.announce_discovery_with_provider("topic_publish", topic_name, topic_publish_ue_id)
            .await?;
        Ok(())
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
        let local_ue_id = resolve_ue_id();

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

        dbg_log(format!(
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
        let listener: Arc<dyn UListener> = Arc::new(ClosureListener {
            expected_resource_id: resource_id,
            callback: Box::new(callback),
        });

        dbg_log(format!(
            "subscribe topic='{}' resource_id=0x{:04X} wildcard_fallback_enabled={}",
            topic_name,
            resource_id,
            wildcard_local_subscribe_enabled()
        ));

        // Optional fallback for startup races. Disabled by default because broad
        // wildcards can create ambiguous SOME/IP registrations in mixed deployments.
        if wildcard_local_subscribe_enabled() {
            if let Ok(wildcard_uri) = UUri::try_from_parts("*", 0xFFFF, 0xFF, resource_id) {
                match self
                    .router
                    .register_listener(&wildcard_uri, None, listener.clone())
                    .await
                {
                    Ok(_) => dbg_log(format!(
                        "subscribe wildcard listener registered for topic='{}' uri='{}'",
                        topic_name,
                        wildcard_uri.to_uri(false)
                    )),
                    Err(e) => dbg_log(format!(
                        "subscribe wildcard listener failed for topic='{}' uri='{}': code={:?}",
                        topic_name,
                        wildcard_uri.to_uri(false),
                        e.code
                    )),
                }
            }
        }

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
                resource_id,
            )
            .map_err(|e| PacomError::Config(format!("Invalid subscribe topic URI: {e:?}")))?;
            dbg_log(format!(
                "subscribe: provider already known topic='{}' register_listener uri='{}'",
                topic_name,
                uri.to_uri(false)
            ));
            dbg_log(format!(
                "subscribe immediate topic='{}' using provider authority='{}' ue=0x{:04x} uri='{}'",
                topic_name,
                info.authority,
                info.ue_id,
                uri.to_uri(false)
            ));
            dbg_log(format!(
                "subscribe immediate provider={}",
                provider_dbg(&info)
            ));
            self.router.register_listener(&uri, None, listener).await?;
        } else {
            // Publisher not yet known: enqueue as pending.
            dbg_log(format!(
                "subscribe: provider not yet known topic='{}' enqueue pending",
                topic_name
            ));
            // DiscoveryListener will trigger the registration when the publisher announces.
            if let Ok(mut map) = self.pending_subscriptions.lock() {
                map.entry(topic_name.to_string())
                    .or_default()
                    .push(PendingSubscription {
                        listener,
                        resource_id,
                    });
                dbg_log(format!("subscribe pending topic='{}'", topic_name));
                let total_pending = map.values().map(|v| v.len()).sum::<usize>();
                dbg_log(format!(
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
        let temp_uri = UUri::try_from_parts(authority, 0, 0, 0)
            .map_err(|e| PacomError::Config(format!("Invalid URI: {e:?}")))?;

        let listener: Arc<dyn UListener> = Arc::new(ClosureListener {
            expected_resource_id: resource_id,
            callback: Box::new(callback),
        });

        dbg_log(format!(
            "subscribe_from_authority topic='{}' target_authority='{}' resource_id=0x{:04X} cloud_bound={} wildcard_fallback_enabled={}",
            topic_name,
            authority,
            resource_id,
            self.router.is_cloud_bound(&temp_uri),
            wildcard_local_subscribe_enabled()
        ));

        if self.router.is_cloud_bound(&temp_uri) {
            // For cross-domain subscriptions the MQTT transport routes by authority and
            // topic string, not by vSomeIP service ID. Use 0xFFFF as a neutral wildcard.
            let uri = UUri::try_from_parts(authority, 0xFFFF, 1, resource_id)
                .map_err(|e| PacomError::Config(format!("Invalid subscribe URI: {e:?}")))?;
            self.router.register_listener(&uri, None, listener).await?;
        } else {
            // Optional fallback for startup races. Disabled by default for the same
            // reason as subscribe(): avoid overbroad SOME/IP wildcard registrations.
            if wildcard_local_subscribe_enabled() {
                if let Ok(wildcard_uri) = UUri::try_from_parts("*", 0xFFFF, 0xFF, resource_id) {
                    let _ = self
                        .router
                        .register_listener(&wildcard_uri, None, listener.clone())
                        .await;
                }
            }

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
                dbg_log(format!(
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
                            resource_id,
                        });
                    let total_pending = map.values().map(|v| v.len()).sum::<usize>();
                    dbg_log(format!(
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

        let method_id = self.manifest.method_id_for(service_name);
        let info = self.resolve_rpc_provider_with_retry(service_name).await?;

        let method_uri = UUri::try_from_parts(
            &info.authority,
            info.ue_id as u32,
            info.major_version,
            method_id,
        )
        .map_err(|e| PacomError::Config(format!("Invalid method URI: {e:?}")))?;

        let timeout_ms = rpc_timeout_ms();
        let retry_count = rpc_retry_count();
        let retry_backoff = rpc_retry_backoff();
        let rpc_message_id = UUID::build();
        let rpc_id = next_rpc_diag_id();
        let start = Instant::now();
        rpc_diag_log(format!(
            "client rpc_id={} phase=start service='{}' timeout_ms={} retries={} message_id={} method_uri='{}' payload_len={}",
            rpc_id,
            service_name,
            timeout_ms,
            retry_count,
            rpc_message_id.to_hyphenated_string(),
            method_uri.to_uri(false),
            payload.len()
        ));
        let max_attempts = (retry_count as usize) + 1;

        for attempt in 1..=max_attempts {
            let payload_obj = UPayload::new(payload.clone(), UPayloadFormat::UPAYLOAD_FORMAT_RAW);
            let call_options = CallOptions::for_rpc_request(
                timeout_ms,
                Some(rpc_message_id.clone()),
                None,
                None,
            );

            if attempt > 1 {
                rpc_diag_log(format!(
                    "client rpc_id={} phase=retry_start attempt={} max_attempts={} elapsed_ms={}",
                    rpc_id,
                    attempt,
                    max_attempts,
                    start.elapsed().as_millis()
                ));
            }

            let response = self
                .rpc_client
                .invoke_method(method_uri.clone(), call_options, Some(payload_obj))
                .await;

            match response {
                Ok(Some(p)) => {
                    let response_bytes = p.payload().to_vec();
                    rpc_diag_log(format!(
                        "client rpc_id={} phase=ok attempts={} elapsed_ms={} response_len={}",
                        rpc_id,
                        attempt,
                        start.elapsed().as_millis(),
                        response_bytes.len()
                    ));
                    return Ok(response_bytes);
                }
                Ok(None) => {
                    rpc_diag_log(format!(
                        "client rpc_id={} phase=empty attempts={} elapsed_ms={}",
                        rpc_id,
                        attempt,
                        start.elapsed().as_millis()
                    ));
                    return Err(PacomError::EmptyResponse);
                }
                Err(e) => {
                    let retryable = is_transient_rpc_error(&e);
                    let has_next_attempt = attempt < max_attempts;
                    rpc_diag_log(format!(
                        "client rpc_id={} phase=error attempt={} max_attempts={} elapsed_ms={} retryable={} err={:?}",
                        rpc_id,
                        attempt,
                        max_attempts,
                        start.elapsed().as_millis(),
                        retryable,
                        e
                    ));

                    if retryable && has_next_attempt {
                        sleep(retry_backoff).await;
                        continue;
                    }

                    return Err(PacomError::Config(format!("RPC invocation failed: {e:?}")));
                }
            }
        }

        Err(PacomError::Config(
            "RPC invocation failed: exhausted retry attempts".to_string(),
        ))
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
        let endpoint_source = self.router.get_source_uri();

        let wrapper = Arc::new(ClosureHandler {
            handler,
            rpc_dedup: self.rpc_dedup.clone(),
        });
        self.rpc_server
            .register_endpoint(Some(&endpoint_source), method_id, wrapper)
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
            dbg_log(format!(
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
        dbg_log(format!(
            "resolve_rpc_provider_with_retry waiting service='{}' timeout_ms={} poll_ms={}",
            service_name,
            timeout.as_millis(),
            poll.as_millis()
        ));

        while Instant::now() < deadline {
            sleep(poll).await;
            attempts += 1;
            if let Some(info) = self.lookup_rpc_provider(service_name)? {
                dbg_log(format!(
                    "resolve_rpc_provider_with_retry resolved service='{}' attempts={} provider={}",
                    service_name,
                    attempts,
                    provider_dbg(&info)
                ));
                return Ok(info);
            }
            if attempts % 20 == 0 {
                dbg_log(format!(
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

fn rpc_timeout_ms() -> u32 {
    std::env::var("PACOM_RPC_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(5000)
}

fn rpc_retry_count() -> u8 {
    std::env::var("PACOM_RPC_RETRY_COUNT")
        .ok()
        .and_then(|v| v.parse::<u8>().ok())
        .unwrap_or(1)
}

fn rpc_retry_backoff() -> Duration {
    std::env::var("PACOM_RPC_RETRY_BACKOFF_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_millis(120))
}

fn rpc_dedup_enabled() -> bool {
    std::env::var("PACOM_RPC_DEDUP_ENABLED")
        .ok()
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes" || v == "on"
        })
        .unwrap_or(true)
}

fn rpc_dedup_ttl() -> Duration {
    std::env::var("PACOM_RPC_DEDUP_TTL_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_millis(500))
}

fn rpc_dedup_max_entries() -> usize {
    std::env::var("PACOM_RPC_DEDUP_MAX_ENTRIES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(2048)
}

fn is_transient_rpc_error(err: &ServiceInvocationError) -> bool {
    match err {
        ServiceInvocationError::DeadlineExceeded => true,
        ServiceInvocationError::Unavailable(_) => true,
        ServiceInvocationError::Internal(_) => true,
        ServiceInvocationError::RpcError(status) => {
            matches!(
                status.code.enum_value_or_default(),
                up_rust::UCode::UNAVAILABLE
                    | up_rust::UCode::DEADLINE_EXCEEDED
                    | up_rust::UCode::INTERNAL
                    | up_rust::UCode::UNKNOWN
            )
        }
        _ => false,
    }
}

fn spawn_discovery_reannounce_task(
    router: Arc<PacomRouter>,
    provided_capabilities: Arc<RwLock<ProvidedCapabilities>>,
) {
    tokio::spawn(async move {
        let interval = discovery_reannounce_interval();
        dbg_log(format!(
            "discovery reannounce task started interval_secs={}",
            interval.as_secs()
        ));
        loop {
            sleep(interval).await;

            let (services, topics) = match provided_capabilities.read() {
                Ok(caps) => (
                    caps.rpc_services.iter().cloned().collect::<Vec<_>>(),
                    caps.published_topics.iter().cloned().collect::<Vec<_>>(),
                ),
                Err(_) => continue,
            };

            let local_ue_id = resolve_ue_id();
            let topic_publish_ue_id = RuntimeEngine::derive_topic_publish_ue_id(local_ue_id);
            let channel = (local_ue_id % 16) as u16;
            dbg_log(format!(
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
                        name: service,
                        provider_ue_id: local_ue_id,
                        major_version: 1,
                        provider_authority: authority,
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
                        name: topic,
                        provider_ue_id: topic_publish_ue_id,
                        major_version: 1,
                        provider_authority: authority,
                    };
                    let _ = send_discovery_event(&router, source, event).await;
                }
            }
        }
    });
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

    dbg_log(format!(
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
        Ok(_) => dbg_log("send_discovery_event transport send result=ok"),
        Err(e) => {
            dbg_log(format!(
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
    std::env::var("UP_AUTHORITY").unwrap_or_else(|_| "local_ecu".to_string())
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
    expected_resource_id: u16,
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
            dbg_log(format!(
                "ClosureListener received message: expected_resource_id={}, source_uri={}, sink_uri={}, payload_len={}",
                self.expected_resource_id,
                source_uri,
                sink_uri,
                message.payload.as_ref().map(|p| p.len()).unwrap_or(0)
            ));
        }

        if let Some(attributes) = message.attributes.into_option() {
            if let Some(source) = attributes.source.into_option() {
                if source.resource_id != self.expected_resource_id as u32 {
                    dbg_log(format!(
                        "ClosureListener dropped message: expected_resource_id={}, got_resource_id={}",
                        self.expected_resource_id, source.resource_id
                    ));
                    return; // Ignore messages intended for other topics (MQTT broadcast workaround)
                }
            } else {
                dbg_log("ClosureListener: message has no source attribute");
            }
        } else {
            dbg_log("ClosureListener: message has no attributes");
        }
        if let Some(payload) = message.payload {
            dbg_log(format!(
                "ClosureListener delivering payload: resource_id={}, payload_len={}",
                self.expected_resource_id,
                payload.len()
            ));
            (self.callback)(payload.to_vec());
        } else {
            dbg_log("ClosureListener: message has no payload");
        }
    }
}

/// Internal adapter from uProtocol request handlers to byte closures.
struct ClosureHandler<F> {
    handler: F,
    rpc_dedup: Arc<RpcDedupState>,
}

#[async_trait]
impl<F, Fut> RequestHandler for ClosureHandler<F>
where
    F: Fn(Vec<u8>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Vec<u8>> + Send + 'static,
{
    async fn handle_request(
        &self,
        method_id: u16,
        attributes: &UAttributes,
        request_payload: Option<UPayload>,
    ) -> Result<Option<UPayload>, ServiceInvocationError> {
        let rpc_id = next_rpc_diag_id();
        let start = Instant::now();
        let req_bytes = request_payload
            .map(|p| p.payload().to_vec())
            .unwrap_or_default();

        let source = attributes
            .source
            .as_ref()
            .map(|u| u.to_uri(false))
            .unwrap_or_else(|| "<none>".to_string());
        let sink = attributes
            .sink
            .as_ref()
            .map(|u| u.to_uri(false))
            .unwrap_or_else(|| "<none>".to_string());

        let local_ue = resolve_ue_id() as u32;
        let source_ue = attributes.source.as_ref().map(|u| u.ue_id).unwrap_or_default();

        let dedup_keys: Vec<String> = if self.rpc_dedup.enabled {
            let mut keys = Vec::with_capacity(2);
            if let Some(id) = attributes.id.as_ref() {
                keys.push(format!(
                    "reqid:{}-0x{:04X}",
                    id.to_hyphenated_string(),
                    method_id
                ));
            }

            // Fallback key for anomalous paths where request IDs may not correlate
            // across retries and the incoming source appears as local UE.
            if source_ue == local_ue {
                let sig = payload_signature(&req_bytes);
                keys.push(format!(
                    "fallback:srcue=0x{:08X}:method=0x{:04X}:sig=0x{:016X}",
                    source_ue, method_id, sig
                ));
            }

            keys
        } else {
            Vec::new()
        };

        if !dedup_keys.is_empty() {
            let now = Instant::now();
            let cached = {
                let mut cache = self.rpc_dedup.cache.lock().map_err(|_| {
                    ServiceInvocationError::Internal("failed to lock rpc dedup cache".to_string())
                })?;
                dedup_keys
                    .iter()
                    .find_map(|key| cache.get_valid_response(key, now).map(|resp| (key, resp)))
            };

            if let Some((matched_key, cached_resp)) = cached {
                rpc_diag_log(format!(
                    "server rpc_id={} phase=dedup_replay method_id=0x{:04X} req_key='{}' response_len={}",
                    rpc_id,
                    method_id,
                    matched_key,
                    cached_resp.len()
                ));
                let replay_payload =
                    UPayload::new(cached_resp, UPayloadFormat::UPAYLOAD_FORMAT_RAW);
                return Ok(Some(replay_payload));
            }
        }

        rpc_diag_log(format!(
            "server rpc_id={} phase=start method_id=0x{:04X} source='{}' sink='{}' payload_len={}",
            rpc_id,
            method_id,
            source,
            sink,
            req_bytes.len()
        ));

        if rpc_diag_enabled() {
            if let Some(src) = attributes.source.as_ref() {
                if src.ue_id == local_ue {
                    rpc_diag_log(format!(
                        "WARN rpc_id={} request_source_matches_local_ue local_ue=0x{:04X} source='{}' sink='{}'",
                        rpc_id,
                        local_ue,
                        source,
                        sink
                    ));
                }
            }
        }

        let resp_bytes = (self.handler)(req_bytes).await;

        if !dedup_keys.is_empty() {
            let now = Instant::now();
            let mut cache = self.rpc_dedup.cache.lock().map_err(|_| {
                ServiceInvocationError::Internal("failed to lock rpc dedup cache".to_string())
            })?;
            for key in dedup_keys {
                cache.insert_response(
                    key,
                    resp_bytes.clone(),
                    now,
                    self.rpc_dedup.ttl,
                    self.rpc_dedup.max_entries,
                );
            }
        }

        rpc_diag_log(format!(
            "server rpc_id={} phase=ok method_id=0x{:04X} elapsed_ms={} response_len={}",
            rpc_id,
            method_id,
            start.elapsed().as_millis(),
            resp_bytes.len()
        ));

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
