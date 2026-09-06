#!/usr/bin/env sh
set -eu

APP_BIN="${APP_BIN:-pacom-server}"

if [ -z "${PACOM_MANIFEST_PATH:-}" ]; then
  case "$APP_BIN" in
    pacom-server)
      PACOM_MANIFEST_PATH="/opt/pacom/examples/rtt/server/manifest.json"
      ;;
    pacom-client)
      PACOM_MANIFEST_PATH="/opt/pacom/examples/rtt/client/manifest.json"
      ;;
    *)
      echo "[ENTRYPOINT] PACOM_MANIFEST_PATH is required for APP_BIN=${APP_BIN}" >&2
      exit 64
      ;;
  esac
  export PACOM_MANIFEST_PATH
fi

if [ -z "${UP_AUTHORITY:-}" ]; then
  export UP_AUTHORITY="ecu-local"
fi

if mount | grep "on /tmp " > /dev/null; then
  echo "[ENTRYPOINT] Shared vSomeIP /tmp volume is mounted."
  if ls /tmp/vsomeip-* 1> /dev/null 2>&1; then
    echo "[ENTRYPOINT] Found existing vSomeIP IPC sockets in /tmp"
  fi
else
  echo "[ENTRYPOINT] /tmp is not mounted; the external routing manager may be unreachable."
fi

if [ "$#" -gt 0 ]; then
  exec "$@"
fi

exec "/opt/pacom/bin/${APP_BIN}"
