use log::info;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::net::UdpSocket;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use up_rust::{UCode, UStatus, UUri};
use up_transport_vsomeip::UPTransportVsomeip;

const IPC_DIR: &str = "/tmp/vsomeip-ipc";
const ROUTER_SOCKET: &str = "/tmp/vsomeip-ipc/vsomeip-0";
const LOCK_FILE: &str = "/tmp/vsomeip-ipc/router.lock";
const ROUTER_NAME: &str = "vsomeip-router";
const DEFAULT_LOCK_STALE_MS: u64 = 8_000;
const DEFAULT_ELECTION_WAIT_MS: u64 = 4_000;
const DEFAULT_RPC_UNRELIABLE_PORT: u16 = 30509;
const DEFAULT_RPC_RELIABLE_PORT: u16 = 30508;
const DEFAULT_DISCOVERY_UNRELIABLE_PORT: u16 = 30510;
const DEFAULT_TOPIC_PUBLISH_UNRELIABLE_PORT: u16 = 30511;

use crate::utils::{discovery_channel_count, env_flag_enabled, env_string_non_empty};

fn discovery_service_id_for(ue_id: u16) -> u16 {
    0x0F00u16 + (ue_id % discovery_channel_count())
}

fn topic_publish_service_id_for(ue_id: u16) -> u16 {
    let mut topic_ue = ue_id ^ 0x4000;
    if topic_ue == 0 || topic_ue == 0xFFFF {
        topic_ue ^= 0x2000;
    }
    topic_ue
}

fn normalize_hex_u16(value: u16) -> String {
    format!("0x{:04x}", value)
}

fn normalize_service_port_from_env(var: &str, default: u16) -> u16 {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(default)
}

fn cleanup_temp_vsomeip_configs() {
    if !env_flag_enabled("PACOM_VSOMEIP_CLEAN_TEMP_CONFIGS", true) {
        return;
    }

    let Ok(entries) = fs::read_dir("/tmp") else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        let is_generated = (name.starts_with("vsomeip-") && name.ends_with(".json"))
            || (name.starts_with("pacom-vsomeip.effective.") && name.ends_with(".json"));

        if is_generated {
            let _ = fs::remove_file(path);
        }
    }
}

