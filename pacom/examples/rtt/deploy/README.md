# RTT on two Linux containers (two ECU simulation)

This folder contains minimal files to run RTT examples in two Docker containers connected by a Docker network.

Important: these files are templates for deployment patterns, not rigid one-to-one naming rules.

## Files

- `vsomeip-router.json`: vSomeIP config for the routing manager process of an ECU.
- `vsomeip-client.json`: generic vSomeIP config template for one application process on an ECU.
- `manifest-server.json`: logical PACOM manifest for the server app.
- `manifest-client.json`: logical PACOM manifest for the client app.

## Why two vSomeIP files

- Scalable container model (recommended here):
- one router config per ECU (`vsomeip-router.json`),
- one app config template (`vsomeip-client.json`) rendered at container startup.

You do NOT need to manually edit one huge ECU file whenever a new app/container appears.
Each container can generate its own minimal app identity config from env (`UP_UE_ID`, `APP_NAME`) and still use the same ECU routing-manager.

If you run many apps on the same ECU, create one app config per app container by replacing:

- `${APP_NAME}` with a unique app name
- `${APP_ID_HEX}` with a unique hex id (for example `0x2234`)

This scales to many containers on one ECU without manual edits of a central app list.

## Required environment variables per container

- `UP_AUTHORITY`: unique authority per ECU (example: `ecu-a`, `ecu-b`).
- `UP_UE_ID`: unique app UE id in hex (example: `0x1234`, `0x2234`).
- `PACOM_MANIFEST_PATH`: path to the logical manifest file inside the container.
- `PACOM_VSOMEIP_CONFIG_PATH`: path to the vSomeIP config JSON inside the container.

Point router container to `vsomeip-router.json`.
Point each app container to its generated per-app config file.

## Important notes for two-container vSomeIP

- `unicast` must be reachable from the other container. Do not use `127.0.0.1`.
- For different ECUs, keep different `UP_AUTHORITY` values.
- Ensure UDP multicast for service discovery is allowed in your Docker network.
- The ECU should have exactly one routing-manager process.
- Each app process can use its own generated config derived from `vsomeip-client.json`.

## Discovery behavior in PACOM

The high-level API does not expose discovery details. Discovery stays internal in runtime:

- providers announce capabilities internally,
- consumers resolve providers from runtime cache,
- manifests remain logical only (`provide/consume/publish/subscribe`).

This means no provider UE id hardcoding is needed in consumer manifests.

## About manifest placement

The manifests under `examples/rtt/server` and `examples/rtt/client` are example-app files.
In production, each app container should mount its own manifest file and set `PACOM_MANIFEST_PATH`.

## Docker build and run (Single ECU - Recommended)

Per far comunicare container sulla stessa ECU (stesso host Docker) senza usare `--network host` o preoccuparsi del multicast snooping, usa un volume condiviso per i socket Unix IPC di vSomeIP.

Crea un volume condiviso per l'IPC:

```bash
docker volume create pacom-ipc
```

Run ECU A Server (routing manager):

```bash
docker run --rm -d \
	--name ecu-a-server \
	-v pacom-ipc:/tmp \
	-e APP_BIN=rtt_server \
	-e UP_AUTHORITY=ecu-a \
	-e UP_UE_ID=0x1234 \
	-e PACOM_MANIFEST_PATH=/config/manifest-server.json \
	-e PACOM_VSOMEIP_CONFIG_PATH=/config/vsomeip-router.json \
	-v "$PWD/examples/rtt/deploy/manifest-server.json:/config/manifest-server.json:ro" \
	-v "$PWD/examples/rtt/deploy/vsomeip-router.json:/config/vsomeip-router.json:ro" \
	pacom-rtt:latest
```

Run ECU A Client (stessa ECU):

```bash
docker run --rm -it \
	--name ecu-a-client \
	-v pacom-ipc:/tmp \
	-e APP_BIN=rtt_client \
	-e UP_AUTHORITY=ecu-a \
	-e UP_UE_ID=0x2234 \
	-e PACOM_MANIFEST_PATH=/config/manifest-client.json \
	-e PACOM_VSOMEIP_CONFIG_PATH=/config/vsomeip-client.json \
	-v "$PWD/examples/rtt/deploy/manifest-client.json:/config/manifest-client.json:ro" \
	-v "$PWD/examples/rtt/deploy/vsomeip-client.json:/config/vsomeip-client.json:ro" \
	pacom-rtt:latest
```

> **Note:** con `-v pacom-ipc:/tmp` i due container condividono il file `/tmp/vsomeip-0` (creato dal router). vSomeIP riconosce che sono sulla stessa macchina e usa comunicazione IPC ultra-veloce invece del networking UDP multicast. Non serve creare una rete Docker dedicata.


Run ECU B detached (if you prefer collecting logs with docker logs):

```bash
docker run --rm -d \
	--name ecu-b-client \
	--network pacom-net \
	-e APP_BIN=rtt_client \
	-e UP_AUTHORITY=ecu-b \
	-e UP_UE_ID=0x2234 \
	-e PACOM_MANIFEST_PATH=/config/manifest-client.json \
	-e PACOM_VSOMEIP_CONFIG_PATH=/config/vsomeip-client.json \
	-v "$PWD/examples/rtt/deploy/manifest-client.json:/config/manifest-client.json:ro" \
	-v "$PWD/examples/rtt/deploy/vsomeip-client.json:/config/vsomeip-client.json:ro" \
	pacom-rtt:latest
```

Inspect logs:

```bash
docker logs -f ecu-a-server
```

Inspect client logs (when client is detached):

```bash
docker logs -f ecu-b-client
```

Stop server:

```bash
docker stop ecu-a-server
```
