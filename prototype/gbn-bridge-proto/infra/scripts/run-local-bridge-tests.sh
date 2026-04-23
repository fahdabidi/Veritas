#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET_DIR="${VERITAS_BRIDGE_TARGET_DIR:-${TMPDIR:-/tmp}/veritas-bridge-target-phase9}"

if command -v cargo >/dev/null 2>&1; then
  CARGO_BIN="cargo"
elif [[ -x "/mnt/c/Users/fahd_/.cargo/bin/cargo.exe" ]]; then
  CARGO_BIN="/mnt/c/Users/fahd_/.cargo/bin/cargo.exe"
else
  echo "cargo not found in PATH and no Windows cargo.exe fallback found" >&2
  exit 1
fi

cd "$ROOT_DIR"

echo "==> Veritas Conduit local bridge harness"
echo "Root: $ROOT_DIR"
echo "Target dir: $TARGET_DIR"

"$CARGO_BIN" fmt --all --check
"$CARGO_BIN" check --workspace
CARGO_INCREMENTAL=0 "$CARGO_BIN" test --workspace --target-dir "$TARGET_DIR"

if command -v docker >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then
  docker compose -f docker-compose.bridge-smoke.yml config >/dev/null
  echo "Validated docker-compose.bridge-smoke.yml"
else
  echo "docker compose not available; skipped compose validation"
fi
