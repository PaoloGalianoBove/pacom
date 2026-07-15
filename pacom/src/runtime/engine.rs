use std::sync::Arc;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::RwLock;
use std::time::{Duration, Instant};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use up_rust::{
    UListener, UMessage, UMessageBuilder, UPayloadFormat, UStatus, 
    UTransport, UUri, LocalUriProvider, UAttributes
};
use up_rust::communication::{
    CallOptions, InMemoryRpcClient, InMemoryRpcServer, RequestHandler, RpcClient, RpcServer, UPayload, ServiceInvocationError
};
use tokio::time::sleep;
use crate::error::PacomError;
use crate::transport::{vsomeip, mqtt, router::UStreamerRouter};
use super::logical_registry::ManifestConfig;

const DISCOVERY_UE_ID: u16 = 0x0F00;
const DISCOVERY_RESOURCE_ID: u16 = 0x8F01;

#[derive(Default)]
struct DiscoveryCache {
    rpc_providers: HashMap<String, u16>,
    topic_publishers: HashMap<String, u16>,
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
}

struct DiscoveryListener {
    cache: Arc<RwLock<DiscoveryCache>>,
}

#[async_trait]
impl UListener for DiscoveryListener {
    async fn on_receive(&self, message: UMessage) {
        if let Some(payload) = message.payload {
            if let Ok(event) = serde_json::from_slice::<DiscoveryEvent>(&payload) {
                if let Ok(mut cache) = self.cache.write() {
                    match event.kind.as_str() {
                        "rpc_provide" => {
                            cache.rpc_providers.insert(event.name, event.provider_ue_id);
                        }
                        "topic_publish" => {
                            cache.topic_publishers.insert(event.name, event.provider_ue_id);
                        }
                        _ => {}
                    }
                }
            }
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
    router: Arc<UStreamerRouter>,
    rpc_client: Arc<InMemoryRpcClient>,
    rpc_server: Arc<InMemoryRpcServer>,
    discovery_cache: Arc<RwLock<DiscoveryCache>>,
    provided_capabilities: Arc<RwLock<ProvidedCapabilities>>,
    manifest: ManifestConfig,
}

impl RuntimeEngine {
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

        // 2. Set up the local vSomeIP transport (Router or Client) if not disabled
        let vsomeip_transport = if std::env::var("PACOM_DISABLE_VSOMEIP").unwrap_or_else(|_| "false".to_string()) != "true" {
            Some(vsomeip::setup_vsomeip_transport(ue_id, &authority).await?)
        } else {
            None
        };

        // 3. Set up the optional MQTT 5 transport
        let mqtt_transport = if let Some(mqtt_cfg) = config.mqtt_config {
            let mqtt = mqtt::setup_mqtt_transport(
                &mqtt_cfg.broker_uri,
                &mqtt_cfg.client_id,
                &authority,
            ).await?;
            Some(mqtt)
        } else {
            None
        };

        // 4. Wrap both inside the uStreamer router
        let router = Arc::new(UStreamerRouter::new(
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
                })?
        );
        let rpc_server = Arc::new(InMemoryRpcServer::new(router.clone(), router.clone()));

        let discovery_cache = Arc::new(RwLock::new(DiscoveryCache::default()));
        let provided_capabilities = Arc::new(RwLock::new(ProvidedCapabilities {
            rpc_services: manifest.rpc.provide.clone(),
            published_topics: manifest.topics.publish.clone(),
        }));

        // Due to a bug in up-transport-vsomeip v0.1.1 which hardcodes instance_id = 1
        // and doesn't support wildcards, we cannot use a single DISCOVERY_UE_ID (0x0F00).
        // Instead, we shard the discovery service across 16 different UE_IDs (0x0F00 - 0x0F0F).
        // Each node subscribes to all 16 channels to receive discovery events.
        let discovery_listener = Arc::new(DiscoveryListener {
            cache: discovery_cache.clone(),
        });

        for i in 0..16 {
            let discovery_uri = UUri::try_from_parts(&authority, (DISCOVERY_UE_ID + i) as u32, 1, DISCOVERY_RESOURCE_ID)
                .map_err(|e| PacomError::Config(format!("Failed to build discovery URI: {e:?}")))?;
            router
                .register_listener(&discovery_uri, None, discovery_listener.clone())
                .await?;
        }

        spawn_discovery_reannounce_task(router.clone(), provided_capabilities.clone());

        Ok(Self {
            router,
            rpc_client,
            rpc_server,
            discovery_cache,
            provided_capabilities,
            manifest,
        })
    }

