use std::sync::OnceLock;
use std::collections::HashMap;

/// Dynamic name resolution for services, loading from /etc/pacom/services.json with a built-in fallback.
pub fn get_service_mapping(name: &str) -> Option<(u16, u16)> {
    static MAPPINGS: OnceLock<HashMap<String, (u16, u16)>> = OnceLock::new();
    let map = MAPPINGS.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("light-switch".to_string(), (0x1234, 1));
        if let Ok(content) = std::fs::read_to_string("/etc/pacom/services.json") {
            if let Ok(custom_map) = serde_json::from_str::<HashMap<String, (u16, u16)>>(&content) {
                m.extend(custom_map);
            }
        }
        m
    });
    map.get(name).copied()
}

/// Dynamic name resolution for topics, loading from /etc/pacom/topics.json with a built-in fallback.
pub fn get_topic_resource_id(name: &str) -> Option<u16> {
    static TOPICS: OnceLock<HashMap<String, u16>> = OnceLock::new();
    let map = TOPICS.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("light-status".to_string(), 0x8001);
        if let Ok(content) = std::fs::read_to_string("/etc/pacom/topics.json") {
            if let Ok(custom_map) = serde_json::from_str::<HashMap<String, u16>>(&content) {
                m.extend(custom_map);
            }
        }
        m
    });
    map.get(name).copied()
}
