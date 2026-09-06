#!/usr/bin/env sh
set -eu

ROUTER_CONFIG="${PACOM_VSOMEIP_CONFIG_PATH:-/tmp/pacom-routingmanagerd.json}"
UNICAST_IP="${PACOM_VSOMEIP_UNICAST_IP:-$(hostname -i | awk '{print $1}')}"
UE_IDS="${PACOM_VSOMEIP_NODE_UE_IDS:-}"
RPC_RELIABLE_PORT="${PACOM_VSOMEIP_RPC_RELIABLE_PORT:-30508}"
TOPIC_PORT="${PACOM_VSOMEIP_TOPIC_PUBLISH_PORT:-30511}"
DISCOVERY_PORT="${PACOM_VSOMEIP_DISCOVERY_PORT:-30510}"
DISCOVERY_CHANNELS="${PACOM_DISCOVERY_CHANNELS:-16}"

mkdir -p "$(dirname "$ROUTER_CONFIG")"
chmod 1777 /tmp

services=""
separator=""
old_ifs="$IFS"
IFS=','
for raw_ue_id in $UE_IDS; do
  ue_id=$((raw_ue_id))
  topic_id=$((ue_id ^ 0x4000))
  if [ "$topic_id" -eq 0 ] || [ "$topic_id" -eq 65535 ]; then
    topic_id=$((topic_id ^ 0x2000))
  fi
  discovery_id=$((0x0f00 + (ue_id % DISCOVERY_CHANNELS)))

  rpc_hex="$(printf '0x%04x' "$ue_id")"
  topic_hex="$(printf '0x%04x' "$topic_id")"
  discovery_hex="$(printf '0x%04x' "$discovery_id")"

  services="${services}${separator}
    {\"service\":\"${rpc_hex}\",\"instance\":\"0x0001\",\"reliable\":\"${RPC_RELIABLE_PORT}\"},
    {\"service\":\"${topic_hex}\",\"instance\":\"0x0001\",\"reliable\":\"${TOPIC_PORT}\"},
    {\"service\":\"${discovery_hex}\",\"instance\":\"0x0001\",\"unreliable\":\"${DISCOVERY_PORT}\",\"events\":[{\"event\":\"0x8f01\",\"is_field\":\"false\",\"is_reliable\":\"false\"}],\"eventgroups\":[{\"eventgroup\":\"0x8f01\",\"events\":[\"0x8f01\"],\"is_reliable\":\"false\"}]}"
  separator=","
done
IFS="$old_ifs"

cat > "$ROUTER_CONFIG" <<EOF
{
  "unicast": "${UNICAST_IP}",
  "logging": { "level": "${PACOM_VSOMEIP_LOG_LEVEL:-error}", "console": "true" },
  "applications": [{ "name": "routingmanagerd", "id": "0x0100" }],
  "routing": "routingmanagerd",
  "service-discovery": {
    "enable": "true",
    "multicast": "${PACOM_VSOMEIP_MULTICAST:-224.224.224.224}",
    "port": "${PACOM_VSOMEIP_SD_PORT:-30490}",
    "protocol": "udp",
    "initial_delay_min": 10,
    "initial_delay_max": 100,
    "repetitions_base_delay": 200,
    "repetitions_max": 3,
    "ttl": "3"
  },
  "services": [${services}
  ]
}
EOF

export VSOMEIP_APPLICATION_NAME="routingmanagerd"
export VSOMEIP_CONFIGURATION="$ROUTER_CONFIG"

echo "[ROUTER] Starting on ${UNICAST_IP}; node UE IDs: ${UE_IDS:-none}"
exec /opt/pacom/bin/pacom-vsomeip-router -q