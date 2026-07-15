#!/usr/bin/env sh
set -eu

APP_BIN="${APP_BIN:-rtt_server}"

if [ -z "${PACOM_MANIFEST_PATH:-}" ]; then
  case "$APP_BIN" in
    rtt_server)
      PACOM_MANIFEST_PATH="/opt/pacom/examples/rtt/deploy/manifest-server.json"
      ;;
    rtt_client)
      PACOM_MANIFEST_PATH="/opt/pacom/examples/rtt/deploy/manifest-client.json"
      ;;
    *)
      PACOM_MANIFEST_PATH="/opt/pacom/examples/rtt/deploy/manifest-client.json"
      ;;
  esac
  export PACOM_MANIFEST_PATH
fi

if [ -z "${PACOM_VSOMEIP_CONFIG_PATH:-}" ]; then
  if [ -z "${PACOM_VSOMEIP_TEMPLATE_PATH:-}" ]; then
    case "$APP_BIN" in
      rtt_server)
        PACOM_VSOMEIP_TEMPLATE_PATH="/opt/pacom/examples/rtt/deploy/vsomeip-router.json"
        ;;
      rtt_client)
        PACOM_VSOMEIP_TEMPLATE_PATH="/opt/pacom/examples/rtt/deploy/vsomeip-client.json"
        ;;
      *)
        PACOM_VSOMEIP_TEMPLATE_PATH="/opt/pacom/examples/rtt/deploy/vsomeip-client.json"
        ;;
    esac
    export PACOM_VSOMEIP_TEMPLATE_PATH
  fi

  ECU_IP="${ECU_IP:-$(hostname -i | awk '{print $1}') }"
  ECU_IP="$(echo "$ECU_IP" | awk '{print $1}')"
  APP_ID_HEX="${APP_ID_HEX:-${UP_UE_ID:-0x2234}}"
  APP_NAME="${APP_NAME:-app-${APP_ID_HEX}}"

  PACOM_VSOMEIP_CONFIG_PATH="/tmp/pacom-vsomeip.generated.json"
  sed \
    -e "s|\${ECU_IP}|${ECU_IP}|g" \
    -e "s|\${APP_NAME}|${APP_NAME}|g" \
    -e "s|\${APP_ID_HEX}|${APP_ID_HEX}|g" \
    "$PACOM_VSOMEIP_TEMPLATE_PATH" > "$PACOM_VSOMEIP_CONFIG_PATH"
  export PACOM_VSOMEIP_CONFIG_PATH

  echo "[ENTRYPOINT] Generated vSomeIP config: ${PACOM_VSOMEIP_CONFIG_PATH}"
  echo "[ENTRYPOINT] ECU_IP=${ECU_IP} APP_NAME=${APP_NAME} APP_ID_HEX=${APP_ID_HEX}"
fi

if [ -z "${PACOM_VSOMEIP_CONFIG_PATH:-}" ]; then
  case "$APP_BIN" in
    rtt_server)
      PACOM_VSOMEIP_CONFIG_PATH="/opt/pacom/examples/rtt/deploy/vsomeip-router.json"
      ;;
    rtt_client)
      PACOM_VSOMEIP_CONFIG_PATH="/opt/pacom/examples/rtt/deploy/vsomeip-client.json"
      ;;
    *)
      PACOM_VSOMEIP_CONFIG_PATH="/opt/pacom/examples/rtt/deploy/vsomeip-client.json"
      ;;
  esac
  export PACOM_VSOMEIP_CONFIG_PATH
fi

if [ -z "${UP_AUTHORITY:-}" ]; then
  export UP_AUTHORITY="ecu-local"
fi

if mount | grep "on /tmp" > /dev/null; then
  echo "[ENTRYPOINT] /tmp is mounted as a volume. IPC shared mode enabled."
  if ls /tmp/vsomeip* 1> /dev/null 2>&1; then
    echo "[ENTRYPOINT] Found existing vSomeIP IPC sockets in /tmp"
  fi
else
  echo "[ENTRYPOINT] WARNING: /tmp is NOT a shared volume. Single-ECU multi-container communication via vSomeIP will fail without host networking."
fi

exec "/opt/pacom/bin/${APP_BIN}"
