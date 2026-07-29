use crate::error::PacomError;
use crate::runtime::engine::RuntimeEngine;

/// High-level API that application code should consume.
///
/// This facade intentionally exposes only logical operations
/// (RPC invoke/serve and publish/subscribe).
pub struct PacomRuntime {
    inner: RuntimeEngine,
}

impl PacomRuntime {
    /// Creates a new instance of the PACOM runtime.
    ///
    /// This method initializes the transport layer (vSomeIP and optionally MQTT),
    /// loads the manifest, checks for ID collisions, and starts the background
    /// discovery and routing tasks.
    pub async fn new(config: crate::runtime::RuntimeConfig) -> Result<Self, PacomError> {
        let inner = RuntimeEngine::new(config).await?;
        Ok(Self { inner })
    }

    /// Publishes a fire-and-forget event on the specified logical topic.
    ///
    /// This uses local publish semantics (intra-domain fan-out).
    /// For cross-domain delivery to a specific authority, use `publish_event_to`.
    /// The topic must be declared in the `topics.publish` section of the application manifest.
    pub async fn publish(&self, topic_name: &str, payload: Vec<u8>) -> Result<(), PacomError> {
        self.inner.publish(topic_name, payload).await
    }

    /// Subscribes to events on the specified logical topic.
    ///
    /// The provided callback will be invoked whenever an event is received on the topic.
    /// The topic must be declared in the `topics.subscribe` section of the manifest.
    pub async fn subscribe<F>(&self, topic_path: &str, callback: F) -> Result<(), PacomError>
    where
        F: Fn(Vec<u8>) + Send + Sync + 'static,
    {
        self.inner.subscribe(topic_path, callback).await
    }

    /// Invokes a remote procedure call (RPC) method and waits for the response.
    ///
    /// The method must be declared in the `rpc.consume` section of the manifest.
    /// If no provider is discovered, this method will wait up to the discovery timeout.
    pub async fn call_rpc(
        &self,
        service_name: &str,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, PacomError> {
        self.inner.call_rpc(service_name, payload).await
    }

    /// Alias for `call_rpc`. Invokes a remote procedure call (RPC) method.
    pub async fn invoke_method(
        &self,
        logical_method: &str,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, PacomError> {
        self.inner.call_rpc(logical_method, payload).await
    }

    /// Registers a handler for a remote procedure call (RPC) method.
    ///
    /// The method must be declared in the `rpc.provide` section of the manifest.
    /// The handler receives the request payload and must return the response payload asynchronously.
    pub async fn register_rpc_method<F, Fut>(
        &self,
        service_name: &str,
        handler: F,
    ) -> Result<(), PacomError>
    where
        F: Fn(Vec<u8>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Vec<u8>> + Send + 'static,
    {
        self.inner.register_rpc_method(service_name, handler).await
    }

    /// Alias for `publish`. Publishes an event to a logical topic.
    pub async fn publish_event(
        &self,
        logical_topic: &str,
        payload: Vec<u8>,
    ) -> Result<(), PacomError> {
        self.inner.publish(logical_topic, payload).await
    }

    /// Publishes an event to a logical topic targeted at a specific authority.
    ///
    /// Useful for cross-domain routing (e.g., sending an event to the cloud via the MQTT bridge).
    pub async fn publish_event_to(
        &self,
        logical_topic: &str,
        target_authority: &str,
        payload: Vec<u8>,
    ) -> Result<(), PacomError> {
        self.inner
            .publish_to_authority(logical_topic, target_authority, payload)
            .await
    }

    /// Alias for `subscribe`. Subscribes to events on a logical topic.
    pub async fn subscribe_event<F>(
        &self,
        logical_topic: &str,
        callback: F,
    ) -> Result<(), PacomError>
    where
        F: Fn(Vec<u8>) + Send + Sync + 'static,
    {
        self.inner.subscribe(logical_topic, callback).await
    }

    /// Subscribes to events on a logical topic from a specific source authority.
    ///
    /// Useful for receiving events from cross-domain sources (e.g., from the cloud via MQTT).
    pub async fn subscribe_event_from<F>(
        &self,
        logical_topic: &str,
        source_authority: &str,
        callback: F,
    ) -> Result<(), PacomError>
    where
        F: Fn(Vec<u8>) + Send + Sync + 'static,
    {
        self.inner
            .subscribe_from_authority(logical_topic, source_authority, callback)
            .await
    }

    /// Gracefully stops PACOM background tasks.
    pub async fn shutdown(&self) -> Result<(), PacomError> {
        self.inner.shutdown().await
    }
}

impl From<RuntimeEngine> for PacomRuntime {
    fn from(inner: RuntimeEngine) -> Self {
        Self { inner }
    }
}
