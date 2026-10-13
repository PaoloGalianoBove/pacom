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

Install Ankaios 1.0 and Podman on the destination host. The manifest assumes the
connected agent is named `agent_A`. Its systemd agent uses *rootful* Podman, so
images built by rootless Podman on the PC must be loaded again into rootful
Podman on the board. The PACOM ARM64 image already built as
`localhost/pacom-demo:arm64` can be reused; no PACOM rebuild is needed unless its
application code or Dockerfile changed.

On the source PC, from `pacom-develop`, prepare ARM64 archives and a small config
archive. First verify that the PACOM image is `arm64`:

```bash
cd ~/u-protocol/pacom-develop
podman image inspect localhost/pacom-demo:arm64 --format '{{.Architecture}}'
podman pull --platform linux/arm64 docker.io/library/eclipse-mosquitto:1.6
podman pull --platform linux/arm64 ghcr.io/eclipse-kuksa/kuksa-databroker:0.6.0
podman image inspect docker.io/library/eclipse-mosquitto:1.6 --format '{{.Architecture}}'
podman image inspect ghcr.io/eclipse-kuksa/kuksa-databroker:0.6.0 --format '{{.Architecture}}'
podman save -o pacom-demo-arm64.tar localhost/pacom-demo:arm64
podman save -o mosquitto-arm64.tar docker.io/library/eclipse-mosquitto:1.6
podman save -o databroker-arm64.tar ghcr.io/eclipse-kuksa/kuksa-databroker:0.6.0
tar -cf pacom-ankaios-config.tar pacom/docker/ankaios.multi-host.yaml pacom/docker/ankaios.rtt.multi-host.yaml pacom/docker/ankaios.rtt.ipc.yaml pacom/docker/mosquitto.conf pacom/docker/vss.json
```

All three architecture checks must print `arm64`. If `pacom-demo-arm64.tar`
already exists and the image has not changed, it can be reused. Copy the four
archives to the board with `scp`, or copy them to the USB drive:

```bash
scp pacom-demo-arm64.tar mosquitto-arm64.tar databroker-arm64.tar pacom-ankaios-config.tar root@BOARD_IP:~/
```

For USB instead, copy the same four archives to its mounted directory and run
`sync` before ejecting it.

On the ARM64 board, as root, ensure Ankaios 1.0 is installed and its CLI and
services are available. If Ankaios is not installed, install the pinned release
and then verify the commands:

```bash
curl -fsSL https://github.com/eclipse-ankaios/ankaios/releases/download/v1.0.4/install.sh -o /tmp/ankaios-install.sh
bash /tmp/ankaios-install.sh -v v1.0.4
command -v ank
systemctl start ank-server ank-agent
```

If the CLI is already installed, skip the installer and just start the services.
On an existing deployment, stop its workloads before updating; on a fresh board,
skip this delete command:

```bash
ank delete workload pacom-app-a pacom-app-b pacom-app-ecu pacom-cloud-app pacom-router-a pacom-router-b pacom-mosquitto pacom-databroker
```

Load all images into the same rootful Podman store used by `ank-agent`, unpack
the manifest/config, prepare its bind mounts, and create the dedicated network
with the static subnet used in the manifest:

```bash
podman load -i ~/pacom-demo-arm64.tar
podman load -i ~/mosquitto-arm64.tar
podman load -i ~/databroker-arm64.tar
podman tag localhost/pacom-demo:arm64 localhost/pacom-demo:latest
podman image inspect localhost/pacom-demo:latest --format '{{.Architecture}}'
podman image inspect docker.io/library/eclipse-mosquitto:1.6 --format '{{.Architecture}}'
podman image inspect ghcr.io/eclipse-kuksa/kuksa-databroker:0.6.0 --format '{{.Architecture}}'
mkdir -p ~/pacom-ankaios-config
tar -xf ~/pacom-ankaios-config.tar -C ~/pacom-ankaios-config
cd ~/pacom-ankaios-config
if ip route | grep -Fq '10.90.55.'; then echo 'Subnet conflict: choose a different free subnet and update the manifest'; exit 1; fi
if podman network exists pacom-ankaios-net; then podman network inspect pacom-ankaios-net; else podman network create --subnet 10.90.55.0/24 --gateway 10.90.55.1 pacom-ankaios-net; fi
install -d /opt/pacom/ankaios
install -m 0644 pacom/docker/vss.json /opt/pacom/ankaios/vss.json
install -m 0644 pacom/docker/mosquitto.conf /opt/pacom/ankaios/mosquitto.conf
systemctl start ank-server ank-agent
ank get agents
```

All image checks must print `arm64`; `ank get agents` must show `agent_A`. If
the route check reports an overlapping route, choose a free subnet and update
the two fixed IPs and matching `--add-host` entries in the manifest before
creating the network. This network intentionally does not rely on Podman DNS:
Mosquitto is `10.90.55.10`, Databroker is `10.90.55.11`, and the apps receive
those names through `--add-host`. Databroker is reachable on the internal
network, so its port is not published on the host; this avoids the board's
missing CNI `multiport` iptables match. The manifest runs Mosquitto with AppArmor
unconfined because the host's `mosquitto` profile blocked both its config read
and stop signals; this exception applies only to the test broker. Recreate the
Podman network if it already exists with a different subnet, after ensuring no
other workloads use it.

Apply the manifest and check that all eight workloads are running:

```bash
ank apply pacom/docker/ankaios.multi-host.yaml
ank get workloads
```

Open four terminals on the board: follow logs for switch and ECU, attach to the
two interactive menus. Detach from either menu with `Ctrl-p`, `Ctrl-q`.

