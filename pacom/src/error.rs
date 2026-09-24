use up_rust::{UCode, UStatus};

/// Unified error type for all PACOM runtime operations.
///
/// Replaces ad-hoc `String` errors with structured variants
/// that carry semantic meaning and enable pattern matching.
#[derive(Debug, thiserror::Error)]
pub enum PacomError {
    /// An error propagated from the underlying uProtocol transport layer.
    #[error("Transport error: {0}")]
    Transport(#[from] UStatus),

    /// The requested operation is not declared in the application manifest.
    #[error("Manifest violation: operation '{operation}' not declared for '{name}'")]
    ManifestViolation {
        /// The type of operation (e.g., 'provide', 'consume', 'publish', 'subscribe').
        operation: String,
        /// The logical name of the topic or method.
        name: String,
    },

    /// No provider was discovered for the requested service within the timeout.
    #[error("Discovery timeout: no provider for '{name}' within {timeout_ms}ms")]
    DiscoveryTimeout {
        /// The logical name of the method that timed out.
        name: String,
        /// The timeout duration in milliseconds.
        timeout_ms: u64,
    },

    /// A configuration or message-building error.
    #[error("Configuration error: {0}")]
    Config(String),

    /// Two logical names in the same manifest resolve to the same numeric ID.
    #[error("ID collision: '{name_a}' and '{name_b}' both resolve to ID {id:#06x}")]
    IdCollision {
        /// The first logical name.
        name_a: String,
        /// The second logical name.
        name_b: String,
        /// The colliding numeric ID.
        id: u16,
    },

    /// An RPC invocation returned an empty response payload.
    #[error("RPC returned empty response")]
    EmptyResponse,

    /// An RPC response was not received within the configured timeout.
    #[error("RPC timeout: no response received for '{name}' within {timeout_ms}ms")]
    RpcTimeout {
        /// The logical name of the method.
        name: String,
        /// The timeout duration configured for this call.
        timeout_ms: u32,
    },

    /// An RPC invocation failed at runtime.
    #[error("RPC system error: invocation failed for '{name}' within {timeout_ms}ms: {message}")]
    RpcError {
        /// The logical name of the method.
        name: String,
        /// The timeout duration configured for this call.
        timeout_ms: u32,
        /// The underlying error message (e.g., from uProtocol).
        message: String,
    },
}

impl From<PacomError> for UStatus {
    fn from(error: PacomError) -> Self {
        match error {
            PacomError::Transport(status) => status,
            PacomError::ManifestViolation { .. } => {
                UStatus::fail_with_code(UCode::PERMISSION_DENIED, error.to_string())
            }
            PacomError::DiscoveryTimeout { .. } => {
                UStatus::fail_with_code(UCode::DEADLINE_EXCEEDED, error.to_string())
            }
            PacomError::Config(_) => UStatus::fail_with_code(UCode::INTERNAL, error.to_string()),
            PacomError::IdCollision { .. } => {
                UStatus::fail_with_code(UCode::ALREADY_EXISTS, error.to_string())
            }
            PacomError::EmptyResponse => {
                UStatus::fail_with_code(UCode::NOT_FOUND, error.to_string())
            }
            PacomError::RpcTimeout { .. } => {
                UStatus::fail_with_code(UCode::DEADLINE_EXCEEDED, error.to_string())
            }
            PacomError::RpcError { .. } => {
                UStatus::fail_with_code(UCode::UNAVAILABLE, error.to_string())
            }
        }
    }
}
