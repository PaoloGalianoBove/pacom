use pacom::error::PacomError;
use up_rust::{UStatus, UCode};

#[test]
fn test_error_display_manifest_violation() {
    let err = PacomError::ManifestViolation {
        operation: "rpc.provide".to_string(),
        name: "/rpc/test".to_string(),
    };
    
    assert_eq!(
        err.to_string(),
        "Manifest violation: operation 'rpc.provide' not declared for '/rpc/test'"
    );
}

#[test]
fn test_error_display_id_collision() {
    let err = PacomError::IdCollision {
        name_a: "/rpc/test1".to_string(),
        name_b: "/rpc/test2".to_string(),
        id: 0x1234,
    };
    
    assert_eq!(
        err.to_string(),
        "ID collision: '/rpc/test1' and '/rpc/test2' both resolve to ID 0x1234"
    );
}

#[test]
fn test_error_from_ustatus() {
    let status = UStatus::fail_with_code(UCode::UNAVAILABLE, "network down");
    let err: PacomError = status.into();
    
    match err {
        PacomError::Transport(s) => {
            assert_eq!(s.code, UCode::UNAVAILABLE.into());
            assert_eq!(s.message, Some("network down".to_string()));
        }
        _ => panic!("Expected Transport error"),
    }
}