fn maybe_autofix_config_services(config_path: &str, ue_id: u16) -> Result<String, UStatus> {
    let auto_fix_enabled = env_flag_enabled("PACOM_VSOMEIP_AUTO_EXPOSE_SERVICES", true);
    let rpc_tcp_only = env_flag_enabled("PACOM_VSOMEIP_RPC_TCP_ONLY", true);
    let expose_topic_publish_service =
        env_flag_enabled("PACOM_VSOMEIP_EXPOSE_TOPIC_PUBLISH_SERVICE", true);
    // Self-routing is always enabled: every app registers itself in the vSomeIP
    // applications array so the routing manager can identify it correctly.
    let self_routing_enabled = true;
    let logging_level_override = env_string_non_empty("PACOM_VSOMEIP_LOG_LEVEL");

    if !auto_fix_enabled {
        dbg_log("Auto service exposure disabled via PACOM_VSOMEIP_AUTO_EXPOSE_SERVICES");
        return Ok(config_path.to_string());
    }

    let raw = std::fs::read_to_string(config_path).map_err(|e| {
        UStatus::fail_with_code(
            UCode::INTERNAL,
            format!("Failed to read vSomeIP config '{}': {e}", config_path),
        )
    })?;

    let mut json: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
        UStatus::fail_with_code(
            UCode::INVALID_ARGUMENT,
            format!("Invalid JSON in vSomeIP config '{}': {e}", config_path),
        )
    })?;

    let rpc_service_hex = normalize_hex_u16(ue_id);
    let topic_publish_service_hex = normalize_hex_u16(topic_publish_service_id_for(ue_id));
    let discovery_service_hex = normalize_hex_u16(discovery_service_id_for(ue_id));
    let rpc_port =
        normalize_service_port_from_env("PACOM_VSOMEIP_RPC_PORT", DEFAULT_RPC_UNRELIABLE_PORT);
    let rpc_reliable_port = normalize_service_port_from_env(
        "PACOM_VSOMEIP_RPC_RELIABLE_PORT",
        DEFAULT_RPC_RELIABLE_PORT,
    );
    let topic_publish_port = normalize_service_port_from_env(
        "PACOM_VSOMEIP_TOPIC_PUBLISH_PORT",
        DEFAULT_TOPIC_PUBLISH_UNRELIABLE_PORT,
    );
    let discovery_port = normalize_service_port_from_env(
        "PACOM_VSOMEIP_DISCOVERY_PORT",
        DEFAULT_DISCOVERY_UNRELIABLE_PORT,
    );

    let mut modified = false;

    let root = json.as_object_mut().ok_or_else(|| {
        UStatus::fail_with_code(
            UCode::INVALID_ARGUMENT,
            format!("vSomeIP config '{}' must be a JSON object", config_path),
        )
    })?;

    if self_routing_enabled {
        let app_name = format!("app-0x{:04x}", ue_id);
        let app_id_hex = normalize_hex_u16(ue_id);

        root.insert(
            "applications".to_string(),
            serde_json::json!([
                {
                    "name": app_name,
                    "id": app_id_hex,
                }
            ]),
        );
        root.insert("routing".to_string(), serde_json::Value::String(app_name));
        modified = true;
    }

    if let Some(level) = logging_level_override {
        let logging = root
            .entry("logging")
            .or_insert_with(|| serde_json::json!({}));
        if let Some(logging_obj) = logging.as_object_mut() {
            let current = logging_obj
                .get("level")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if current != level {
                logging_obj.insert("level".to_string(), serde_json::Value::String(level));
                modified = true;
            }
        }
    }

    let services = root
        .entry("services")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| {
            UStatus::fail_with_code(
                UCode::INVALID_ARGUMENT,
                format!(
                    "vSomeIP config '{}' has non-array 'services' field",
                    config_path
                ),
            )
        })?;

    let mut has_rpc = false;
    let mut has_topic_publish = false;
    let mut has_discovery = false;

    for service in services.iter_mut() {
        if let Some(sid) = service.get("service").and_then(|v| v.as_str()) {
            let sid = sid.to_ascii_lowercase();
            if sid == rpc_service_hex {
                has_rpc = true;
                if let Some(obj) = service.as_object_mut() {
                    if !obj.contains_key("reliable") {
                        obj.insert(
                            "reliable".to_string(),
                            serde_json::Value::String(rpc_reliable_port.to_string()),
                        );
                        modified = true;
                    }
                    if rpc_tcp_only {
                        if obj.remove("unreliable").is_some() {
                            modified = true;
                        }
                    } else if !obj.contains_key("unreliable") {
                        obj.insert(
                            "unreliable".to_string(),
                            serde_json::Value::String(rpc_port.to_string()),
                        );
                        modified = true;
                    }
                }
            }
            if expose_topic_publish_service && sid == topic_publish_service_hex {
                has_topic_publish = true;
                if let Some(obj) = service.as_object_mut() {
                    // Topic publish is event-oriented; keep it UDP-only to avoid
                    // mixing an unnecessary reliable path with RPC response traffic.
                    if obj.remove("reliable").is_some() {
                        modified = true;
                    }
                    let expected = topic_publish_port.to_string();
                    let current = obj
                        .get("unreliable")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if current != expected {
                        obj.insert(
                            "unreliable".to_string(),
                            serde_json::Value::String(expected),
                        );
                        modified = true;
                    }
                }
            }
            if sid == discovery_service_hex {
                has_discovery = true;
            }
        }
    }

    if !has_rpc {
        let rpc_service = if rpc_tcp_only {
            serde_json::json!({
                "service": rpc_service_hex,
                "instance": "0x0001",
                "reliable": rpc_reliable_port.to_string()
            })
        } else {
            serde_json::json!({
                "service": rpc_service_hex,
                "instance": "0x0001",
                "unreliable": rpc_port.to_string(),
                "reliable": rpc_reliable_port.to_string()
            })
        };
        services.push(rpc_service);
        modified = true;
    }

    if expose_topic_publish_service && !has_topic_publish {
        services.push(serde_json::json!({
            "service": topic_publish_service_hex,
            "instance": "0x0001",
            "unreliable": topic_publish_port.to_string()
        }));
        modified = true;
    }

    if !has_discovery {
        services.push(serde_json::json!({
            "service": discovery_service_hex,
            "instance": "0x0001",
            "unreliable": discovery_port.to_string(),
            "events": [
                {
                    "event": "0x8f01",
                    "is_field": "false",
                    "is_reliable": "false"
                }
            ],
            "eventgroups": [
                {
                    "eventgroup": "0x8f01",
                    "events": ["0x8f01"],
                    "is_reliable": "false"
                }
            ]
        }));
        modified = true;
    }

    if !modified {
        dbg_log(format!(
            "vSomeIP config '{}' reused unchanged (rpc={}, rpc_tcp_only={}, topic_publish={}, topic_publish_enabled={}, discovery={}, self_routing={})",
            config_path,
            normalize_hex_u16(ue_id),
            rpc_tcp_only,
            normalize_hex_u16(topic_publish_service_id_for(ue_id)),
            expose_topic_publish_service,
            normalize_hex_u16(discovery_service_id_for(ue_id)),
            self_routing_enabled
        ));
        return Ok(config_path.to_string());
    }

    let effective_path = format!("/tmp/pacom-vsomeip.effective.{}.json", ue_id);
    let serialized = serde_json::to_string_pretty(&json).map_err(|e| {
        UStatus::fail_with_code(
            UCode::INTERNAL,
            format!("Failed to serialize effective vSomeIP config: {e}"),
        )
    })?;

    std::fs::write(&effective_path, serialized).map_err(|e| {
        UStatus::fail_with_code(
            UCode::INTERNAL,
            format!(
                "Failed to write effective vSomeIP config '{}': {e}",
                effective_path
            ),
        )
    })?;

    dbg_log(format!(
        "vSomeIP effective config written to '{}' (rpc={}, rpc_tcp_only={}, topic_publish={}, topic_publish_enabled={}, discovery={}, rpc_port={}, rpc_reliable_port={}, topic_publish_port={}, discovery_port={}, self_routing={})",
        effective_path,
        normalize_hex_u16(ue_id),
        rpc_tcp_only,
        normalize_hex_u16(topic_publish_service_id_for(ue_id)),
        expose_topic_publish_service,
        normalize_hex_u16(discovery_service_id_for(ue_id)),
        rpc_port,
        rpc_reliable_port,
        topic_publish_port,
        discovery_port,
        self_routing_enabled
    ));

    Ok(effective_path)
}

