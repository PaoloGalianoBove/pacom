use pacom::ManifestConfig;

#[test]
fn test_empty_manifest_defaults() {
    let manifest = ManifestConfig::default();
    assert!(!manifest.is_rpc_provided("/rpc/test"));
    assert!(!manifest.is_rpc_consumed("/rpc/test"));
    assert!(!manifest.is_topic_published("/topic/test"));
    assert!(!manifest.is_topic_subscribed("/topic/test"));
}

#[test]
fn test_is_rpc_provided() {
    let mut manifest = ManifestConfig::default();
    manifest.rpc.provide.insert("/rpc/echo".to_string());

    assert!(manifest.is_rpc_provided("/rpc/echo"));
    assert!(!manifest.is_rpc_provided("/rpc/other"));
}

#[test]
fn test_is_topic_published() {
    let mut manifest = ManifestConfig::default();
    manifest.topics.publish.insert("/topic/temp".to_string());

    assert!(manifest.is_topic_published("/topic/temp"));
    assert!(!manifest.is_topic_published("/topic/other"));
}

#[test]
fn test_method_id_range() {
    let manifest = ManifestConfig::default();
    let id1 = manifest.method_id_for("/rpc/test1");
    let id2 = manifest.method_id_for("/rpc/test2");

    assert!(id1 >= 0x0001 && id1 <= 0x7FFF);
    assert!(id2 >= 0x0001 && id2 <= 0x7FFF);
    assert_ne!(id1, id2);
}

#[test]
fn test_resource_id_range() {
    let manifest = ManifestConfig::default();
    let id1 = manifest.resource_id_for("/topic/test1");
    let id2 = manifest.resource_id_for("/topic/test2");

    assert!(id1 >= 0x8000);
    assert!(id2 >= 0x8000);
    assert_ne!(id1, id2);
}

#[test]
fn test_collision_detection_ok() {
    let mut manifest = ManifestConfig::default();
    manifest.rpc.provide.insert("/rpc/test1".to_string());
    manifest.rpc.consume.insert("/rpc/test2".to_string());

    assert!(manifest.validate_no_collisions().is_ok());
}

#[test]
fn test_multiple_manifests_independent() {
    let mut m1 = ManifestConfig::default();
    m1.rpc.provide.insert("/rpc/a".to_string());

    let mut m2 = ManifestConfig::default();
    m2.rpc.provide.insert("/rpc/b".to_string());

    assert!(m1.is_rpc_provided("/rpc/a"));
    assert!(!m1.is_rpc_provided("/rpc/b"));

    assert!(!m2.is_rpc_provided("/rpc/a"));
    assert!(m2.is_rpc_provided("/rpc/b"));
}
