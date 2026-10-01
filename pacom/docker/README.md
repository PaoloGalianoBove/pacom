# PACOM Docker deployment

Run all commands from the `pacom-develop` directory, where the root `Dockerfile`
and `Makefile` are located.

## Podman Compose

The `podman-*` Make targets use the same scenario environment files and Compose
files as the `docker-*` targets; no separate stack needs to be maintained. Install
Podman and a Compose provider supported by `podman compose` (for example,
Docker Compose v2 or `podman-compose`). Check that `podman compose version`
works before starting. Run these commands from `pacom-develop`:

When `podman compose` selects Docker Compose v2 as its external provider, start
the rootless Podman API socket first (without `sudo`):

```bash
systemctl --user enable --now podman.socket
systemctl --user is-active podman.socket
```

If the build reports `failed to connect to the docker API` at
`/run/user/<uid>/podman/podman.sock`, check this socket before retrying; tar
writer errors in the same output can be a consequence of the lost connection.

Then run the build and start the selected topology:

```bash
make podman-build SCENARIO=mqtt-bridge TOPOLOGY=single-host
make podman-up SCENARIO=mqtt-bridge TOPOLOGY=single-host
make podman-attach SCENARIO=mqtt-bridge TOPOLOGY=single-host SERVICE=app-a
make podman-logs SCENARIO=mqtt-bridge TOPOLOGY=single-host
make podman-down SCENARIO=mqtt-bridge TOPOLOGY=single-host
```

The MQTT broker uses the included `mosquitto.conf`. If an existing stack was
started before this file was mounted, restart it with `podman-down` followed by
`podman-up` for the same scenario and topology; no image rebuild is needed.

The simulated `app-ecu` is disabled by default in the single-host and multi-host
topologies. The `databroker` stays available for an external ECU provider. To
remove a previously running `app-ecu`, run `podman-down` and then `podman-up`
with the same scenario and topology. To start the simulated ECU as well, opt in
to its profile:

```bash
make podman-up SCENARIO=mqtt-bridge TOPOLOGY=multi-host PROFILE_ARGS='--profile mqtt --profile ecu'
```

Use `SCENARIO=rtt`, `birthday`, or `mqtt-bridge` and `TOPOLOGY=single-host`,
`multi-host`, or `hybrid-host` just as with Docker. The other matching targets
are `podman-config`, `podman-up-build`, `podman-rebuild`, `podman-ps`,
`podman-log-service`, and `podman-output`; `SERVICE` selects the application
for service-specific commands. For finite applications, use `podman-output`
after they exit instead of `podman-attach`.

The multi-host and hybrid simulations rely on `network_mode: service:router-*`
and healthy-router startup ordering. Choose a Compose provider that supports
these options. Host networking and SOME/IP multicast can behave differently
with rootless Podman; validate network reachability on the target host before
using the physical multi-host deployment.

### Move to another host

The PACOM image alone contains the application binaries, but Compose also needs
the scenario files and the Mosquitto/Databroker images. On the source host, from
`pacom-develop`, export the images and the runtime configuration (no Rust source
or build context is needed):

```bash
podman save -o pacom-demo.tar pacom-demo:latest
podman save -o mosquitto.tar eclipse-mosquitto:1.6
podman save -o databroker.tar ghcr.io/eclipse-kuksa/kuksa-databroker:0.6.0
tar -cf pacom-deploy.tar Makefile pacom/docker/compose.*.yaml pacom/docker/env/*.env pacom/docker/mosquitto.conf pacom/docker/vss.json
```

Transfer these four archives to a host with the same CPU architecture, Podman,
and a Compose provider. On that host, from the directory containing the
archives:

```bash
podman load -i pacom-demo.tar
podman load -i mosquitto.tar
podman load -i databroker.tar
mkdir -p pacom-deploy
tar -xf pacom-deploy.tar -C pacom-deploy
cd pacom-deploy
systemctl --user start podman.socket
make podman-up-portable SCENARIO=mqtt-bridge TOPOLOGY=multi-host
make podman-ps SCENARIO=mqtt-bridge TOPOLOGY=multi-host
```

Use `podman-up-portable` only with preloaded images: it neither rebuilds PACOM
nor pulls missing images. The other `podman-*` targets work from `pacom-deploy`
as well, including `podman-attach` and `podman-down`. The `multi-host` topology
still simulates two hosts on one machine; moving this bundle does not by itself
create a distributed deployment across physical machines.

## Ankaios MQTT multi-host

`ankaios.multi-host.yaml` runs the complete MQTT multi-host simulation as eight
Ankaios 1.0 Podman workloads on `agent_A`: two routers, switch, dashboard,
Mosquitto, cloud app, Databroker, and the simulated ECU. This deployment includes
`app-ecu` even though it is optional in the Compose deployment. Ankaios manages
the workloads directly; do not run the Compose stack at the same time (both
publish Databroker on port 55555). `multi-host` still means two simulated network
namespaces on one physical host, not two physical hosts.

Install Ankaios 1.0 and Podman on the destination host and ensure `ank-server`
and `ank-agent` are running. The manifest assumes the default `agent_A` name.
The systemd agent uses *rootful* Podman, which cannot see images built with
rootless Podman or Docker. On the source host, build with `make podman-build
SCENARIO=mqtt-bridge TOPOLOGY=multi-host` only if the PACOM image does not
already exist. Then export everything Ankaios needs from `pacom-develop` (use
`docker save` if PACOM was built with Docker):