fn verbose_debug_enabled() -> bool {
    std::env::var("PACOM_DEBUG_VERBOSE")
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes" || v == "on"
        })
        .unwrap_or(false)
}

fn dbg_log(msg: impl AsRef<str>) {
    if verbose_debug_enabled() {
        println!("[PACOM-DBG][vSomeIP] {}", msg.as_ref());
    }
}

/// Set up the local vSomeIP transport (routing manager or client role).
/// Negotiates the role dynamically by checking for an active Router Unix socket.
/// Dynamically detects the local network interface IP to configure the unicast address.
pub async fn setup_vsomeip_transport(
    ue_id: u16,
    authority: &str,
) -> Result<Arc<UPTransportVsomeip>, UStatus> {
    unsafe {
        std::env::set_var("VSOMEIP_BASE_PATH", IPC_DIR);
    }
    
    dbg_log(format!(
        "setup start: ue_id=0x{:04X} authority='{}' role_override={:?} lock_stale_ms={} election_wait_ms={}",
        ue_id,
        authority,
        std::env::var("PACOM_VSOMEIP_ROLE").ok(),
        lock_stale_timeout().as_millis(),
        election_wait_timeout().as_millis()
    ));

    cleanup_temp_vsomeip_configs();

    // If a configuration path is explicitly provided, use it as-is.
    if let Ok(config_path) = std::env::var("PACOM_VSOMEIP_CONFIG_PATH") {
        let effective_config_path = maybe_autofix_config_services(&config_path, ue_id)?;
        dbg_log(format!(
            "Using PACOM_VSOMEIP_CONFIG_PATH='{}' (effective='{}') for UE=0x{:04x}, authority='{}'",
            config_path, effective_config_path, ue_id, authority
        ));
        unsafe {
            std::env::set_var("VSOMEIP_CONFIGURATION", &effective_config_path);
        }

        let local_uri = UUri::try_from_parts(authority, ue_id as u32, 1, 0).map_err(|e| {
            UStatus::fail_with_code(
                UCode::INVALID_ARGUMENT,
                format!("Failed to build local UUri: {e:?}"),
            )
        })?;

        let transport = UPTransportVsomeip::new_with_config(
            local_uri,
            &authority.to_string(),
            &std::path::PathBuf::from(effective_config_path),
            None,
        )
        .map_err(|e| {
            UStatus::fail_with_code(
                UCode::INTERNAL,
                format!("Failed to build UPTransportVsomeip from PACOM_VSOMEIP_CONFIG_PATH: {e:?}"),
            )
        })?;

        return Ok(Arc::new(transport));
    }

    // Honor VSOMEIP_CONFIGURATION if already set by the container runtime.
    if let Ok(config_path) = std::env::var("VSOMEIP_CONFIGURATION") {
        let effective_config_path = maybe_autofix_config_services(&config_path, ue_id)?;
        dbg_log(format!(
            "Using existing VSOMEIP_CONFIGURATION='{}' (effective='{}') for UE=0x{:04x}, authority='{}'",
            config_path, effective_config_path, ue_id, authority
        ));
        let local_uri = UUri::try_from_parts(authority, ue_id as u32, 1, 0).map_err(|e| {
            UStatus::fail_with_code(
                UCode::INVALID_ARGUMENT,
                format!("Failed to build local UUri: {e:?}"),
            )
        })?;

        let transport = UPTransportVsomeip::new_with_config(
            local_uri,
            &authority.to_string(),
            &std::path::PathBuf::from(effective_config_path),
            None,
        )
        .map_err(|e| {
            UStatus::fail_with_code(
                UCode::INTERNAL,
                format!("Failed to build UPTransportVsomeip from VSOMEIP_CONFIGURATION: {e:?}"),
            )
        })?;

        return Ok(Arc::new(transport));
    }

    // 1. Ensure the IPC directory exists
    if let Err(e) = fs::create_dir_all(Path::new(IPC_DIR)) {
        return Err(UStatus::fail_with_code(
            UCode::INTERNAL,
            format!("Failed to create IPC directory: {e}"),
        ));
    }

    // 2. Decide role (router/client). Explicit env override wins, otherwise auto-election.
    let is_router = match std::env::var("PACOM_VSOMEIP_ROLE") {
        Ok(role) if role.eq_ignore_ascii_case("router") => true,
        Ok(role) if role.eq_ignore_ascii_case("client") => false,
        _ => negotiate_router_role().await,
    };
    dbg_log(format!(
        "Role decision: is_router={}, PACOM_VSOMEIP_ROLE={:?}",
        is_router,
        std::env::var("PACOM_VSOMEIP_ROLE").ok()
    ));

    let app_name = if is_router {
        ROUTER_NAME.to_string()
    } else {
        format!("app-0x{:04x}", ue_id)
    };

    info!(
        "[vSomeIP] Initializing as {} (is_router={})",
        app_name, is_router
    );

    // 3. Dynamically detect our local IP address
    let ecu_ip = get_local_ip();
    info!("[vSomeIP] Dynamically detected ECU unicast IP: {}", ecu_ip);

    // 4. Construct the configuration dynamically using serde_json to avoid hardcoded string templates
    let config_path = format!("/tmp/vsomeip-{}.json", app_name);
    let default_log_level = std::env::var("PACOM_VSOMEIP_LOG_LEVEL").unwrap_or_else(|_| "error".to_string());

    let config_value = if is_router {
        serde_json::json!({
            "unicast": ecu_ip,
            "logging": {
                "level": default_log_level,
                "console": "true"
            },
            "applications": [
                {
                    "name": ROUTER_NAME,
                    "id": "0x0100"
                }
            ],
            "routing": ROUTER_NAME,
            "service-discovery": {
                "enable": "true",
                "multicast": "224.224.224.224",
                "port": "30490",
                "protocol": "udp",
                "initial_delay_min": 10,
                "initial_delay_max": 100,
                "repetitions_base_delay": 200,
                "repetitions_max": 3
            }
        })
    } else {
        serde_json::json!({
            "unicast": ecu_ip,
            "logging": {
                "level": default_log_level,
                "console": "true"
            },
            "applications": [
                {
                    "name": app_name,
                    "id": format!("0x{:04x}", ue_id)
                }
            ],
            // Client instances must target the shared router app.
            "routing": ROUTER_NAME,
            "service-discovery": {
                "enable": "true",
                "multicast": "224.224.224.224",
                "port": "30490",
                "protocol": "udp",
                "initial_delay_min": 10,
                "initial_delay_max": 100,
                "repetitions_base_delay": 200,
                "repetitions_max": 3,
                "ttl": "3"
            }
        })
    };

    let config_content = serde_json::to_string_pretty(&config_value).map_err(|e| {
        UStatus::fail_with_code(
            UCode::INTERNAL,
            format!("Failed to serialize vsomeip JSON config: {e}"),
        )
    })?;
    if verbose_debug_enabled() {
        dbg_log(format!(
            "Generated vSomeIP JSON config content:\n{}",
            config_content
        ));
    }

    let mut file = File::create(&config_path).map_err(|e| {
        UStatus::fail_with_code(
            UCode::INTERNAL,
            format!("Failed to create vsomeip config file: {e}"),
        )
    })?;
    file.write_all(config_content.as_bytes()).map_err(|e| {
        UStatus::fail_with_code(
            UCode::INTERNAL,
            format!("Failed to write vsomeip config: {e}"),
        )
    })?;
    dbg_log(format!(
        "Generated dynamic vSomeIP config at '{}'",
        config_path
    ));

    let effective_config_path = maybe_autofix_config_services(&config_path, ue_id)?;
    dbg_log(format!(
        "Using generated config '{}' (effective='{}')",
        config_path, effective_config_path
    ));

    // 5. Set the environment variable for vSomeIP engine
    unsafe {
        std::env::set_var("VSOMEIP_CONFIGURATION", &effective_config_path);
    }

    // 6. Instantiate UPTransportVsomeip
    let local_uri = UUri::try_from_parts(authority, ue_id as u32, 1, 0).map_err(|e| {
        UStatus::fail_with_code(
            UCode::INVALID_ARGUMENT,
            format!("Failed to build local UUri: {e:?}"),
        )
    })?;

    let transport = UPTransportVsomeip::new_with_config(
        local_uri,
        &authority.to_string(),
        &std::path::PathBuf::from(effective_config_path),
        None,
    )
    .map_err(|e| {
        UStatus::fail_with_code(
            UCode::INTERNAL,
            format!("Failed to build UPTransportVsomeip: {e:?}"),
        )
    })?;

    dbg_log("UPTransportVsomeip initialized successfully");
    Ok(Arc::new(transport))
}

