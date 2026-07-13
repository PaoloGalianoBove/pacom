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

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                      Application Code                           │
│   call_rpc("light-switch", payload)                             │
│   register_rpc_method("light-switch", |bytes| async { ... })    │
│   publish("light-status", payload)                              │
│   subscribe("light-switch/light-status", |bytes| { ... })       │
└───────────────────────┬─────────────────────────────────────────┘
                        │ Vec<u8> + logical service names
                        ▼
┌─────────────────────────────────────────────────────────────────┐
│                   Layer 2 — src/l2/                             │
│                                                                 │
│  ┌─────────────────────┐    ┌──────────────────────────────┐   │
│  │   PlatformClient    │    │   Service Catalog            │   │
│  │   (client.rs)       │◄───│   (catalog.rs)               │   │
│  │                     │    │   "light-switch" →           │   │
│  │  InMemoryRpcClient  │    │   (ue_id=0x1234, method=1)  │   │
│  │  InMemoryRpcServer  │    │   Loaded from built-ins +   │   │
│  └────────┬────────────┘    │   /etc/pacom/services.json  │   │
│           │                 └──────────────────────────────┘   │
└───────────┼─────────────────────────────────────────────────────┘
            │ UUri, UMessage (uProtocol native)
            ▼
┌─────────────────────────────────────────────────────────────────┐
│                   Layer 1 — src/l1/                             │
│                                                                 │
│  ┌──────────────────┐    ┌────────────────────────────────┐    │
│  │  UStreamerRouter │    │  vsomeip.rs                    │    │
│  │  (router.rs)     │───►│  - Leader election             │    │
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
| **L1** | `src/l1/` | Raw uProtocol transports: `UPTransportVsomeip`, `Mqtt5Transport`, `UStreamerRouter` |
| **L2** | `src/l2/` | Developer-facing API: `PlatformClient`, `catalog`, closure wrappers for RPC and pub/sub |
| **App** | `examples/` | Only imports `pacom::{PlatformClient, SdkConfig}` — zero uProtocol knowledge required |

---

## Quick Start

### Server (RPC endpoint)

```rust
use pacom::{PlatformClient, SdkConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = PlatformClient::new(SdkConfig { mqtt_config: None }).await?;

    client.register_rpc_method("light-switch", |request_bytes| async move {
        let command = String::from_utf8_lossy(&request_bytes).into_owned();
        println!("Received command: {}", command);
        format!("Ack: {}", command).into_bytes()
    }).await?;

    println!("Service 'light-switch' is listening...");
    std::thread::park();
    Ok(())
}
```

### Client (RPC caller)

```rust
use pacom::{PlatformClient, SdkConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = PlatformClient::new(SdkConfig { mqtt_config: None }).await?;

    let response = client.call_rpc("light-switch", b"turn-on".to_vec()).await?;
    println!("Response: {}", String::from_utf8_lossy(&response));

    Ok(())
}
```

### Publish / Subscribe

```rust
// Publisher
client.publish("light-status", b"ON".to_vec()).await?;

// Subscriber
client.subscribe("light-switch/light-status", |bytes| {
    println!("Status update: {}", String::from_utf8_lossy(&bytes));
}).await?;
```

### Cloud connectivity (MQTT 5)

```rust
use pacom::{PlatformClient, SdkConfig, MqttConfig};

let client = PlatformClient::new(SdkConfig {
    mqtt_config: Some(MqttConfig {
        broker_uri: "mqtt://broker.example.com:1883".to_string(),
        client_id: "gw-zonale-1".to_string(),
    }),
}).await?;
```

---

## Configuration

### Runtime Identity (environment variables)

`pacom` resolves the ECU identity and application ID at startup from the container environment — **never** hardcoded in application code.

| Variable | Description | Default |
|----------|-------------|---------|
| `UP_AUTHORITY` | Logical name of the ECU / zone (e.g. `gw-zonale-1`) | `local_ecu` |
| `UP_UE_ID` | Application UE ID in hex or decimal (e.g. `0x1234`) | Auto-generated (FNV-1a hash of executable name, range `≥ 0x1000`) |

> In an SDV deployment, these variables are injected by the container orchestrator (e.g. Kubernetes, AUTOSAR Adaptive, or a custom operator). The application source code never changes between deployments.

### Service Catalog

Service names are resolved to their numerical uProtocol identifiers via a **two-layer catalog**:

1. **Built-in defaults** (compiled into the binary):
   - `"light-switch"` → `(ue_id=0x1234, method_id=1)`
   - `"light-status"` → `(resource_id=0x8001)`

2. **Runtime overrides** (loaded at startup, optional):
   - `/etc/pacom/services.json` — maps service names to `(ue_id, method_id)` tuples
   - `/etc/pacom/topics.json` — maps topic names to `resource_id` values

**Example `/etc/pacom/services.json`:**
```json
{
  "climate-control": [7890, 2],
  "door-lock":       [5678, 1]
}
```

---

## Transport Behavior

### vSomeIP (intra-vehicle)

- **Auto role negotiation**: at startup, the SDK detects whether a vSomeIP routing manager is already active on the host (via Unix socket `/tmp/vsomeip-0`). If not, it atomically elects itself as the router using a lock file.
- **Auto IP detection**: uses a zero-packet UDP routing lookup to discover the correct network interface IP without any configuration.
- **Dynamic JSON config**: generates the vSomeIP configuration file at runtime (no hardcoded JSON templates).

### MQTT 5 (off-vehicle / cloud)

- Instantiated **only** if `SdkConfig.mqtt_config` is `Some(...)`.
- Used automatically for messages addressed to authorities outside the local ECU.
- The `UStreamerRouter` routes traffic to MQTT when the destination authority differs from the local one.

---

## Running locally (from this devcontainer)

I binari sono già compilati. Dopo una prima `cargo build --examples`, registra la libreria nel sistema (operazione **una tantum** per sessione):

```bash
LIB=$(find /workspaces/docker-uprotocol/pacom/target -name libvsomeip3.so.3 2>/dev/null | head -1 | xargs dirname)
echo "$LIB" | sudo tee /etc/ld.so.conf.d/vsomeip3.conf && sudo ldconfig
```

Da quel momento in poi, server e client si avviano senza nessuna variabile aggiuntiva:

```bash
# Terminale 1 — Server
UP_AUTHORITY=linux UP_UE_ID=0x1234 ./target/debug/examples/server

# Terminale 2 — Client (benchmark 10K richieste)
UP_AUTHORITY=linux UP_UE_ID=0x5678 ./target/debug/examples/client
```

> Il `postCreateCommand` nel `devcontainer.json` esegue il `ldconfig` automaticamente
> alla riapertura del devcontainer (se i binari sono già presenti).

---

## Docker

### Il Dockerfile

Il [`Dockerfile`](../Dockerfile) ora espone due target distinti:

- `dev`: immagine per devcontainer e sviluppo interattivo, con toolchain Rust e sorgenti.
- `runtime`: immagine pulita di esecuzione, con solo binari e librerie native installate in `/usr/local/lib`.

`up-transport-vsomeip-rust` **non** viene copiato nel repository dell'applicazione finale:
la dipendenza viene scaricata e compilata durante la `cargo build` come specificato in `Cargo.toml`,
ma il suo `build.rs` resta confinato nel grafo delle dipendenze.

```
Context di build = solo pacom/     ← nessun light-switch, nessun up-transport-vsomeip-rust locale
Dipendenze Rust   = da crates.io + GitHub (automatico durante cargo build)
```

### Build dell'immagine

Per il devcontainer o per lavorare dentro `pacom`:

```bash
# Dalla root del workspace (dove si trova il Dockerfile)
cd /workspaces/docker-uprotocol
docker build --target dev -t pacom-dev:latest .
```

Per un'immagine di sola esecuzione, senza `LD_LIBRARY_PATH` e senza sorgenti:

```bash
cd /workspaces/docker-uprotocol
docker build --target runtime -t pacom-runtime:latest .
```

> La prima build scarica e compila `vsomeip-sys` (~5 min). Le build successive usano
> la cache Docker e sono veloci.

### Avviare i due container (intra-host)

I container sulla stessa bridge `docker0` si vedono già via IP.  
Il SOME/IP Service Discovery usa multicast `224.224.224.224:30490/udp`.

```bash
# Terminale 1 — Server
docker run --rm -it \
  --name pacom-server \
  -e UP_AUTHORITY=gw-zonale-1 \
  -e UP_UE_ID=0x1234 \
    pacom-runtime:latest \
    server

# Terminale 2 — Client (benchmark 10K richieste)
docker run --rm -it \
  --name pacom-client \
  -e UP_AUTHORITY=gw-zonale-1 \
  -e UP_UE_ID=0x5678 \
    pacom-runtime:latest \
    client
```

### Avviare i due container (inter-host — due macchine fisiche)