```bash
podman pull docker.io/library/eclipse-mosquitto:1.6
podman pull ghcr.io/eclipse-kuksa/kuksa-databroker:0.6.0
podman save -o pacom-demo.tar pacom-demo:latest
podman save -o mosquitto.tar docker.io/library/eclipse-mosquitto:1.6
podman save -o databroker.tar ghcr.io/eclipse-kuksa/kuksa-databroker:0.6.0
tar -cf pacom-ankaios-config.tar pacom/docker/ankaios.multi-host.yaml pacom/docker/mosquitto.conf pacom/docker/vss.json
scp pacom-demo.tar mosquitto.tar databroker.tar pacom-ankaios-config.tar user@target:~/
```

After a Dockerfile change, rebuild PACOM before exporting: an existing
`pacom-demo.tar` still contains the old image layers. For a smaller transfer
file, replace `podman save -o pacom-demo.tar pacom-demo:latest` above with
`podman save pacom-demo:latest | gzip -1 > pacom-demo.tar.gz`. Transfer the
compressed archive instead, and load it on the destination with
`sudo podman load -i pacom-demo.tar.gz`. Streaming compression avoids writing
an additional uncompressed archive on the source host.

Copy these four archives to a machine with the same CPU architecture and an
installed Ankaios 1.0 server/agent and Podman. From the directory containing
the archives on that machine, load the images into *rootful* Podman and start
without rebuilding, pulling, or cloning the source repository:

```bash
sudo podman load -i pacom-demo.tar
sudo podman load -i mosquitto.tar
sudo podman load -i databroker.tar
mkdir -p pacom-ankaios-config
tar -xf pacom-ankaios-config.tar -C pacom-ankaios-config
cd pacom-ankaios-config
sudo podman image exists localhost/pacom-demo:latest || sudo podman tag docker.io/library/pacom-demo:latest localhost/pacom-demo:latest
uname -m
sudo podman image inspect localhost/pacom-demo:latest --format '{{.Architecture}}'
sudo podman network exists pacom-net || sudo podman network create pacom-net
sudo podman network inspect pacom-net
sudo install -d /opt/pacom/ankaios
sudo install -m 0644 pacom/docker/vss.json /opt/pacom/ankaios/vss.json
sudo install -m 0644 pacom/docker/mosquitto.conf /opt/pacom/ankaios/mosquitto.conf
sudo systemctl start ank-server ank-agent
ank get agents
ank apply pacom/docker/ankaios.multi-host.yaml
ank get workloads
```

The loaded PACOM image must be tagged `localhost/pacom-demo:latest`, as used by
the manifest. Some image archives load as `docker.io/library/pacom-demo:latest`;
the `podman tag` above adds the expected name without duplicating the image.
Before applying the manifest, verify that the image architecture matches the
host (for example, `arm64` for `aarch64`, `amd64` for `x86_64`) and that the
rootful Podman network inspection reports `dns_enabled: true`. An x86_64 image
cannot run natively on an aarch64 host: rebuild PACOM on an ARM64 build machine
with sufficient disk/RAM and transfer the new archive. The Dockerfile resolves
the C++ include path for the build machine's architecture. Also check the
architecture of the Mosquitto and Databroker images; obtain ARM64 versions if
the transferred archives contain AMD64 images. If `dns_enabled` is false,
configure Podman's network DNS (CNI needs the `dnsname` plugin; netavark needs
`aardvark-dns`) and recreate `pacom-net` when no workloads are attached before
applying the manifest. Without DNS, the aliases `mosquitto` and `databroker`
will not resolve. The router and application pairs share `pacom-ipc-a` or
`pacom-ipc-b` as `/tmp`; Podman creates these named volumes automatically.
Mosquitto and Databroker have DNS aliases on the `pacom-net` network; the
Databroker also publishes port 55555 to the host. Ankaios dependencies order
container startup; the apps wait for their router socket and required TCP
endpoints before initializing.

After updating the manifest, reapply it with
`ank apply pacom/docker/ankaios.multi-host.yaml`. Use four separate terminals
on the Ankaios host to follow the request path and operate both menus:

```bash
ank logs --follow pacom-app-a
ank logs --follow pacom-app-ecu
sudo podman attach pacom-app-b
sudo podman attach pacom-cloud-app
```

`pacom-app-a` (light-switch) serves the dashboard RPC and sends actuation
requests to Databroker; `pacom-app-ecu` is the simulated actuation provider that
handles those requests. Both interactive menus accept options `0` through `4`.
Detach with `Ctrl-p`, `Ctrl-q` to keep the container running; `/quit` exits the
app and Ankaios restarts it because its restart policy is `ALWAYS`. Use
`ank logs --follow pacom-mosquitto` to diagnose broker connectivity.

To remove this deployment, run `ank delete workload pacom-app-a pacom-app-b
pacom-app-ecu pacom-cloud-app pacom-router-a pacom-router-b pacom-mosquitto
pacom-databroker` as one command. Podman volumes are retained so that results
and IPC are not deleted implicitly.

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