/// Dynamically determine the local network interface IP address to use for unicast.
/// It uses a UDP routing trick that doesn't send any physical packets on the network.
fn get_local_ip() -> String {
    if let Ok(ip) = std::env::var("PACOM_VSOMEIP_UNICAST_IP") {
        let ip = ip.trim();
        if !ip.is_empty() {
            dbg_log(format!("get_local_ip: using PACOM_VSOMEIP_UNICAST_IP={}", ip));
            return ip.to_string();
        }
    }

    for probe in ["8.8.8.8:80", "172.17.0.1:80", "192.168.0.1:80", "10.0.0.1:80"] {
        if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
            if socket.connect(probe).is_ok() {
                if let Ok(local_addr) = socket.local_addr() {
                    dbg_log(format!(
                        "get_local_ip: selected local IP {} using probe {}",
                        local_addr.ip(),
                        probe
                    ));
                    return local_addr.ip().to_string();
                }
            }
        }
    }

    if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
        dbg_log("get_local_ip: UDP bind to 0.0.0.0:0 succeeded");
        if socket.connect("8.8.8.8:80").is_ok() {
            dbg_log("get_local_ip: UDP connect to 8.8.8.8:80 succeeded");
            if let Ok(local_addr) = socket.local_addr() {
                dbg_log(format!(
                    "get_local_ip: selected local IP {}",
                    local_addr.ip()
                ));
                return local_addr.ip().to_string();
            }
            dbg_log("get_local_ip: local_addr() failed after connect");
        } else {
            dbg_log("get_local_ip: UDP connect to 8.8.8.8:80 failed");
        }
    } else {
        dbg_log("get_local_ip: UDP bind to 0.0.0.0:0 failed");
    }
    // Fallback to localhost if no route is found (e.g. offline environment)
    dbg_log("get_local_ip: falling back to 127.0.0.1");
    "127.0.0.1".to_string()
}

