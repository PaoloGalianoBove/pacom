use up_rust::UUri;

pub fn verbose_debug_enabled() -> bool {
    std::env::var("PACOM_DEBUG_VERBOSE")
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes" || v == "on"
        })
        .unwrap_or(false)
}

pub fn dbg_log(module: &str, msg: impl AsRef<str>) {
    if verbose_debug_enabled() {
        println!("[PACOM-DBG][{}] {}", module, msg.as_ref());
    }
}

pub fn rpc_diag_enabled() -> bool {
    std::env::var("PACOM_RPC_DIAGNOSTICS")
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes" || v == "on"
        })
        .unwrap_or(false)
}

pub fn rpc_diag_log(msg: impl AsRef<str>) {
    if rpc_diag_enabled() {
        println!("[PACOM-RPC-DIAG][Router] {}", msg.as_ref());
    }
}

pub fn uri_dbg(uri: &UUri) -> String {
    format!(
        "{} [auth='{}', ue=0x{:08X}, ver={}, res=0x{:08X}]",
        uri.to_uri(false),
        uri.authority_name(),
        uri.ue_id,
        uri.ue_version_major,
        uri.resource_id
    )
}


