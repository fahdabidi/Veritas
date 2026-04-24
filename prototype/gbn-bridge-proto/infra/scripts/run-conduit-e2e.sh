#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET_DIR="${VERITAS_BRIDGE_TARGET_DIR:-${TMPDIR:-/tmp}/veritas-proto006-phase9-e2e-target}"
ARTIFACT_DIR="${VERITAS_CONDUIT_E2E_ARTIFACT_DIR:-$ROOT_DIR/artifacts/conduit-e2e}"
LOG_FILE="$ARTIFACT_DIR/run.log"

if command -v cargo >/dev/null 2>&1; then
  CARGO_BIN="cargo"
elif [[ -x "/mnt/c/Users/fahd_/.cargo/bin/cargo.exe" ]]; then
  CARGO_BIN="/mnt/c/Users/fahd_/.cargo/bin/cargo.exe"
else
  echo "cargo not found in PATH and no Windows cargo.exe fallback found" >&2
  exit 1
fi

mkdir -p "$ARTIFACT_DIR"

cd "$ROOT_DIR"

{
  echo "==> Veritas Conduit distributed e2e harness"
  echo "Root: $ROOT_DIR"
  echo "Target dir: $TARGET_DIR"
  echo "Artifact dir: $ARTIFACT_DIR"
  echo "Started at: $(date -u +"%Y-%m-%dT%H:%M:%SZ")"

  "$CARGO_BIN" test --test e2e --target-dir "$TARGET_DIR" -- --nocapture

  if command -v docker >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then
    docker compose -f docker-compose.conduit-e2e.yml config >/dev/null
    echo "Validated docker-compose.conduit-e2e.yml"
  else
    echo "docker compose not available; skipped compose validation"
  fi
} | tee "$LOG_FILE"
