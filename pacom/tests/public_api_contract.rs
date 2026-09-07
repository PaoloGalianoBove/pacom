use pacom::{PacomError, PacomRuntime, RuntimeConfig};
use std::path::PathBuf;
use std::time::Duration;

fn test_manifest_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "pacom-public-api-contract-{}.json",
        std::process::id()
    ))
}

fn assert_manifest_violation(
    result: Result<(), PacomError>,
    expected_operation: &str,
    expected_name: &str,
) {
    match result {
        Err(PacomError::ManifestViolation { operation, name }) => {
            assert_eq!(operation, expected_operation);
            assert_eq!(name, expected_name);
        }
        other => panic!("expected manifest violation, got {other:?}"),
    }
}

#[tokio::test]
async fn public_api_is_predictable_without_transport_infrastructure() {
    let manifest_path = test_manifest_path();
    std::fs::write(
        &manifest_path,
        r#"{
            "topics": {
                "publish": ["/events/library/output"],
                "subscribe": ["/events/library/input"]
            }
        }"#,
    )
    .expect("write test manifest");

    unsafe {
        std::env::set_var("UP_UE_ID", "0x3456");
        std::env::set_var("PACOM_DISABLE_VSOMEIP", "true");
        std::env::set_var("PACOM_DISCOVERY_MAX_WAIT_MS", "1");
        std::env::set_var("PACOM_DISCOVERY_POLL_MS", "1");
    }

    let runtime = PacomRuntime::new(RuntimeConfig {
        manifest_path: Some(manifest_path.to_string_lossy().into_owned()),
        authority: Some("library-test-node".to_string()),
        mqtt_config: None,
    })
    .await
    .expect("public configuration should initialize without vSomeIP");

    assert_manifest_violation(
        runtime
            .register_rpc_method("/rpc/undeclared", |_| async { Vec::new() })
            .await,
        "rpc.provide",
        "/rpc/undeclared",
    );
    assert_manifest_violation(
        runtime
            .publish_event("/events/undeclared", Vec::new())
            .await,
        "topics.publish",
        "/events/undeclared",
    );
    assert_manifest_violation(
        runtime
            .subscribe_event("/events/undeclared", |_| async {})
            .await,
        "topics.subscribe",
        "/events/undeclared",
    );
    match runtime
        .invoke_rpc_method("/rpc/undeclared", Vec::new())
        .await
    {
        Err(PacomError::ManifestViolation { operation, name }) => {
            assert_eq!(operation, "rpc.consume");
            assert_eq!(name, "/rpc/undeclared");
        }
        other => panic!("expected manifest violation, got {other:?}"),
    }

    let publication = runtime
        .publish_event("/events/library/output", b"payload".to_vec())
        .await;
    assert!(
        matches!(publication, Err(PacomError::Transport(_))),
        "publish must propagate the unavailable transport status"
    );
    runtime
        .subscribe_event("/events/library/input", |_| async {})
        .await
        .expect("a declared subscription may wait for provider discovery");

    runtime.shutdown().await.expect("shutdown should complete");

    for (capability, manifest) in [
        ("provider", r#"{"rpc":{"provide":["/rpc/library/echo"]}}"#),
        ("consumer", r#"{"rpc":{"consume":["/rpc/library/remote"]}}"#),
    ] {
        std::fs::write(&manifest_path, manifest).expect("write RPC manifest");

        let initialization = tokio::time::timeout(
            Duration::from_secs(1),
            PacomRuntime::new(RuntimeConfig {
                manifest_path: Some(manifest_path.to_string_lossy().into_owned()),
                authority: Some(format!("library-test-{capability}")),
                mqtt_config: None,
            }),
        )
        .await
        .expect("RPC initialization must not hang");

        match initialization {
            Err(PacomError::Config(message)) => assert!(
                message.contains("RPC capabilities require the vSomeIP transport"),
                "unexpected RPC configuration error: {message}"
            ),
            Err(error) => {
                panic!("RPC {capability} should return a configuration error, got {error:?}")
            }
            Ok(_) => panic!("RPC {capability} should require vSomeIP"),
        }
    }

    unsafe {
        std::env::remove_var("UP_UE_ID");
        std::env::remove_var("PACOM_DISABLE_VSOMEIP");
        std::env::remove_var("PACOM_DISCOVERY_MAX_WAIT_MS");
        std::env::remove_var("PACOM_DISCOVERY_POLL_MS");
    }
    std::fs::remove_file(manifest_path).expect("remove test manifest");
}