Con `--network host` il container usa direttamente la NIC della macchina host;
il multicast SOME/IP SD funziona sulla LAN fisica senza configurazione aggiuntiva.

```bash
# Macchina 1 — Server
docker run --rm -it --network host \
  -e UP_AUTHORITY=gw-zonale-1 \
  -e UP_UE_ID=0x1234 \
    pacom-runtime:latest \
    server

# Macchina 2 — Client
docker run --rm -it --network host \
  -e UP_AUTHORITY=gw-zonale-2 \
  -e UP_UE_ID=0x5678 \
    pacom-runtime:latest \
    client
```

### Recuperare il CSV di benchmark

```bash
docker cp pacom-client:/usr/local/bin/rtt_measurements.csv ./rtt_results.csv
```

### Applicazioni che dipendono da `pacom`

Se sviluppi una tua app Rust sopra `pacom`, **non** devi aggiungere un `build.rs` al progetto applicativo.

La tua crate resta normale:

```toml
[dependencies]
pacom = { path = "../pacom" }
tokio = { version = "1", features = ["full"] }
```

La compilazione nativa di vSomeIP avviene dentro la dipendenza transitiva `vsomeip-sys`.
Quello che conta per il rilascio non e' il `build.rs`, ma avere un'immagine runtime che:

- contenga i `.so` di vSomeIP in un path standard come `/usr/local/lib`
- esegua `ldconfig` in fase di build
- copi solo il binario finale dell'app e le librerie native necessarie

In altre parole: `build.rs` resta nel layer di build, non entra nel contratto della tua app.

---

## Benchmark Output

The client example produces a `rtt_measurements.csv` file with the same schema as the `light-switch` reference benchmark, enabling direct performance comparison:

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
| `sys_cpu_pct` | System CPU % (reserved, currently `0.0`) |

---

## Project Structure

```
pacom/
├── Cargo.toml              # Dependencies (tokio, up-rust, up-transport-vsomeip, sysinfo…)
├── examples/
│   ├── server.rs           # Minimal RPC server example (~20 lines)
│   └── client.rs           # 10K-iteration RTT benchmark
└── src/
    ├── lib.rs              # Crate root: re-exports PlatformClient, SdkConfig, MqttConfig
    ├── l1/                 # Layer 1 — uProtocol transport primitives
    │   ├── mod.rs
    │   ├── vsomeip.rs      # vSomeIP: leader election, IP detection, config generation
    │   ├── mqtt.rs         # MQTT 5: optional cloud transport
    │   └── router.rs       # UStreamerRouter: local/cloud traffic steering
    └── l2/                 # Layer 2 — developer-facing SDK
        ├── mod.rs
        ├── catalog.rs      # Service name → (ue_id, method_id) resolution
        └── client.rs       # PlatformClient + closure wrappers for RPC and pub/sub
```

---

## Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `up-rust` | `0.9.0` | uProtocol core types and communication abstractions |
| `up-transport-vsomeip` | GitHub | SOME/IP transport for in-vehicle communication |
| `up-transport-mqtt5` | `0.4.0` | MQTT 5 transport for cloud/off-vehicle communication |
| `tokio` | `1` | Async runtime |
| `sysinfo` | `0.39.6` | Process and system resource monitoring for benchmarks |
| `serde_json` | `1.0` | Typesafe vSomeIP JSON configuration generation |
| `async-trait` | `0.1` | Async trait support |

---

## Design Principles

### 1. SDV-first identity model
Applications are **location-agnostic**. The same binary can run on *Gateway Zonale 1* today and *Gateway Zonale 2* tomorrow with no code changes — only the injected environment variables change.

### 2. Strict layer separation
The three-layer model (L1 transport → L2 SDK → application) enforces a clean boundary:
- **L1** is the only layer that knows about uProtocol wire formats.
- **L2** is the only layer that maps human-readable service names to wire addresses.
- **Applications** never touch either layer directly.

### 3. Zero-copy hot loop
The benchmark follows the same discipline as the `light-switch` reference implementation:
- Resource metrics are sampled in a **background thread** using lock-free atomics.
- All measurements are accumulated in-memory.
- The CSV is flushed **once** at the end — no I/O overhead in the measurement loop.

### 4. Graceful degradation
- No MQTT configuration? The cloud transport is simply not instantiated.
- No `UP_UE_ID`? A deterministic UE ID is derived from the executable name.
- No `UP_AUTHORITY`? Defaults to `local_ecu` for single-node development.
