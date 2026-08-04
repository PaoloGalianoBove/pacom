//! pacom exposes a high-level runtime API over uProtocol transports.

#![warn(missing_docs)]
/// Error definitions for the PACOM runtime.
pub mod error;
mod public_api;
mod runtime;
mod transport;
/// Internal environment and debug helpers shared across runtime and transport code.
pub mod utils;

// Re-export the public API and core runtime types at the crate root.
pub use error::PacomError;
pub use public_api::PacomRuntime;
pub use runtime::{ManifestConfig, MqttConfig, RuntimeConfig};