    async fn announce_discovery(&self, kind: &str, name: &str) -> Result<(), PacomError> {
        let authority = self.router.get_authority();
        let local_ue_id = resolve_ue_id();
        let channel = (local_ue_id % 16) as u16;
        
        let source = UUri::try_from_parts(&authority, (DISCOVERY_UE_ID + channel) as u32, 1, DISCOVERY_RESOURCE_ID)
            .map_err(|e| PacomError::Config(format!("Invalid discovery URI: {e:?}")))?;

        let event = DiscoveryEvent {
            kind: kind.to_string(),
            name: name.to_string(),
            provider_ue_id: local_ue_id,
        };

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
            let mut caps = self
                .provided_capabilities
                .write()
                .map_err(|_| PacomError::Config("Failed to update provided capabilities".to_string()))?;
            caps.published_topics.insert(topic_name.to_string());
        }

        let resource_id = self.manifest.resource_id_for(topic_name);
            
        let local_authority = self.router.get_authority();
        let local_ue_id = resolve_ue_id();
        
        let uri = UUri::try_from_parts(&local_authority, local_ue_id as u32, 1, resource_id)
            .map_err(|e| PacomError::Config(format!("Invalid topic URI: {e:?}")))?;
            
        let msg = UMessageBuilder::publish(uri)
            .build_with_payload(payload, UPayloadFormat::UPAYLOAD_FORMAT_RAW)
            .map_err(|e| PacomError::Config(format!("Failed to build message: {e:?}")))?;
            
        self.router.send(msg).await?;
        self.announce_discovery("topic_publish", topic_name).await?;
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