/// Dynamic Router role negotiation.
/// Checks if the router Unix socket is alive.
/// If not alive, attempts to atomically create the lock file and become the router.
async fn negotiate_router_role() -> bool {
    let wait_budget = election_wait_timeout();
    let stale_budget = lock_stale_timeout();
    dbg_log(format!(
        "Election start: socket_exists={}, lock_exists={}, wait_budget_ms={}, stale_budget_ms={}",
        Path::new(ROUTER_SOCKET).exists(),
        Path::new(LOCK_FILE).exists(),
        wait_budget.as_millis(),
        stale_budget.as_millis()
    ));

    // Check if the socket already exists and is accepting connections
    if socket_is_alive(ROUTER_SOCKET) {
        info!("[vSomeIP] Found active router socket. Joining as client.");
        dbg_log("Election result=client (active socket is alive)");
        return false;
    }

    info!("[vSomeIP] No active router socket found. Attempting to become router.");

    // Atomically try to create the lock file.
    if try_acquire_router_lock() {
        dbg_log("Election: acquired lock on first attempt");
        return become_router_after_lock();
    }

    // Another process may be starting the router. Wait and retry before giving up.
    let poll_step = Duration::from_millis(120);
    let start = std::time::Instant::now();
    let mut poll_round: u64 = 0;

    while start.elapsed() < wait_budget {
        poll_round += 1;
        if poll_round % 10 == 0 {
            dbg_log(format!(
                "Election wait poll_round={} elapsed_ms={} socket_exists={} lock_exists={}",
                poll_round,
                start.elapsed().as_millis(),
                Path::new(ROUTER_SOCKET).exists(),
                Path::new(LOCK_FILE).exists()
            ));
        }

        if socket_is_alive(ROUTER_SOCKET) {
            info!("[vSomeIP] Router became active during wait. Joining as client.");
            dbg_log("Election result=client (socket alive after wait)");
            return false;
        }

        // If the lock owner is gone, immediately attempt takeover.
        if lock_owner_is_dead(LOCK_FILE) {
            info!("[vSomeIP] Router lock owner process is gone. Attempting lock takeover.");
            let _ = fs::remove_file(LOCK_FILE);
            tokio::time::sleep(Duration::from_millis(30)).await;
            if try_acquire_router_lock() {
                dbg_log("Election: acquired lock after dead-owner recovery");
                return become_router_after_lock();
            }
        }

        tokio::time::sleep(poll_step).await;
    }

    // Recovery path: stale lock from a crashed/stopped router.
    if lock_is_stale(LOCK_FILE, lock_stale_timeout()) {
        info!("[vSomeIP] Detected stale router lock. Attempting lock takeover.");
        if let Err(e) = fs::remove_file(LOCK_FILE) {
            dbg_log(format!("Election stale-lock remove failed: {}", e));
        } else {
            dbg_log("Election stale-lock remove succeeded");
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
        if try_acquire_router_lock() {
            dbg_log("Election: acquired lock after stale-lock recovery");
            return become_router_after_lock();
        }
    }

    info!("[vSomeIP] Lock held by another process. Joining as client.");
    false
}

fn become_router_after_lock() -> bool {
    // If we won the election and a stale socket file exists, remove it now.
    // This avoids clients deleting an active router socket during startup races.
    if Path::new(ROUTER_SOCKET).exists() && !socket_is_alive(ROUTER_SOCKET) {
        if let Err(e) = fs::remove_file(ROUTER_SOCKET) {
            dbg_log(format!("Stale router socket removal failed: {}", e));
        } else {
            dbg_log("Stale router socket removal succeeded");
        }
    }
    info!("[vSomeIP] Lock acquired. We are the Router.");
    dbg_log("Election result=router");
    true
}

fn try_acquire_router_lock() -> bool {
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(LOCK_FILE)
    {
        Ok(mut file) => {
            let _ = writeln!(file, "pid={}", std::process::id());
            dbg_log(format!(
                "Lock acquired and stamped pid={} at {}",
                std::process::id(),
                LOCK_FILE
            ));
            true
        }
        Err(e) => {
            dbg_log(format!("Lock acquire failed at {}: {}", LOCK_FILE, e));
            false
        }
    }
}

fn lock_stale_timeout() -> Duration {
    Duration::from_millis(DEFAULT_LOCK_STALE_MS)
}

fn election_wait_timeout() -> Duration {
    Duration::from_millis(DEFAULT_ELECTION_WAIT_MS)
}

fn lock_is_stale(path: &str, stale_after: Duration) -> bool {
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let modified = match metadata.modified() {
        Ok(t) => t,
        Err(_) => return false,
    };
    match modified.elapsed() {
        Ok(age) => {
            let stale = age >= stale_after;
            if stale {
                dbg_log(format!(
                    "Lock '{}' considered stale: age_ms={} >= stale_ms={}",
                    path,
                    age.as_millis(),
                    stale_after.as_millis()
                ));
            }
            stale
        }
        Err(_) => false,
    }
}

fn lock_owner_is_dead(path: &str) -> bool {
    // Linux-specific check: relies on /proc to verify owner process liveness.
    let contents = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };

    let pid = contents
        .lines()
        .find_map(|line| line.strip_prefix("pid="))
        .and_then(|v| v.trim().parse::<u32>().ok());

    let Some(pid) = pid else {
        return false;
    };

    let proc_path = format!("/proc/{pid}");
    let dead = !Path::new(&proc_path).exists();
    if dead {
        dbg_log(format!("Lock owner pid={} no longer exists", pid));
    }
    dead
}

fn socket_is_alive(path: &str) -> bool {
    if !Path::new(path).exists() {
        dbg_log(format!(
            "socket_is_alive('{}')=false reason=path_missing",
            path
        ));
        return false;
    }
    // Attempt a brief connection to check if it's alive
    let alive = UnixStream::connect(Path::new(path)).is_ok();
    dbg_log(format!("socket_is_alive('{}')={}", path, alive));
    alive
}
