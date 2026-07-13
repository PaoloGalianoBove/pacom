use std::sync::Arc;
use async_trait::async_trait;
use up_rust::{
    UListener, UMessage, UMessageBuilder, UPayloadFormat, UStatus, UCode, UTransport, UUri, LocalUriProvider, UAttributes
};
use up_rust::communication::{
    CallOptions, InMemoryRpcClient, InMemoryRpcServer, RequestHandler, RpcClient, RpcServer, UPayload, ServiceInvocationError
};
use crate::l1::{vsomeip, mqtt, router::UStreamerRouter};
use super::catalog;

/// Optional MQTT 5 broker configuration for telemetry.
#[derive(Clone, Debug)]
pub struct MqttConfig {
    pub broker_uri: String,
    pub client_id: String,
}

/// SdkConfig containing optional MQTT connection parameters.
#[derive(Clone, Debug)]
pub struct SdkConfig {
    /// Optional MQTT 5 broker options for cloud-bound messaging
    pub mqtt_config: Option<MqttConfig>,
}

/// The unified uProtocol client for the Software-Defined Vehicle applications.
/// Dynamically coordinates vSomeIP (intra-vehicle) and MQTT (cloud) transports,
/// and fully encapsulates the Layer 2 (L2) communication abstractions.
pub struct PlatformClient {
    router: Arc<UStreamerRouter>,
    rpc_client: Arc<InMemoryRpcClient>,
    rpc_server: Arc<InMemoryRpcServer>,
}

impl PlatformClient {
    /// Initalizes the PlatformClient by dynamically negotiating local vsomeip roles,
    /// resolving the authority and application ID from the environment,
    /// and optionally connecting to the remote MQTT 5 broker.
    pub async fn new(config: SdkConfig) -> Result<Self, UStatus> {
        // 1. Resolve authority/ECU name and application UE_ID dynamically from environment
        let authority = resolve_authority();
        let ue_id = resolve_ue_id();

        // 2. Set up the local vSomeIP transport (Router or Client)
        let vsomeip_transport = vsomeip::setup_vsomeip_transport(ue_id, &authority).await?;

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
            authority,
            vsomeip_transport,
            mqtt_transport,
        ));

        // 5. Initialize the Layer 2 (L2) RPC Client and Server components
        let rpc_client = Arc::new(
            InMemoryRpcClient::new(router.clone(), router.clone())
                .await
                .map_err(|e| {
                    UStatus::fail_with_code(
                        UCode::INTERNAL,
                        format!("Failed to initialize InMemoryRpcClient: {e:?}"),
                    )
                })?
        );
        let rpc_server = Arc::new(InMemoryRpcServer::new(router.clone(), router.clone()));

        Ok(Self {
            router,
            rpc_client,
            rpc_server,
        })
    }

    /// Publishes raw bytes payload to a logical topic name.
    /// The topic is dynamically mapped to its resource ID and published locally.
    pub async fn publish(&self, topic_name: &str, payload: Vec<u8>) -> Result<(), String> {
        let resource_id = catalog::get_topic_resource_id(topic_name)
            .ok_or_else(|| format!("Unknown topic name: {topic_name}"))?;
            
        let local_authority = self.router.get_authority();
        let local_ue_id = resolve_ue_id();
        
        let uri = UUri::try_from_parts(&local_authority, local_ue_id as u32, 1, resource_id)
            .map_err(|e| format!("Invalid topic URI: {e:?}"))?;
            
        let msg = UMessageBuilder::publish(uri)
            .build_with_payload(payload, UPayloadFormat::UPAYLOAD_FORMAT_RAW)
            .map_err(|e| format!("Failed to build message: {e:?}"))?;
            
        self.router.send(msg).await.map_err(|e| format!("Failed to send: {e:?}"))
    }

    /// Subscribes a generic closure callback to receive raw bytes from a logical topic path.
    /// Format: "service-name/topic-name" (e.g. "light-switch/light-status").
    pub async fn subscribe<F>(&self, topic_path: &str, callback: F) -> Result<(), String>
    where
        F: Fn(Vec<u8>) + Send + Sync + 'static,
    {
        let local_authority = self.router.get_authority();
        let uri = resolve_subscribe_topic(topic_path, &local_authority)?;
        let listener = Arc::new(ClosureListener {
            callback: Box::new(callback),
        });
        self.router.register_listener(&uri, None, listener).await.map_err(|e| format!("Failed to subscribe: {e:?}"))
    }

    /// Invokes a remote procedure call (RPC) on a target service name, returning raw response bytes.
    pub async fn call_rpc(&self, service_name: &str, payload: Vec<u8>) -> Result<Vec<u8>, String> {
        let (ue_id, method_id) = catalog::get_service_mapping(service_name)
            .ok_or_else(|| format!("Unknown service name: {service_name}"))?;
            
        let target_authority = self.router.get_authority();
        
        let method_uri = UUri::try_from_parts(&target_authority, ue_id as u32, 0, method_id)
            .map_err(|e| format!("Invalid method URI: {e:?}"))?;
            
        let payload_obj = UPayload::new(payload, UPayloadFormat::UPAYLOAD_FORMAT_RAW);
        let call_options = CallOptions::for_rpc_request(5000, None, None, None);
        
        let response = self.rpc_client.invoke_method(method_uri, call_options, Some(payload_obj))
            .await
            .map_err(|e| format!("RPC invocation failed: {e:?}"))?;
            
        match response {
            Some(p) => Ok(p.payload().to_vec()),
            None => Err("RPC returned empty response".to_string()),
        }
    }

    /// Registers an asynchronous handler closure for a remote procedure call (RPC) served by this application.
    pub async fn register_rpc_method<F, Fut>(&self, service_name: &str, handler: F) -> Result<(), String>
    where
        F: Fn(Vec<u8>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Vec<u8>> + Send + 'static,
    {
        let (_, method_id) = catalog::get_service_mapping(service_name)
            .ok_or_else(|| format!("Unknown service name: {service_name}"))?;
            
        let wrapper = Arc::new(ClosureHandler { handler });
        self.rpc_server.register_endpoint(None, method_id, wrapper).await.map_err(|e| format!("Failed to register RPC: {e:?}"))
    }

    /// Exposes the underlying router as an Arc<dyn UTransport>
    pub fn transport(&self) -> Arc<dyn UTransport> {
        self.router.clone()
    }
}

