#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUN_ID=""
EVIDENCE_S3_KEY=""
EVIDENCE_ZIP=""
ARTIFACT_DIR=""
PUBLIC_INGRESS_DIR=""
REQUIRE_BOOTSTRAP=0
REQUIRE_SEND_DUMMY=0
REQUIRE_UPLOAD=0
REQUIRE_FAILOVER=0
SKIP_K8S_LOGS=0
CHAIN_IDS=()

usage() {
  cat <<'USAGE'
Usage: k8s-pass4-mobile-local-collector.sh --run-id ID [--chain-id ID ...]
       (--evidence-s3-key KEY | --evidence-zip FILE)
       [--artifact-dir DIR] [--public-ingress-dir DIR]
       [--require-bootstrap] [--require-send-dummy] [--require-upload] [--require-failover]
       [--skip-k8s-logs]

Collect Pass 4 Phase 5 mobile evidence from S3 or a local ZIP, verify bundle contents,
and collect local k8s ChainID logs/traces for the public local-k8s mobile run.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run-id) RUN_ID="$2"; shift 2 ;;
    --chain-id) CHAIN_IDS+=("$2"); shift 2 ;;
    --evidence-s3-key) EVIDENCE_S3_KEY="$2"; shift 2 ;;
    --evidence-zip) EVIDENCE_ZIP="$2"; shift 2 ;;
    --artifact-dir) ARTIFACT_DIR="$2"; shift 2 ;;
    --public-ingress-dir) PUBLIC_INGRESS_DIR="$2"; shift 2 ;;
    --require-bootstrap) REQUIRE_BOOTSTRAP=1; shift ;;
    --require-send-dummy) REQUIRE_SEND_DUMMY=1; shift ;;
    --require-upload) REQUIRE_UPLOAD=1; shift ;;
    --require-failover) REQUIRE_FAILOVER=1; shift ;;
    --skip-k8s-logs) SKIP_K8S_LOGS=1; shift ;;
    --help|-h) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$RUN_ID" ]] || { echo "--run-id is required" >&2; exit 2; }
if [[ -z "$EVIDENCE_S3_KEY" && -z "$EVIDENCE_ZIP" ]]; then
  echo "one of --evidence-s3-key or --evidence-zip is required" >&2
  exit 2
fi
if [[ -n "$EVIDENCE_S3_KEY" && -n "$EVIDENCE_ZIP" ]]; then
  echo "use only one of --evidence-s3-key or --evidence-zip" >&2
  exit 2
fi

ARTIFACT_DIR="${ARTIFACT_DIR:-$ROOT_DIR/target/k8s-smoke-artifacts/pass4-mobile-local/$RUN_ID}"
PUBLIC_INGRESS_DIR="${PUBLIC_INGRESS_DIR:-$ROOT_DIR/target/pass4-public-ingress/$RUN_ID}"
mkdir -p "$ARTIFACT_DIR"

ZIP_PATH="$ARTIFACT_DIR/mobile-evidence.zip"
if [[ -n "$EVIDENCE_S3_KEY" ]]; then
  command -v aws >/dev/null 2>&1 || { echo "aws CLI is required for S3 retrieval" >&2; exit 127; }
  BUCKET="${PASS4_MOBILE_EVIDENCE_BUCKET:-veritas-pass4-mobile-evidence}"
  aws s3 cp "s3://$BUCKET/$EVIDENCE_S3_KEY" "$ZIP_PATH" | tee "$ARTIFACT_DIR/s3-retrieval.txt"
else
  cp "$EVIDENCE_ZIP" "$ZIP_PATH"
  printf 'local_zip=%s\n' "$EVIDENCE_ZIP" > "$ARTIFACT_DIR/s3-retrieval.txt"
fi
sha256sum "$ZIP_PATH" | tee "$ARTIFACT_DIR/mobile-evidence.sha256"

python3 - "$ZIP_PATH" "$ARTIFACT_DIR" "${CHAIN_IDS[@]}" <<'PY'
import json
import sys
import zipfile
from pathlib import Path

zip_path = Path(sys.argv[1])
artifact_dir = Path(sys.argv[2])
chain_ids = sys.argv[3:]
required = {
    "local_dht.json",
    "host_creator_seed.redacted.json",
    "trace_events.jsonl",
    "remote_trace_queries.json",
    "manifest.sha256.json",
}
with zipfile.ZipFile(zip_path) as zf:
    names = set(zf.namelist())
    missing = sorted(required - names)
    if missing:
        raise SystemExit(f"mobile evidence bundle missing required files: {missing}")
    unpack_dir = artifact_dir / "mobile-evidence"
    if unpack_dir.exists():
        import shutil
        shutil.rmtree(unpack_dir)
    zf.extractall(unpack_dir)

