# RTT on two Linux containers (two ECU simulation)

This folder contains minimal manifests to run RTT examples in two Docker containers connected by a Docker network.

## Files

- `manifest-server.json`: logical PACOM manifest for the server app.
- `manifest-client.json`: logical PACOM manifest for the client app.

## vSomeIP Configuration

The vSomeIP configuration is **generated dynamically in memory** by pacom when the container starts. 
You do not need to provide static `vsomeip.json` files anymore.

## Required environment variables per container

- `UP_AUTHORITY`: unique authority per ECU (example: `ecu-a`, `ecu-b`).
- `UP_UE_ID`: unique app UE id in hex (example: `0x1100`, `0x1101`).
- `PACOM_MANIFEST_PATH`: path to the logical manifest file inside the container.

## Important notes for two-container vSomeIP

- For different ECUs, keep different `UP_AUTHORITY` values.
- Ensure UDP multicast for service discovery is allowed in your Docker network.
- The ECU should have exactly one routing-manager process. By default pacom auto-elects the first application as router.

## Docker build and run (Virtual Network Mode)

```bash
# Create network (if not exists)
docker network create pacom-net

# Build image from the workspace root, where the Dockerfile is located
docker build -t pacom-demo:latest -f Dockerfile .
```

### Run Server

```bash
docker run -it --rm --name rtt-server --network pacom-net \
  -e UP_AUTHORITY=rtt-server -e UP_UE_ID=0x1100 \
  -e PACOM_MANIFEST_PATH=/opt/pacom/examples/rtt/server/manifest.json \
  pacom-demo:latest /opt/pacom/bin/pacom-server
```

### Run Client

```bash
docker run -it --rm --name rtt-client --network pacom-net \
  -e UP_AUTHORITY=rtt-client -e UP_UE_ID=0x1101 \
  -e PACOM_MANIFEST_PATH=/opt/pacom/examples/rtt/client/manifest.json \
  pacom-demo:latest /opt/pacom/bin/pacom-client
```

## Quick RTT regression checklist

1. Start server container first, then client.
2. Trigger at least one RPC round-trip from client.
3. Confirm no vSomeIP registration conflicts and no timeout.
