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
const DEFAULT_LOCK_STALE_MS: u64 = 8_000;
const DEFAULT_ELECTION_WAIT_MS: u64 = 4_000;

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
    dbg_log(format!(
        "setup start: ue_id=0x{:04X} authority='{}' role_override={:?} lock_stale_ms={} election_wait_ms={}",
        ue_id,
        authority,
        std::env::var("PACOM_VSOMEIP_ROLE").ok(),
        lock_stale_timeout().as_millis(),
        election_wait_timeout().as_millis()
    ));

    // If a configuration path is explicitly provided, use it as-is.
    if let Ok(config_path) = std::env::var("PACOM_VSOMEIP_CONFIG_PATH") {
        dbg_log(format!(
            "Using PACOM_VSOMEIP_CONFIG_PATH='{}' for UE=0x{:04x}, authority='{}'",
            config_path, ue_id, authority
        ));
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
        dbg_log(format!(
            "Using existing VSOMEIP_CONFIGURATION='{}' for UE=0x{:04x}, authority='{}'",
            config_path, ue_id, authority
        ));
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
                "level": "warning",
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
                "level": "warning",
                "console": "true"
            },
            "applications": [
                {
                    "name": app_name,
                    "id": format!("0x{:04x}", ue_id)
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
        dbg_log(format!("Generated vSomeIP JSON config content:\n{}", config_content));
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
    dbg_log(format!("Generated dynamic vSomeIP config at '{}'", config_path));

    // 5. Set the environment variable for vSomeIP engine
    unsafe {
        std::env::set_var("VSOMEIP_CONFIGURATION", &config_path);
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
        &std::path::PathBuf::from(config_path),
        None,
    ).map_err(|e| {
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
    if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
        dbg_log("get_local_ip: UDP bind to 0.0.0.0:0 succeeded");
        if socket.connect("8.8.8.8:80").is_ok() {
            dbg_log("get_local_ip: UDP connect to 8.8.8.8:80 succeeded");
            if let Ok(local_addr) = socket.local_addr() {
                dbg_log(format!("get_local_ip: selected local IP {}", local_addr.ip()));
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
    match OpenOptions::new().write(true).create_new(true).open(LOCK_FILE) {
        Ok(mut file) => {
            let _ = writeln!(file, "pid={}", std::process::id());
            dbg_log(format!("Lock acquired and stamped pid={} at {}", std::process::id(), LOCK_FILE));
            true
        }
        Err(e) => {
            dbg_log(format!("Lock acquire failed at {}: {}", LOCK_FILE, e));
            false
        }
    }
}

fn lock_stale_timeout() -> Duration {
    let stale_ms = std::env::var("PACOM_VSOMEIP_LOCK_STALE_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_LOCK_STALE_MS);
    let d = Duration::from_millis(stale_ms);
    dbg_log(format!("lock_stale_timeout={}ms", d.as_millis()));
    d
}

fn election_wait_timeout() -> Duration {
    let wait_ms = std::env::var("PACOM_VSOMEIP_ELECTION_WAIT_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_ELECTION_WAIT_MS);
    let d = Duration::from_millis(wait_ms);
    dbg_log(format!("election_wait_timeout={}ms", d.as_millis()));
    d
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
        dbg_log(format!("socket_is_alive('{}')=false reason=path_missing", path));
        return false;
    }
    // Attempt a brief connection to check if it's alive
    let alive = UnixStream::connect(Path::new(path)).is_ok();
    dbg_log(format!("socket_is_alive('{}')={}", path, alive));
    alive
}
