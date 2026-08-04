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

    /// Publishes a fire-and-forget event on the specified logical topic.
    ///
    /// This uses logical publish semantics and lets the runtime decide
    /// whether the event stays local or must be routed to the cloud.
    /// The topic must be declared in the `topics.publish` section of the application manifest.
    pub async fn publish_event(
        &self,
        logical_topic: &str,
        payload: Vec<u8>,
    ) -> Result<(), PacomError> {
        self.inner.publish(logical_topic, payload).await
    }

    /// Subscribes to events on the specified logical topic.
    ///
    /// The provided callback will be invoked whenever an event is received on the topic.
    /// The topic must be declared in the `topics.subscribe` section of the manifest.
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

    /// Invokes a remote procedure call (RPC) method and waits for the response.
    ///
    /// The method must be declared in the `rpc.consume` section of the manifest.
    /// If no provider is discovered, this method will wait up to the discovery timeout.
    pub async fn invoke_rpc_method(
        &self,
        logical_method: &str,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, PacomError> {
        self.inner.call_rpc(logical_method, payload).await
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
