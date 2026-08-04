use up_rust::UUri;

/// Reads a boolean-like environment flag, accepting common false values.
pub fn env_flag_enabled(var: &str, default: bool) -> bool {
    std::env::var(var)
        .ok()
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            !(v == "0" || v == "false" || v == "no" || v == "off")
        })
        .unwrap_or(default)
}

/// Returns the configured number of discovery channels, clamped to `1..=64`.
pub fn discovery_channel_count() -> u16 {
    std::env::var("PACOM_DISCOVERY_CHANNELS")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .filter(|v| *v > 0 && *v <= 64)
        .unwrap_or(16)
}

/// Returns whether verbose PACOM debug logging is enabled.
pub fn verbose_debug_enabled() -> bool {
    env_flag_enabled("PACOM_DEBUG_VERBOSE", false)
}

/// Emits a debug log line when verbose PACOM logging is enabled.
pub fn dbg_log(module: &str, msg: impl AsRef<str>) {
    if verbose_debug_enabled() {
        println!("[PACOM-DBG][{}] {}", module, msg.as_ref());
    }
}

/// Returns whether RPC diagnostics are enabled.
pub fn rpc_diag_enabled() -> bool {
    env_flag_enabled("PACOM_RPC_DIAGNOSTICS", false)
}

/// Emits an RPC diagnostic log line when enabled.
pub fn rpc_diag_log(msg: impl AsRef<str>) {
    if rpc_diag_enabled() {
        println!("[PACOM-RPC-DIAG][Router] {}", msg.as_ref());
    }
}

/// Formats a `UUri` into a verbose debug-friendly string.
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

/// Returns a trimmed environment string only when it is non-empty.
pub fn env_string_non_empty(var: &str) -> Option<String> {
    std::env::var(var)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}