```bash
ank logs --follow pacom-app-a
ank logs --follow pacom-app-ecu
podman attach pacom-app-b
podman attach pacom-cloud-app
```

Use menu options `0`-`4` in dashboard and cloud. Verify actuation in the ECU
log. To stop the complete test deployment, run `ank delete workload` followed
by the same eight workload names above. Podman images and named IPC/results
volumes are left in place.

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

## Ankaios RTT on the ARM64 board

The RTT manifests reuse the `localhost/pacom-demo:latest` image and the
`pacom-server`/`pacom-client` binaries already included in it. The
`ankaios.rtt.multi-host.yaml` manifest uses two router containers, separate
vSomeIP IPC volumes, and the existing `pacom-ankaios-net` bridge. The
`ankaios.rtt.ipc.yaml` manifest matches Compose `single-host`: one router,
host networking, and a shared vSomeIP IPC volume. Each client writes its own
CSV to `/opt/pacom/results` on the board. The client restart policy is `NEVER`
so a completed run cannot overwrite its measurements.

Create and transfer a fresh config archive from the project root (the archive
command above includes both RTT manifests). On the board, first stop the MQTT
workloads so their SOME/IP services do not interfere with the benchmark, then
prepare the persistent result directory:

```bash
ank delete workload pacom-app-a pacom-app-b pacom-app-ecu pacom-cloud-app pacom-router-a pacom-router-b pacom-mosquitto pacom-databroker
install -d -m 0777 /opt/pacom/results
```

Run the two topologies sequentially. The network and PACOM image are reused;
do not run both RTT manifests at once. After applying each manifest, inspect
the client log and verify that the CSV contains the header plus 10,000 samples
(10,001 lines total):

```bash
cd ~/pacom-ankaios-config
ank apply pacom/docker/ankaios.rtt.multi-host.yaml
ank get workloads
ank logs pacom-rtt-client
wc -l /opt/pacom/results/rtt_measurements_multi_host.csv
awk -F, 'NR > 1 && $3 != "ok" { errors++ } END { print "failed samples:", errors + 0 }' /opt/pacom/results/rtt_measurements_multi_host.csv
ank delete workload pacom-rtt-client pacom-rtt-server pacom-rtt-router-a pacom-rtt-router-b

ank apply pacom/docker/ankaios.rtt.ipc.yaml
ank get workloads
ank logs pacom-rtt-client
wc -l /opt/pacom/results/rtt_measurements_ipc.csv
awk -F, 'NR > 1 && $3 != "ok" { errors++ } END { print "failed samples:", errors + 0 }' /opt/pacom/results/rtt_measurements_ipc.csv
```

The expected CSVs are
`/opt/pacom/results/rtt_measurements_multi_host.csv` and
`/opt/pacom/results/rtt_measurements_ipc.csv`. Copy them off the board with
`scp` after both runs (from the PC, replace `BOARD_IP` with the board address):

```bash
scp root@BOARD_IP:/opt/pacom/results/rtt_measurements_multi_host.csv root@BOARD_IP:/opt/pacom/results/rtt_measurements_ipc.csv ./
```

If the client workload is not `Succeeded` or the CSV is missing/short, inspect
`ank logs pacom-rtt-server` and `ank logs pacom-rtt-client` before comparing
results.

## Native ARM64 build on the board

Build the PACOM image natively with the board's rootful Podman. This avoids
QEMU and keeps the Rust toolchain inside the build image. Cargo defaults to one
job to stay within the board's memory; use more jobs only on a roomier builder.
The first build compiles the dependencies and can take a while. Subsequent
builds reuse Podman layers; do not prune them between builds.

From the project root on the PC, create a source archive without local Rust
artifacts, benchmark results, or plots, then transfer it to the board:

```bash
cd ~/u-protocol/pacom-develop
tar -czf /tmp/pacom-arm64-src.tar \
  --exclude='.git' --exclude='pacom/target' \
  Dockerfile .dockerignore pacom
scp /tmp/pacom-arm64-src.tar root@BOARD_IP:/tmp/
```

On the ARM64 board, check available space, extract the sources, and build into
the same rootful Podman store used by `ank-agent`:

```bash
df -h /
free -h
mkdir -p /opt/pacom-build
mkdir -p /opt/pacom-build/tmp
tar -xzf /tmp/pacom-arm64-src.tar -C /opt/pacom-build
cd /opt/pacom-build
df -h /opt/pacom-build/tmp
TMPDIR=/opt/pacom-build/tmp podman build --layers --format docker --network=host \
  --build-arg CARGO_BUILD_JOBS=1 \
  -t localhost/pacom-demo:latest -f Dockerfile .
podman image inspect localhost/pacom-demo:latest --format '{{.Os}}/{{.Architecture}}'
```

The architecture check should print `linux/arm64`. The build changes only the
local PACOM image; it does not restart existing workloads. Apply the dedicated
Ankaios app manifest only after confirming the image build succeeded. The
`ankaios.mqtt-rgb.yaml` manifest includes DLT, Databroker, CAN provider,
Mosquitto, a local vSomeIP router, `light_switch`, dashboard, and cloud app. It
deliberately excludes `can-replay` and `vambienttemp` so temperature updates do
not overwrite manually selected colors.

```bash
cd /opt/pacom-build
ank apply pacom/docker/ankaios.mqtt-rgb.yaml
ank get workloads
```

Attach to the interactive dashboard and cloud menus from two board terminals:

```bash
podman attach pacom-rgb-dashboard
podman attach pacom-rgb-cloud
```

In either menu, choose `0` to send RGB off, or `2` and enter six hex digits
(for example `12A4FF`) to send a color. Detach with `Ctrl-p`, `Ctrl-q` without
stopping either app.

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