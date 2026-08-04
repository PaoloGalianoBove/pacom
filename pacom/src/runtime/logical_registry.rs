use crate::error::PacomError;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RpcManifest {
    #[serde(default)]
    pub provide: HashSet<String>,
    #[serde(default)]
    pub consume: HashSet<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TopicManifest {
    #[serde(default)]
    pub publish: HashSet<String>,
    #[serde(default)]
    pub subscribe: HashSet<String>,
}

/// Per-instance manifest describing which RPC methods and event topics
/// this application provides, consumes, publishes or subscribes to.
///
/// Each `PacomRuntime` instance loads its own `ManifestConfig`,
/// allowing multiple runtimes in the same process (or across
/// processes inside the same container).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ManifestConfig {
    #[serde(default)]
    /// RPC methods provided and consumed by the application.
    pub rpc: RpcManifest,
    #[serde(default)]
    /// Topics published and subscribed to by the application.
    pub topics: TopicManifest,
    #[serde(skip)]
    /// Resolved numeric method IDs for RPCs declared in `rpc.provide`.
    pub resolved_rpc_ids: HashMap<String, u16>,
    #[serde(skip)]
    /// Resolved numeric resource IDs for topics declared in `topics.publish`.
    pub resolved_topic_ids: HashMap<String, u16>,
}

impl ManifestConfig {
    /// Loads a manifest from the given filesystem path.
    /// Returns a default (empty) manifest on any I/O or parse failure,
    /// and logs the reason to ease diagnostics.
    pub fn load(path: &str) -> Self {
        let mut config = match std::fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<ManifestConfig>(&content) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "[PACOM][Manifest] Invalid JSON in '{}': {}. Falling back to empty manifest.",
                        path, e
                    );
                    ManifestConfig::default()
                }
            },
            Err(e) => {
                eprintln!(
                    "[PACOM][Manifest] Failed to read '{}': {}. Falling back to empty manifest.",
                    path, e
                );
                ManifestConfig::default()
            }
        };
        config.resolve_and_store_ids();
        config
    }

    /// Loads the manifest from the `PACOM_MANIFEST_PATH` environment variable,
    /// falling back to `/etc/pacom/manifest.json` if unset.
    pub fn load_from_env() -> Self {
        let path = std::env::var("PACOM_MANIFEST_PATH")
            .unwrap_or_else(|_| "/etc/pacom/manifest.json".to_string());
        Self::load(&path)
    }

    // ── ID Resolution ──────────────────────────────────────────

    /// Resolves and stores deterministic IDs for locally provided RPCs and published topics.
    ///
    /// When two names hash to the same value inside the same semispazio, PACOM
    /// increments the candidate ID until it finds a free slot.
    pub fn resolve_and_store_ids(&mut self) {
        let mut rpc_used = HashSet::new();
        for name in &self.rpc.provide {
            let mut id = stable_id16("rpc", name) & 0x7FFF;
            if id == 0 { id = 1; }
            while rpc_used.contains(&id) {
                id = (id.wrapping_add(1)) & 0x7FFF;
                if id == 0 { id = 1; }
            }
            rpc_used.insert(id);
            self.resolved_rpc_ids.insert(name.clone(), id);
        }

        let mut topic_used = HashSet::new();
        for name in &self.topics.publish {
            let mut id = stable_id16("topic", name) | 0x8000;
            while topic_used.contains(&id) {
                id = (id.wrapping_add(1)) | 0x8000;
            }
            topic_used.insert(id);
            self.resolved_topic_ids.insert(name.clone(), id);
        }
    }

    // ── Lookup helpers ─────────────────────────────────────────

    /// Checks if a given RPC method name is declared in the `provide` section.
    pub fn is_rpc_provided(&self, name: &str) -> bool {
        self.rpc.provide.contains(name.trim())
    }

    /// Checks if a given RPC method name is declared in the `consume` section.
    pub fn is_rpc_consumed(&self, name: &str) -> bool {
        self.rpc.consume.contains(name.trim())
    }

    /// Checks if a given topic name is declared in the `publish` section.
    pub fn is_topic_published(&self, name: &str) -> bool {
        self.topics.publish.contains(name.trim())
    }

    /// Checks if a given topic name is declared in the `subscribe` section.
    pub fn is_topic_subscribed(&self, name: &str) -> bool {
        self.topics.subscribe.contains(name.trim())
    }

    // ── ID generation ──────────────────────────────────────────

    /// Returns a deterministic method ID in `[0x0001, 0x7FFF]` for an RPC name.
    pub fn method_id_for(&self, logical_method: &str) -> u16 {
        if let Some(&id) = self.resolved_rpc_ids.get(logical_method) {
            return id;
        }
        let id = stable_id16("rpc", logical_method) & 0x7FFF;
        if id == 0 { 1 } else { id }
    }

    /// Returns a deterministic resource ID in `[0x8000, 0xFFFF]` for a topic name.
    pub fn resource_id_for(&self, logical_topic: &str) -> u16 {
        if let Some(&id) = self.resolved_topic_ids.get(logical_topic) {
            return id;
        }
        stable_id16("topic", logical_topic) | 0x8000
    }

    // ── Collision detection (Deprecated) ───────────────────────

    /// Kept for compatibility with `engine.rs` startup flow.
    pub fn validate_no_collisions(&self) -> Result<(), PacomError> {
        Ok(())
    }
}

/// FNV-1a hash folded to 16 bits, producing a deterministic
/// mapping from `(namespace, logical_name)` → `u16`.
fn stable_id16(namespace: &str, logical_name: &str) -> u16 {
    let mut hash = 0x811c9dc5u32;
    for byte in namespace.bytes().chain(logical_name.bytes()) {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    let folded = (hash ^ (hash >> 16)) as u16;
    if folded == 0 { 1 } else { folded }
}