/// Helper function to resolve the node/ECU authority name from the environment.
fn resolve_authority() -> String {
    std::env::var("UP_AUTHORITY").unwrap_or_else(|_| "local_ecu".to_string())
}

/// Helper function to resolve or dynamically hash the application UE_ID from the environment.
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

/// Helper to parse "service-name/topic-name" and resolve it to a subscription UUri.
fn resolve_subscribe_topic(topic_path: &str, local_authority: &str) -> Result<UUri, String> {
    let parts: Vec<&str> = topic_path.split('/').collect();
    if parts.len() != 2 {
        return Err("Topic path must be in format 'service-name/topic-name'".to_string());
    }
    let service_name = parts[0];
    let topic_name = parts[1];
    
    let (ue_id, _) = catalog::get_service_mapping(service_name)
        .ok_or_else(|| format!("Unknown service name: {service_name}"))?;
        
    let resource_id = catalog::get_topic_resource_id(topic_name)
        .ok_or_else(|| format!("Unknown topic name: {topic_name}"))?;
        
    UUri::try_from_parts(local_authority, ue_id as u32, 1, resource_id)
        .map_err(|e| format!("Failed to create subscription UUri: {e:?}"))
}

/// Internal wrapper to map uProtocol UListener events to simple raw bytes callback closures.
struct ClosureListener {
    callback: Box<dyn Fn(Vec<u8>) + Send + Sync + 'static>,
}

#[async_trait]
impl UListener for ClosureListener {
    async fn on_receive(&self, message: UMessage) {
        if let Some(payload) = message.payload {
            (self.callback)(payload.to_vec());
        }
    }
}

/// Internal wrapper to map uProtocol RequestHandler events to simple raw bytes handler closures.
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
impl UTransport for PlatformClient {
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

impl LocalUriProvider for PlatformClient {
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
