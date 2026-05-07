#!/usr/bin/env bash
# Run publisher Postgres-backed tests against the local Kubernetes Postgres StatefulSet.
set -euo pipefail

NAMESPACE="${VERITAS_K8S_NAMESPACE:-veritas}"
LOCAL_PORT="${VERITAS_K8S_POSTGRES_LOCAL_PORT:-15432}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

for dep in kubectl python3 cargo; do
  command -v "$dep" >/dev/null 2>&1 || {
    echo "ERROR: '$dep' is required." >&2
    exit 1
  }
done

password="$(
  kubectl -n "$NAMESPACE" get secret postgres-credentials \
    -o jsonpath='{.data.GBN_BRIDGE_POSTGRES_PASSWORD}' |
    python3 -c 'import base64,sys; print(base64.b64decode(sys.stdin.read().strip()).decode())'
)"

if [[ -z "$password" ]]; then
  echo "ERROR: could not read GBN_BRIDGE_POSTGRES_PASSWORD from postgres-credentials secret." >&2
  exit 1
fi

port_forward_log="$(mktemp)"
kubectl -n "$NAMESPACE" port-forward svc/postgres "${LOCAL_PORT}:5432" >"$port_forward_log" 2>&1 &
pf_pid=$!

cleanup() {
  kill "$pf_pid" >/dev/null 2>&1 || true
  rm -f "$port_forward_log"
}
trap cleanup EXIT INT TERM

python3 - "$LOCAL_PORT" <<'PY'
import socket
import sys
import time

port = int(sys.argv[1])
deadline = time.time() + 30
last_error = None
while time.time() < deadline:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=1):
            sys.exit(0)
    except OSError as exc:
        last_error = exc
        time.sleep(0.5)
print(f"port-forward did not become ready on 127.0.0.1:{port}: {last_error}", file=sys.stderr)
sys.exit(1)
PY

export GBN_BRIDGE_POSTGRES_HOST=127.0.0.1
export GBN_BRIDGE_POSTGRES_PORT="$LOCAL_PORT"
export GBN_BRIDGE_POSTGRES_DATABASE=veritas_conduit
export GBN_BRIDGE_POSTGRES_USER=veritas
export GBN_BRIDGE_POSTGRES_PASSWORD="$password"
export GBN_BRIDGE_POSTGRES_SCHEMA=conduit_publisher
export GBN_BRIDGE_POSTGRES_SSLMODE=disable
export GBN_BRIDGE_TEST_POSTGRES_URL="host=127.0.0.1 port=${LOCAL_PORT} user=veritas password=${password} dbname=veritas_conduit sslmode=disable"

cd "$ROOT_DIR"

if [[ $# -gt 0 ]]; then
  cargo test "$@"
else
  cargo test -p gbn-bridge-publisher --test persistence_flow
fi
