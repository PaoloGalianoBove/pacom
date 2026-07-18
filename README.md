# `pacom` — Platform Abstraction for Connected and Automotive Middleware

> **A zero-coupling uProtocol SDK for Software-Defined Vehicle applications.**  
> Write your app once. Deploy it anywhere — any ECU, any zone, any topology.

---

## Overview

`pacom` is a Rust SDK that wraps the [Eclipse uProtocol](https://github.com/eclipse-uprotocol) transport stack for use in **Software-Defined Vehicle (SDV)** applications. It exposes a clean, ergonomic API that completely hides the complexity of underlying protocols (SOME/IP, MQTT 5), network topology, and service addresses.

An application developer using `pacom`:

- Does **not** need to know the IP address of any ECU
- Does **not** need to know any numerical UE ID, Service ID or Method ID
- Does **not** need to know whether it is a vSomeIP Router or a Client
- Does **not** need to import any `up-rust` types whatsoever

The library resolves all of this transparently at runtime.

--## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                      Application Code                           │
│   invoke_method("/rpc/echo", payload)                           │
│   register_rpc_method("/rpc/echo", |bytes| async { ... })      │
│   publish_event("/sensors/speed", payload)                      │
│   subscribe_event("/sensors/speed", |bytes| { ... })            │
└───────────────────────┬─────────────────────────────────────────┘
                        │ Vec<u8> + logical service/topic names
                        ▼
┌─────────────────────────────────────────────────────────────────┐
│              Public API Layer — src/public_api/                 │
│                                                                 │
│   ┌─────────────────────────────────────────────────────────┐   │
│   │   PacomRuntime (facade)                                 │   │
│   └───────────────────────────┬─────────────────────────────┘   │
└───────────────────────────────┼─────────────────────────────────┘
                                │ Delegations
                                ▼
┌─────────────────────────────────────────────────────────────────┐
│               Runtime Layer — src/runtime/                      │
│                                                                 │
│  ┌─────────────────────┐    ┌──────────────────────────────┐   │
│  │   RuntimeEngine     │    │   Logical Registry           │   │
│  │   (engine.rs)       │◄───│   (logical_registry.rs)      │   │
│  │                     │    │   FNV-1a Dynamic Hashing     │   │
│  │  InMemoryRpcClient  │    │   Manifest validation        │   │
│  │  InMemoryRpcServer  │    │   Collision detection        │   │
│  └────────┬────────────┘    └──────────────────────────────┘   │
└───────────┼─────────────────────────────────────────────────────┘
            │ UUri, UMessage (uProtocol native)
            ▼
┌─────────────────────────────────────────────────────────────────┐
│              Transport Layer — src/transport/                   │
│                                                                 │
│  ┌──────────────────┐    ┌────────────────────────────────┐    │
│  │  PacomRouter      │    │  vsomeip.rs                    │    │
│  │  (router.rs)     │───►│  - Threaded execution          │    │
│  │                  │    │  - Auto IP detection           │    │
│  │  local → SOME/IP │    │  - JSON config generation      │    │
│  │  cloud → MQTT 5  │    │  - UPTransportVsomeip          │    │
│  └──────────────────┘    └────────────────────────────────┘    │
│                          ┌────────────────────────────────┐    │
│                          │  mqtt.rs                       │    │
│                          │  - Optional cloud transport    │    │
│                          │  - Mqtt5Transport              │    │
│                          └────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
            │
            ▼
   vSomeIP (SOME/IP SD)    MQTT 5 broker (optional)
   intra-vehicle network   cloud / off-vehicle telemetry
```

### Layer separation

| Layer | Package | Responsibility |
|-------|---------|----------------|
| **Public API** | `src/public_api/` | Facade: `PacomRuntime` providing clean, logical methods for application developers. |
| **Runtime** | `src/runtime/` | Engine orchestrating RPC and pub/sub flows, and the service catalog/logical mapping. |
| **Transport** | `src/transport/` | Raw uProtocol transports: `vsomeip.rs`, `mqtt.rs`, and the `PacomRouter` proxy. |

---

## Quick Start

### Server (RPC endpoint)

```rust
use pacom::{PacomRuntime, RuntimeConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = PacomRuntime::new(RuntimeConfig { 
        mqtt_config: None,
        manifest_path: None,
    }).await?;

    client.register_rpc_method("/rpc/echo", |request_bytes| async move {
        let command = String::from_utf8_lossy(&request_bytes).into_owned();
        println!("Received command: {}", command);
        format!("Ack: {}", command).into_bytes()
    }).await?;

    println!("Service '/rpc/echo' is listening...");
    std::thread::park();
    Ok(())
}
```

### Client (RPC caller)

```rust
use pacom::{PacomRuntime, RuntimeConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = PacomRuntime::new(RuntimeConfig { 
        mqtt_config: None,
        manifest_path: None,
    }).await?;

    let response = client.invoke_method("/rpc/echo", b"turn-on".to_vec()).await?;
    println!("Response: {}", String::from_utf8_lossy(&response));

    Ok(())
}
```

### Publish / Subscribe

```rust
// Publisher
client.publish_event("/sensors/speed", b"100".to_vec()).await?;

// Subscriber
client.subscribe_event("/sensors/speed", |bytes| {
    println!("Speed update: {}", String::from_utf8_lossy(&bytes));
}).await?;
```
## Transport Behavior

### vSomeIP (intra-vehicle)

- **Auto role negotiation**: at startup, the SDK detects whether a vSomeIP routing manager is already active on the host (via Unix socket `/tmp/vsomeip-0`). If not, it atomically elects itself as the router using a lock file.
- **Auto IP detection**: uses a zero-packet UDP routing lookup to discover the correct network interface IP without any configuration.
- **Dynamic JSON config**: generates the vSomeIP configuration file at runtime (no hardcoded JSON templates).

### MQTT 5 (off-vehicle / cloud)

- Instantiated **only** if `RuntimeConfig.mqtt_config` is `Some(...)`.
- Used automatically for messages addressed to authorities outside the local ECU.
- The `PacomRouter` routes traffic to MQTT when the destination authority differs from the local one.

---

## Running locally (from this devcontainer)

The binaries are already compiled (or can be compiled with cargo). First, register the library in the dynamic loader system (one-time operation per session):

```bash
LIB=$(find /workspaces/pacom-develop/pacom/target -name libvsomeip3.so.3 2>/dev/null | head -1 | xargs dirname)
echo "$LIB" | sudo tee /etc/ld.so.conf.d/vsomeip3.conf && sudo ldconfig
```

From that point, server and client can be started using `cargo run`:

```bash
# Terminal 1 — Server
UP_AUTHORITY=linux UP_UE_ID=0x1234 cargo run --example rtt_server

# Terminal 2 — Client (benchmark 10K requests)
UP_AUTHORITY=linux UP_UE_ID=0x5678 cargo run --example rtt_client
```

> The `postCreateCommand` in the `devcontainer.json` runs `ldconfig` automatically upon reopening the devcontainer (if the binaries are already present).

---

## Docker

### The Dockerfile

The [`Dockerfile`](file:///workspaces/pacom-develop/pacom/Dockerfile) inside `pacom/` exposes a clean multi-stage build:
- It compiles all example binaries: `rtt_server`, `rtt_client`, `mqtt_bridge_edge`, `mqtt_bridge_hub`, `mqtt_bridge_probe`, `mqtt_bridge_sender`.
- It installs the native `vsomeip` libraries to `/usr/local/lib`.

### Building the Image

To build the demo image:

```bash
# From the pacom subdirectory (where the Dockerfile is located)
cd /workspaces/pacom-develop/pacom
docker build -t pacom-demo:latest .
```

### Running the RTT Containers locally

To run the RTT client and server in separate containers sharing a SOME/IP IPC connection (using a shared volume for the Unix socket to bypass network configuration):

```bash
# Create a shared volume for vSomeIP IPC sockets
docker volume create pacom-ipc

# Run the Server (routing manager role)
docker run --rm -it \
  --name pacom-server \
  -v pacom-ipc:/tmp \
  -e APP_BIN=rtt_server \
  -e UP_AUTHORITY=ecu-a \
  -e UP_UE_ID=0x1234 \
  -e PACOM_MANIFEST_PATH=/opt/pacom/examples/rtt/deploy/manifest-server.json \
  -e PACOM_VSOMEIP_CONFIG_PATH=/opt/pacom/examples/rtt/deploy/vsomeip-router.json \
  pacom-demo:latest

# Run the Client
docker run --rm -it \
  --name pacom-client \
  -v pacom-ipc:/tmp \
  -e APP_BIN=rtt_client \
  -e UP_AUTHORITY=ecu-a \
  -e UP_UE_ID=0x2234 \
  -e PACOM_MANIFEST_PATH=/opt/pacom/examples/rtt/deploy/manifest-client.json \
  -e PACOM_VSOMEIP_CONFIG_PATH=/opt/pacom/examples/rtt/deploy/vsomeip-client.json \
  pacom-demo:latest
```

### Retrieving the Benchmark CSV

```bash
docker cp pacom-client:/opt/pacom/rtt_measurements.csv ./rtt_results.csv
```

### Applications depending on `pacom`

When developing a Rust application using `pacom`, you do **not** need a custom `build.rs`. Simply add it to your dependencies:

```toml
[dependencies]
pacom = { path = "../pacom" }
tokio = { version = "1", features = ["full"] }
```

The native compilation of `vSomeIP` is handled transitively by the dependency.

---

## Benchmark Output

The `rtt_client` example produces a `rtt_measurements.csv` file with the following columns:

```
iteration,rtt_ms,status,proc_ram_mb,proc_vsz_mb,proc_cpu_pct,sys_ram_pct,sys_cpu_pct
0,0.152,ok,12.431,1201.840,0.0,43.1,0.0
1,0.141,ok,12.431,1201.840,0.0,43.1,0.0
...
```

| Field | Description |
|-------|-------------|
| `iteration` | Request index (0–9999) |
| `rtt_ms` | Round-trip time in **milliseconds** (3 decimal places) |
| `status` | `ok` or error description |
| `proc_ram_mb` | Process RSS memory in MB |
| `proc_vsz_mb` | Process virtual memory in MB |
| `proc_cpu_pct` | Process CPU usage % (sampled every 200ms) |
| `sys_ram_pct` | System RAM usage % at startup |
| `sys_cpu_pct` | System CPU % (currently reserved/unused) |

---

## Project Structure

```
pacom/
├── Cargo.toml              # Dependencies (tokio, up-rust, up-transport-vsomeip, sysinfo…)
├── Dockerfile              # Containerization definition
├── examples/
│   ├── mqtt_bridge/        # Example showing SOME/IP to MQTT bridging
│   └── rtt/                # Example showing RTT measurements
└── src/
    ├── lib.rs              # Crate root: re-exports PacomRuntime, RuntimeConfig, MqttConfig, etc.
    ├── error.rs            # Custom PacomError type
    ├── public_api/         # Public API: PacomRuntime facade
    ├── runtime/            # Runtime: Engine orchestrating discovery, mapping and routing
    └── transport/          # Transport: vsomeip, mqtt, and dynamic router (uStreamer)
```

---

## Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `up-rust` | `0.9.0` | uProtocol core types and communication abstractions |
| `up-transport-vsomeip` | GitHub | SOME/IP transport for in-vehicle communication |
| `up-transport-mqtt5` | `0.4.0` | MQTT 5 transport for cloud/off-vehicle communication |
| `tokio` | `1` | Async runtime |
| `sysinfo` | `0.37.2` | Process and system resource monitoring for benchmarks |
| `serde_json` | `1.0` | Typesafe JSON parsing and config generation |
| `async-trait` | `0.1` | Async trait support |

---

## Design Principles

### 1. SDV-first identity model
Applications are location-agnostic. The same binary runs anywhere with no code changes — identity is resolved dynamically from environment variables injected by the environment.

### 2. Strict layer separation
Enforces a clean boundary between the clean logical facade (`public_api`), the middleware logic and registry (`runtime`), and the low-level transport mechanisms (`transport`).

### 3. Zero-copy hot loop
To ensure accurate performance benchmarks, resource metrics are collected asynchronously in a background thread and written to CSV once after the loop finishes to prevent disk I/O interference.

### 4. Graceful degradation
Cloud transports and configurations are bypassed gracefully if not requested or missing, ensuring the application remains functional in offline/local-only scenarios.