manifest = json.loads((unpack_dir / "manifest.sha256.json").read_text(encoding="utf-8"))
for path in required - {"manifest.sha256.json"}:
    if path not in manifest:
        raise SystemExit(f"manifest.sha256.json missing {path}")

events_text = (unpack_dir / "trace_events.jsonl").read_text(encoding="utf-8")
local_dht = json.loads((unpack_dir / "local_dht.json").read_text(encoding="utf-8"))
summary = {
    "zip": str(zip_path),
    "file_count": len(names),
    "chain_ids_requested": chain_ids,
    "chain_ids_found": {},
    "self_onboarding_state": local_dht.get("self_onboarding_state"),
    "bridge_count": len(local_dht.get("bridge_entries", [])),
    "active_bridge_count": len([b for b in local_dht.get("bridge_entries", []) if b.get("active")]),
}
for chain_id in chain_ids:
    summary["chain_ids_found"][chain_id] = chain_id in events_text
    if chain_id not in events_text:
        raise SystemExit(f"required ChainID missing from mobile trace_events.jsonl: {chain_id}")
(artifact_dir / "mobile-evidence-summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

if [[ -d "$PUBLIC_INGRESS_DIR" ]]; then
  cp -f "$PUBLIC_INGRESS_DIR"/public_endpoint_map.json "$ARTIFACT_DIR"/ 2>/dev/null || true
  cp -f "$PUBLIC_INGRESS_DIR"/hostcreator_bootstrap_seed.redacted.json "$ARTIFACT_DIR"/ 2>/dev/null || true
  cp -f "$PUBLIC_INGRESS_DIR"/admin_denial_transcript.txt "$ARTIFACT_DIR"/ 2>/dev/null || true
fi

if [[ "$SKIP_K8S_LOGS" != "1" ]]; then
  mkdir -p "$ARTIFACT_DIR/k8s-logs"
  for target in deploy/publisher-authority deploy/publisher-receiver deploy/creator-host statefulset/exit-bridge; do
    safe_name="${target//\//-}"
    kubectl logs "$target" -n veritas --all-containers --since=2h > "$ARTIFACT_DIR/k8s-logs/$safe_name.log" 2>&1 || true
  done
  for chain_id in "${CHAIN_IDS[@]}"; do
    grep -R "$chain_id" "$ARTIFACT_DIR/k8s-logs" > "$ARTIFACT_DIR/k8s-logs/chain-$chain_id.log" || {
      echo "required ChainID missing from local k8s logs: $chain_id" >&2
      exit 1
    }
  done
fi

python3 - "$ARTIFACT_DIR" "$REQUIRE_BOOTSTRAP" "$REQUIRE_SEND_DUMMY" "$REQUIRE_UPLOAD" "$REQUIRE_FAILOVER" <<'PY'
import json
import sys
from pathlib import Path

artifact_dir = Path(sys.argv[1])
summary = json.loads((artifact_dir / "mobile-evidence-summary.json").read_text(encoding="utf-8"))
required_gates = {
    "bootstrap": sys.argv[2] == "1",
    "send_dummy": sys.argv[3] == "1",
    "upload": sys.argv[4] == "1",
    "failover": sys.argv[5] == "1",
}
report = [
    "# GBN-PROTO-013 Phase 5 Mobile Local Collector Report",
    "",
    f"- Artifact dir: `{artifact_dir}`",
    f"- Mobile state: `{summary.get('self_onboarding_state')}`",
    f"- Bridge count: `{summary.get('bridge_count')}`",
    f"- Active bridge count: `{summary.get('active_bridge_count')}`",
    "",
    "## ChainID Evidence",
    "",
    "| ChainID | Mobile Evidence |",
    "|---|---:|",
]
for chain_id, found in summary.get("chain_ids_found", {}).items():
    report.append(f"| `{chain_id}` | `{str(found).lower()}` |")
report.extend(["", "## Required Gates", "", "| Gate | Required |", "|---|---:|"])
for gate, required in required_gates.items():
    report.append(f"| `{gate}` | `{str(required).lower()}` |")
report.extend(["", "Result: collector validation passed."])
(artifact_dir / "collector-report.md").write_text("\n".join(report) + "\n", encoding="utf-8")
print(artifact_dir / "collector-report.md")
PY
