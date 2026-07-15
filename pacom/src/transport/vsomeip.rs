use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use std::net::UdpSocket;
use log::info;
use up_rust::{UUri, UStatus, UCode};
use up_transport_vsomeip::UPTransportVsomeip;

const IPC_DIR: &str = "/tmp/vsomeip-ipc";
const ROUTER_SOCKET: &str = "/tmp/vsomeip-0";
const LOCK_FILE: &str = "/tmp/vsomeip-ipc/router.lock";
const ROUTER_NAME: &str = "vsomeip-router";

/// Set up the local vSomeIP transport (routing manager or client role).
/// Negotiates the role dynamically by checking for an active Router Unix socket.
/// Dynamically detects the local network interface IP to configure the unicast address.
pub async fn setup_vsomeip_transport(
    ue_id: u16,
    authority: &str,
) -> Result<Arc<UPTransportVsomeip>, UStatus> {
    // If a configuration path is explicitly provided, use it as-is.
    if let Ok(config_path) = std::env::var("PACOM_VSOMEIP_CONFIG_PATH") {
        unsafe {
            std::env::set_var("VSOMEIP_CONFIGURATION", &config_path);
        }

        let local_uri = UUri::try_from_parts(authority, ue_id as u32, 0, 0).map_err(|e| {
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
        ).map_err(|e| {
            UStatus::fail_with_code(
                UCode::INTERNAL,
                format!("Failed to build UPTransportVsomeip from PACOM_VSOMEIP_CONFIG_PATH: {e:?}"),
            )
        })?;

        return Ok(Arc::new(transport));
    }

    // Honor VSOMEIP_CONFIGURATION if already set by the container runtime.
    if let Ok(config_path) = std::env::var("VSOMEIP_CONFIGURATION") {
        let local_uri = UUri::try_from_parts(authority, ue_id as u32, 0, 0).map_err(|e| {
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
        ).map_err(|e| {
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

    // 2. Perform Leader Election to decide if we are Router or Client
    let is_router = negotiate_router_role().await;

    let app_name = if is_router {
        ROUTER_NAME.to_string()
    } else {
        format!("app-0x{:04x}", ue_id)
    };

    info!("[vSomeIP] Initializing as {} (is_router={})", app_name, is_router);

    // 3. Dynamically detect our local IP address
    let ecu_ip = get_local_ip();
    info!("[vSomeIP] Dynamically detected ECU unicast IP: {}", ecu_ip);

    // 4. Construct the configuration dynamically using serde_json to avoid hardcoded string templates
    let config_path = format!("/tmp/vsomeip-{}.json", app_name);
    let config_value = if is_router {
        serde_json::json!({
            "unicast": ecu_ip,
            "logging": {
                "level": "info",
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
            "unicast": "127.0.0.1",
            "logging": {
                "level": "info",
                "console": "true"
            },
            "applications": [
                {
                    "name": app_name,
                    "id": format!("0x{:04x}", ue_id)
                }
            ],
            "routing": ROUTER_NAME
        })
    };

    let config_content = serde_json::to_string_pretty(&config_value).map_err(|e| {
        UStatus::fail_with_code(
            UCode::INTERNAL,
            format!("Failed to serialize vsomeip JSON config: {e}"),
        )
    })?;

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

    // 5. Set the environment variable for vSomeIP engine
    unsafe {
        std::env::set_var("VSOMEIP_CONFIGURATION", &config_path);
    }

    // 6. Instantiate UPTransportVsomeip
    let local_uri = UUri::try_from_parts(authority, ue_id as u32, 0, 0).map_err(|e| {
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
    ).map_err(|e| {
        UStatus::fail_with_code(
            UCode::INTERNAL,
            format!("Failed to build UPTransportVsomeip: {e:?}"),
        )
    })?;

    Ok(Arc::new(transport))
}

/// Dynamically determine the local network interface IP address to use for unicast.
/// It uses a UDP routing trick that doesn't send any physical packets on the network.
fn get_local_ip() -> String {
    if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
        if socket.connect("8.8.8.8:80").is_ok() {
            if let Ok(local_addr) = socket.local_addr() {
                return local_addr.ip().to_string();
            }
        }
    }
    // Fallback to localhost if no route is found (e.g. offline environment)
    "127.0.0.1".to_string()
}

/// Dynamic Router role negotiation.
/// Checks if the router Unix socket is alive.
/// If not alive, attempts to atomically create the lock file and become the router.
async fn negotiate_router_role() -> bool {
    // Check if the socket already exists and is accepting connections
    if socket_is_alive(ROUTER_SOCKET) {
        info!("[vSomeIP] Found active router socket. Joining as client.");
        return false;
    }

    info!("[vSomeIP] No active router socket found. Attempting to become router.");

    // Clean up dead socket or lock file if they exist
    let _ = fs::remove_file(ROUTER_SOCKET);
    let _ = fs::remove_file(LOCK_FILE);

    // Atomically try to create the lock file
    match OpenOptions::new().write(true).create_new(true).open(LOCK_FILE) {
        Ok(_) => {
            info!("[vSomeIP] Lock acquired. We are the Router.");
            true
        }
        Err(_) => {
            // Another container/process was faster and is spawning the router
            info!("[vSomeIP] Lock file creation failed. Joining as client.");
            // Brief sleep to allow the router to initialize its socket
            tokio::time::sleep(Duration::from_millis(500)).await;
            false
        }
    }
}

fn socket_is_alive(path: &str) -> bool {
    if !Path::new(path).exists() {
        return false;
    }
    // Attempt a brief connection to check if it's alive
    UnixStream::connect(Path::new(path)).is_ok()
}
