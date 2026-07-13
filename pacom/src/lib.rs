pub mod l1;
pub mod l2;

// Re-export Layer 2 high-level entities directly at the crate root
pub use l2::{PlatformClient, SdkConfig, MqttConfig};