        self.router.send(msg).await?;
        Ok(())
    }

    /// Subscribes a closure callback to receive raw bytes from a logical topic key.
    pub async fn subscribe<F>(&self, topic_name: &str, callback: F) -> Result<(), PacomError>
    where
        F: Fn(Vec<u8>) + Send + Sync + 'static,
    {
        let local_authority = self.router.get_authority();
        let uri = self
            .resolve_subscribe_topic_for_authority(topic_name, &local_authority)
            .await?;
        let listener = Arc::new(ClosureListener {
            expected_resource_id: self.manifest.resource_id_for(topic_name),
            callback: Box::new(callback),
        });

        self.router.register_listener(&uri, None, listener).await?;
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
        let uri = self
            .resolve_subscribe_topic_for_authority(topic_name, authority)
            .await?;

        let listener = Arc::new(ClosureListener {
            expected_resource_id: self.manifest.resource_id_for(topic_name),
            callback: Box::new(callback),
        });

        self.router.register_listener(&uri, None, listener).await?;
        Ok(())
    }

    /// Invokes an RPC method identified by a logical service name.
    pub async fn call_rpc(&self, service_name: &str, payload: Vec<u8>) -> Result<Vec<u8>, PacomError> {
        if !self.manifest.is_rpc_consumed(service_name) {
            return Err(PacomError::ManifestViolation {
                operation: "rpc.consume".to_string(),
                name: service_name.to_string(),
            });
        }

        let method_id = self.manifest.method_id_for(service_name);

        let ue_id = self.resolve_rpc_provider_with_retry(service_name).await?;
            
        let target_authority = self.router.get_authority();
        
        let method_uri = UUri::try_from_parts(&target_authority, ue_id as u32, 0, method_id)
            .map_err(|e| PacomError::Config(format!("Invalid method URI: {e:?}")))?;
            
        let payload_obj = UPayload::new(payload, UPayloadFormat::UPAYLOAD_FORMAT_RAW);
        let call_options = CallOptions::for_rpc_request(5000, None, None, None);
        
        let response = self.rpc_client.invoke_method(method_uri, call_options, Some(payload_obj))
            .await
            .map_err(|e| PacomError::Config(format!("RPC invocation failed: {e:?}")))?;
            
        match response {
            Some(p) => Ok(p.payload().to_vec()),
            None => Err(PacomError::EmptyResponse),
        }
    }

    /// Registers an asynchronous handler for an RPC method served by this process.
    pub async fn register_rpc_method<F, Fut>(&self, service_name: &str, handler: F) -> Result<(), PacomError>
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
        self
            .rpc_server
            .register_endpoint(None, method_id, wrapper)
            .await
            .map_err(|e| PacomError::Config(format!("Failed to register RPC: {e:?}")))?;

        {
            let mut caps = self
                .provided_capabilities
                .write()
                .map_err(|_| PacomError::Config("Failed to update provided capabilities".to_string()))?;
            caps.rpc_services.insert(service_name.to_string());
        }

        self.announce_discovery("rpc_provide", service_name).await
    }

    async fn resolve_subscribe_topic_for_authority(&self, topic_name: &str, authority: &str) -> Result<UUri, PacomError> {
        if !self.manifest.is_topic_subscribed(topic_name) {
            return Err(PacomError::ManifestViolation {
                operation: "topics.subscribe".to_string(),
                name: topic_name.to_string(),
            });
        }

        let resource_id = self.manifest.resource_id_for(topic_name);
        let local_authority = self.router.get_authority();

        let ue_id = if authority == local_authority {
            self.resolve_topic_publisher_with_retry(topic_name).await?
        } else {
            0xFFFF // Wildcard UE_ID for external subscriptions
        };

        UUri::try_from_parts(authority, ue_id as u32, 0xFF, resource_id)
            .map_err(|e| PacomError::Config(format!("Failed to create subscription UUri: {e:?}")))
    }

    async fn resolve_rpc_provider_with_retry(&self, service_name: &str) -> Result<u16, PacomError> {
        if let Some(ue_id) = self.lookup_rpc_provider(service_name)? {
            return Ok(ue_id);
        }

        let timeout = discovery_wait_timeout();
        let poll = discovery_poll_interval();
        let deadline = Instant::now() + timeout;

        while Instant::now() < deadline {
            sleep(poll).await;
            if let Some(ue_id) = self.lookup_rpc_provider(service_name)? {
                return Ok(ue_id);
            }
        }

        Err(PacomError::DiscoveryTimeout {
            name: service_name.to_string(),
            timeout_ms: timeout.as_millis() as u64,
        })
    }

    async fn resolve_topic_publisher_with_retry(&self, topic_name: &str) -> Result<u16, PacomError> {
        if let Some(ue_id) = self.lookup_topic_publisher(topic_name)? {
            return Ok(ue_id);
        }

        let timeout = discovery_wait_timeout();
        let poll = discovery_poll_interval();
        let deadline = Instant::now() + timeout;

        while Instant::now() < deadline {
            sleep(poll).await;
            if let Some(ue_id) = self.lookup_topic_publisher(topic_name)? {
                return Ok(ue_id);
            }
        }

        Err(PacomError::DiscoveryTimeout {
            name: topic_name.to_string(),
            timeout_ms: timeout.as_millis() as u64,
        })
    }

    fn lookup_rpc_provider(&self, service_name: &str) -> Result<Option<u16>, PacomError> {
        let cache = self
            .discovery_cache
            .read()
            .map_err(|_| PacomError::Config("Failed to read discovery cache".to_string()))?;
        Ok(cache.rpc_providers.get(service_name).copied())
    }

    fn lookup_topic_publisher(&self, topic_name: &str) -> Result<Option<u16>, PacomError> {
        // 1. Check for statically configured routes via Environment Variables
        // Example: Topic "/bridge/down" -> Env Var: PACOM_STATIC_ROUTE_bridge_down=0x3301
        let env_key = format!("PACOM_STATIC_ROUTE_{}", topic_name.trim_start_matches('/').replace('/', "_"));
        if let Ok(val) = std::env::var(&env_key) {
            let parsed = if val.starts_with("0x") {
                u16::from_str_radix(val.trim_start_matches("0x"), 16)
            } else {
                val.parse::<u16>()
            };
            if let Ok(ue_id) = parsed {
                return Ok(Some(ue_id));
            }
        }

        // 2. Check the dynamic Discovery Cache
        let cache = self
            .discovery_cache
            .read()
            .map_err(|_| PacomError::Config("Failed to read discovery cache".to_string()))?;
        Ok(cache.topic_publishers.get(topic_name).copied())
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

fn resolve_cloud_topic_ue_id() -> u16 {
    std::env::var("PACOM_CLOUD_UE_ID")
        .ok()
        .and_then(|val| {
            if val.starts_with("0x") {
                u16::from_str_radix(&val[2..], 16).ok()
            } else {
                val.parse::<u16>().ok()
            }
        })
        .unwrap_or(0x2200)
}

fn spawn_discovery_reannounce_task(
    router: Arc<UStreamerRouter>,
    provided_capabilities: Arc<RwLock<ProvidedCapabilities>>,
) {
    tokio::spawn(async move {
        let interval = discovery_reannounce_interval();
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
            let channel = (local_ue_id % 16) as u16;

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
                        provider_ue_id: local_ue_id,
                    };
                    let _ = send_discovery_event(&router, source, event).await;
                }
            }
        }
    });
}

async fn send_discovery_event(
    router: &Arc<UStreamerRouter>,
    source: UUri,
    event: DiscoveryEvent,
) -> Result<(), PacomError> {
    let payload = serde_json::to_vec(&event)
        .map_err(|e| PacomError::Config(format!("Failed to serialize discovery event: {e}")))?;

    let msg = UMessageBuilder::publish(source)
        .build_with_payload(payload, UPayloadFormat::UPAYLOAD_FORMAT_RAW)
        .map_err(|e| PacomError::Config(format!("Failed to build discovery message: {e:?}")))?;

    router.send(msg).await?;
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
            if val < 0x1000 {
                val + 0x1000
            } else {
                val
            }
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
        if let Some(attributes) = message.attributes.into_option() {
            if let Some(source) = attributes.source.into_option() {
                if source.resource_id != self.expected_resource_id as u32 {
                    return; // Ignore messages intended for other topics (MQTT broadcast workaround)
                }
            }
        }
        if let Some(payload) = message.payload {
            (self.callback)(payload.to_vec());
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
        self.router.register_listener(source_filter, sink_filter, listener).await
    }

    async fn unregister_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        self.router.unregister_listener(source_filter, sink_filter, listener).await
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
