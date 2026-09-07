# PACOM Docker deployment

Run all commands from the `pacom-develop` directory, where the root `Dockerfile`
and `Makefile` are located.

## Scenarios and topologies

Available scenarios:

- `rtt`
- `birthday`
- `mqtt-bridge`

Available topologies:

- `single-host`: one routing manager and two applications sharing host networking
  and the same vSomeIP IPC volume.
- `multi-host`: two simulated hosts. Each host is a network namespace containing
  one routing manager and one application; the two namespaces communicate over a
  Docker bridge network.

Examples:

```bash
make docker-up SCENARIO=rtt TOPOLOGY=single-host
make docker-up SCENARIO=rtt TOPOLOGY=multi-host

make docker-up SCENARIO=birthday TOPOLOGY=single-host
make docker-up SCENARIO=birthday TOPOLOGY=multi-host

make docker-up SCENARIO=mqtt-bridge TOPOLOGY=single-host
make docker-up SCENARIO=mqtt-bridge TOPOLOGY=multi-host
```

Inspect logs or attach to an interactive application:

```bash
make docker-logs SCENARIO=mqtt-bridge TOPOLOGY=multi-host
make docker-attach SCENARIO=mqtt-bridge TOPOLOGY=multi-host SERVICE=app-b
```

Finite applications such as the RTT client exit after completing their work.
Read their retained output instead of attaching:

```bash
make docker-output SCENARIO=rtt TOPOLOGY=single-host SERVICE=app-b
```

The RTT client writes its CSV to `results/rtt_measurements_single_host.csv` or
`results/rtt_measurements_multi_host.csv` in the `pacom-develop` directory.
These bind-mounted files survive `docker-down`.

Detach from a container without stopping it with `Ctrl-p`, `Ctrl-q`.

Stop a scenario and remove its IPC volumes:

```bash
make docker-down SCENARIO=mqtt-bridge TOPOLOGY=multi-host
```

## Build cache

`make docker-up` starts the existing image without requesting a rebuild. Build
the image explicitly before the first run:

Build explicitly without starting containers:

```bash
make docker-build SCENARIO=rtt TOPOLOGY=single-host
```

After changing PACOM source code, rebuild and start in one command:

```bash
make docker-up-build SCENARIO=rtt TOPOLOGY=single-host
```

Docker reuses cached dependency layers when possible. The dependency layer is
keyed by `Cargo.toml` and `Cargo.lock`, while application sources are copied
afterward. Do not use `--no-cache` for normal development.

Use `make docker-rebuild` only when a genuinely clean image is required.

## Router configuration

PACOM applications are always clients of the external `routingmanagerd` process.
The Docker image exposes that process as `pacom-vsomeip-router` and starts it via
`router-entrypoint.sh`.

The Compose files populate `PACOM_VSOMEIP_NODE_UE_IDS` from the selected scenario.
The router entrypoint generates the node configuration before startup. These
variables override the default ports without rebuilding:

```text
PACOM_VSOMEIP_SD_PORT=30490
PACOM_VSOMEIP_RPC_RELIABLE_PORT=30508
PACOM_VSOMEIP_DISCOVERY_PORT=30510
PACOM_VSOMEIP_TOPIC_PUBLISH_PORT=30511  # TCP topic/event endpoint
```

For `single-host`, set `PACOM_HOST_IP` when the node must communicate with an
external machine:

```bash
PACOM_HOST_IP=192.168.1.20 make docker-up SCENARIO=rtt TOPOLOGY=single-host
```

The multi-host topology is a local simulation. A real multi-host deployment uses
one routing manager and one IPC volume per physical host, plus a network that
supports routable addresses and SOME/IP-SD multicast between hosts.

## MQTT bridge on physical hosts

The three applications can be placed on different hosts:

```text
Host A: routing manager + light-switch
Host B: routing manager + light-dashboard
Host C: cloud-app (MQTT only)
```

On hosts A and B, the routing manager and PACOM applications use host networking
and share `/tmp`. This vSomeIP build fixes its Unix socket base path at compile
time, so mounting a different runtime path does not relocate IPC. Each routing manager is configured with the UE IDs
that may be scheduled onto that host. The supplied multi-host simulation gives
both routers the complete UE-ID list, so either application can move between the
two simulated hosts without changing its binary.

For a physical deployment:

- set `PACOM_VSOMEIP_UNICAST_IP` to the real address of each host;
- allow SOME/IP-SD multicast `224.224.224.224:30490/udp` between hosts;
- allow the configured SOME/IP TCP/UDP service ports;
- set `PACOM_MQTT_BROKER_URI` to a DNS name or address reachable from every host;
- never use the Compose-only hostname `mosquitto` across independent Docker hosts.

The current `compose.multi-host.yaml` must therefore be used as a local test, not
copied unchanged to two machines. On real machines deploy the router and local
applications separately, preserving one router and one IPC volume per host.