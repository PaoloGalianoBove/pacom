# MQTT + SOME/IP bridge example

This example demonstrates all PACOM layers together:

- interactive edge app publishes on SOME/IP,
- hub app bridges SOME/IP -> MQTT,
- hub app bridges MQTT command -> SOME/IP,
- probe app listens on MQTT and prints payloads.

All apps use only the high-level `PacomRuntime` API.

## Files

- `edge/main.rs`: interactive SOME/IP producer + SOME/IP consumer.
- `hub/main.rs`: bidirectional bridge.
- `probe/main.rs`: MQTT observer for verification.
- `sender/main.rs`: interactive MQTT command publisher.
- `edge/manifest.json`: edge logical roles.
- `hub/manifest.json`: hub logical roles.
- `probe/manifest.json`: probe logical roles.
- `sender/manifest.json`: sender logical roles.

## Run (local)

1. Start a broker (Mosquitto):

```bash
sudo apt-get update && sudo apt-get install -y mosquitto mosquitto-clients
mosquitto -v
```

2. Start hub:

```bash
PACOM_MQTT_BROKER_URI=mqtt://127.0.0.1:1883 \
UP_AUTHORITY=ecu-hub UP_UE_ID=0x3301 \
PACOM_MANIFEST_PATH=examples/mqtt_bridge/hub/manifest.json \
PACOM_CLOUD_UE_ID=0x2200 \
cargo run --example mqtt_bridge_hub
```

3. Start probe:

```bash
PACOM_MQTT_BROKER_URI=mqtt://127.0.0.1:1883 \
UP_AUTHORITY=ecu-probe UP_UE_ID=0x3303 \
PACOM_MANIFEST_PATH=examples/mqtt_bridge/probe/manifest.json \
PACOM_CLOUD_UE_ID=0x2200 \
cargo run --example mqtt_bridge_probe
```

4. Start MQTT sender (for reverse flow MQTT -> SOME/IP):

```bash
PACOM_MQTT_BROKER_URI=mqtt://127.0.0.1:1883 \
UP_AUTHORITY=ecu-sender UP_UE_ID=0x3304 \
PACOM_MANIFEST_PATH=examples/mqtt_bridge/sender/manifest.json \
cargo run --example mqtt_bridge_sender
```

5. Start edge interactive app:

```bash
UP_AUTHORITY=ecu-edge UP_UE_ID=0x3302 \
PACOM_MANIFEST_PATH=examples/mqtt_bridge/edge/manifest.json \
cargo run --example mqtt_bridge_edge
```

6. In edge terminal type any text and press Enter.

Expected:

- edge sends SOME/IP `/bridge/up`,
- hub forwards to MQTT upstream URI,
- probe prints `[PROBE:UPSTREAM] ...`.

7. In sender terminal type a command and press Enter.

Expected reverse flow:

- sender publishes MQTT command,
- hub receives MQTT command and forwards to SOME/IP `/bridge/down`,
- edge prints `[EDGE] received from hub: ...`.

Optional external verification tools:

- Broker: `mosquitto`.
- CLI tools: `mosquitto-clients`.

Install on Ubuntu:

```bash
sudo apt-get update && sudo apt-get install -y mosquitto mosquitto-clients
```
