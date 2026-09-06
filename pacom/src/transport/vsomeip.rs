use log::info;
use std::net::UdpSocket;
use std::sync::Arc;
use up_rust::{UCode, UStatus, UUri};
use up_transport_vsomeip::UPTransportVsomeip;

const ROUTER_NAME: &str = "routingmanagerd";
const DEFAULT_RPC_RELIABLE_PORT: u16 = 30508;
const DEFAULT_DISCOVERY_UNRELIABLE_PORT: u16 = 30510;
const DEFAULT_TOPIC_PUBLISH_RELIABLE_PORT: u16 = 30511;

use crate::utils::discovery_channel_count;

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

/// Set up the local vSomeIP transport as a client of the host routing manager.
/// Dynamically detects the local network interface IP to configure the unicast address.
pub async fn setup_vsomeip_transport(
    ue_id: u16,
    authority: &str,
) -> Result<Arc<UPTransportVsomeip>, UStatus> {
    dbg_log(format!(
        "setup start: ue_id=0x{:04X} authority='{}' routing='{}'",
        ue_id, authority, ROUTER_NAME
    ));

    let app_name = format!("app-0x{:04x}", ue_id);
    info!(
        "[vSomeIP] Initializing {} as routingmanagerd client",
        app_name
    );

    // Dynamically detect our local IP address.
    let ecu_ip = get_local_ip();
    info!("[vSomeIP] Dynamically detected ECU unicast IP: {}", ecu_ip);

    let config_path = format!("/tmp/vsomeip-{app_name}.json");
    let default_log_level =
        std::env::var("PACOM_VSOMEIP_LOG_LEVEL").unwrap_or_else(|_| "error".to_string());
    let rpc_reliable_port = normalize_service_port_from_env(
        "PACOM_VSOMEIP_RPC_RELIABLE_PORT",
        DEFAULT_RPC_RELIABLE_PORT,
    );
    let topic_publish_port = normalize_service_port_from_env(
        "PACOM_VSOMEIP_TOPIC_PUBLISH_PORT",
        DEFAULT_TOPIC_PUBLISH_RELIABLE_PORT,
    );
    let discovery_port = normalize_service_port_from_env(
        "PACOM_VSOMEIP_DISCOVERY_PORT",
        DEFAULT_DISCOVERY_UNRELIABLE_PORT,
    );
    let sd_port = normalize_service_port_from_env("PACOM_VSOMEIP_SD_PORT", 30490);
    let multicast =
        std::env::var("PACOM_VSOMEIP_MULTICAST").unwrap_or_else(|_| "224.224.224.224".to_string());

    let config_value = serde_json::json!({
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
        "routing": ROUTER_NAME,
        "services": [
            {
                "service": normalize_hex_u16(ue_id),
                "instance": "0x0001",
                "reliable": rpc_reliable_port.to_string()
            },
            {
                "service": normalize_hex_u16(topic_publish_service_id_for(ue_id)),
                "instance": "0x0001",
                "reliable": topic_publish_port.to_string()
            },
            {
                "service": normalize_hex_u16(discovery_service_id_for(ue_id)),
                "instance": "0x0001",
                "unreliable": discovery_port.to_string(),
                "events": [{
                    "event": "0x8f01",
                    "is_field": "false",
                    "is_reliable": "false"
                }],
                "eventgroups": [{
                    "eventgroup": "0x8f01",
                    "events": ["0x8f01"],
                    "is_reliable": "false"
                }]
            }
        ],
        "service-discovery": {
            "enable": "true",
            "multicast": multicast,
            "port": sd_port.to_string(),
            "protocol": "udp",
            "initial_delay_min": 10,
            "initial_delay_max": 100,
            "repetitions_base_delay": 200,
            "repetitions_max": 3,
            "ttl": "3"
        }
    });

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

    std::fs::write(&config_path, config_content).map_err(|e| {
        UStatus::fail_with_code(
            UCode::INTERNAL,
            format!("Failed to write vsomeip config: {e}"),
        )
    })?;
    dbg_log(format!(
        "Generated dynamic vSomeIP config at '{}'",
        config_path
    ));

    unsafe {
        std::env::set_var("VSOMEIP_CONFIGURATION", &config_path);
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
        &std::path::PathBuf::from(config_path),
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
            dbg_log(format!(
                "get_local_ip: using PACOM_VSOMEIP_UNICAST_IP={}",
                ip
            ));
            return ip.to_string();
        }
    }

    for probe in [
        "8.8.8.8:80",
        "172.17.0.1:80",
        "192.168.0.1:80",
        "10.0.0.1:80",
    ] {
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
