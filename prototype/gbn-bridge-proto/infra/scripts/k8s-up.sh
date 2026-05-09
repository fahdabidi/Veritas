#!/usr/bin/env bash
# Bring up the local Conduit topology in k3d.
set -euo pipefail

CLUSTER_NAME="${VERITAS_K3D_CLUSTER:-veritas}"
NAMESPACE="${VERITAS_K8S_NAMESPACE:-veritas}"
SERVERS="${VERITAS_K3D_SERVERS:-1}"
AGENTS="${VERITAS_K3D_AGENTS:-2}"
RUN_SMOKE="${VERITAS_K8S_RUN_SMOKE:-1}"
RUN_CARGO_PERSISTENCE="${VERITAS_K8S_RUN_CARGO_PERSISTENCE:-1}"
DOCKER_STABILITY_SECONDS="${VERITAS_K8S_DOCKER_STABILITY_SECONDS:-3}"
K3D_NODE_STABILITY_SECONDS="${VERITAS_K8S_K3D_NODE_STABILITY_SECONDS:-5}"
POST_SMOKE_STABILITY_SECONDS="${VERITAS_K8S_POST_SMOKE_STABILITY_SECONDS:-45}"
SMOKE_ATTEMPTS="${VERITAS_K8S_SMOKE_ATTEMPTS:-2}"
KUBELET_PROXY_TIMEOUT_SECONDS="${VERITAS_K8S_KUBELET_PROXY_TIMEOUT_SECONDS:-180}"
ALLOW_KUBELET_PROXY_CLUSTER_RECREATE="${VERITAS_K8S_ALLOW_KUBELET_PROXY_CLUSTER_RECREATE:-1}"
WORKLOAD_READY_TIMEOUT_SECONDS="${VERITAS_K8S_WORKLOAD_READY_TIMEOUT_SECONDS:-240}"
WORKLOAD_STABILITY_SECONDS="${VERITAS_K8S_WORKLOAD_STABILITY_SECONDS:-30}"
EXPECTED_CONDUIT_POD_COUNT="${VERITAS_K8S_EXPECTED_CONDUIT_POD_COUNT:-14}"
POSTGRES_ROLLOUT_TIMEOUT="${VERITAS_K8S_POSTGRES_ROLLOUT_TIMEOUT:-240s}"
DEPLOYMENT_ROLLOUT_TIMEOUT="${VERITAS_K8S_DEPLOYMENT_ROLLOUT_TIMEOUT:-240s}"
EXIT_BRIDGE_ROLLOUT_TIMEOUT="${VERITAS_K8S_EXIT_BRIDGE_ROLLOUT_TIMEOUT:-900s}"
PRUNE_OLD_IMAGES="${VERITAS_K8S_PRUNE_OLD_IMAGES:-1}"
PRUNE_OLD_K3D_IMAGES="${VERITAS_K8S_PRUNE_OLD_K3D_IMAGES:-1}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
OVERLAY_DIR="$ROOT_DIR/infra/k8s/conduit/overlays/dev"

command -v python3 >/dev/null 2>&1 || {
  echo "ERROR: 'python3' is required. Run infra/scripts/bootstrap-k8s.sh inside WSL2 first." >&2
  exit 1
}

default_build_source() {
  if command -v git >/dev/null 2>&1 && git -C "$ROOT_DIR" rev-parse HEAD >/dev/null 2>&1; then
    git -C "$ROOT_DIR" rev-parse HEAD
  else
    echo "unknown"
  fi
}

default_build_version() {
  local timestamp source_short dirty=""
  timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
  if command -v git >/dev/null 2>&1 && git -C "$ROOT_DIR" rev-parse --short=12 HEAD >/dev/null 2>&1; then
    source_short="$(git -C "$ROOT_DIR" rev-parse --short=12 HEAD)"
    if ! git -C "$ROOT_DIR" diff --quiet --ignore-submodules -- . ||
      ! git -C "$ROOT_DIR" diff --cached --quiet --ignore-submodules -- .; then
      dirty="-dirty"
    fi
  else
    source_short="nogit"
  fi
  echo "local-${timestamp}-${source_short}${dirty}"
}

sanitize_image_tag() {
  python3 - "$1" <<'PY'
import re
import sys

tag = re.sub(r"[^A-Za-z0-9_.-]+", "-", sys.argv[1]).strip(".-")
if not tag:
    tag = "local"
if not re.match(r"^[A-Za-z0-9_]", tag):
    tag = "v" + tag
print(tag[:128])
PY
}

sanitize_k8s_label() {
  python3 - "$1" <<'PY'
import re
import sys

value = re.sub(r"[^A-Za-z0-9_.-]+", "-", sys.argv[1]).strip(".-")
if not value:
    value = "local"
if not re.match(r"^[A-Za-z0-9]", value):
    value = "v" + value
value = value[:63].rstrip(".-")
if not re.match(r".*[A-Za-z0-9]$", value):
    value = value.rstrip("_.-") or "local"
print(value)
PY
}

BUILD_CREATED="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
BUILD_SOURCE="$(default_build_source)"
BUILD_VERSION="$(sanitize_image_tag "${VERITAS_CONDUIT_BUILD_VERSION:-$(default_build_version)}")"
BUILD_LABEL="$(sanitize_k8s_label "$BUILD_VERSION")"
AUTHORITY_IMAGE="veritas/publisher-authority:${BUILD_VERSION}"
RECEIVER_IMAGE="veritas/publisher-receiver:${BUILD_VERSION}"
BRIDGE_IMAGE="veritas/exit-bridge:${BUILD_VERSION}"
CREATOR_IMAGE="veritas/creator-runner:${BUILD_VERSION}"
BUILD_ARTIFACT_DIR="${VERITAS_K8S_BUILD_ARTIFACT_DIR:-$ROOT_DIR/target/k8s-builds/$BUILD_VERSION}"
VERSIONED_OVERLAY_DIR="$BUILD_ARTIFACT_DIR/kustomize"
DIAGNOSTIC_DIR="$BUILD_ARTIFACT_DIR/diagnostics"
KUBELET_PROXY_CLUSTER_RECREATE_USED=0

docker_restart_count() {
  if command -v systemctl >/dev/null 2>&1; then
    systemctl show docker --property=NRestarts --value 2>/dev/null || true
  fi
}

docker_active_timestamp() {
  if command -v systemctl >/dev/null 2>&1; then
    systemctl show docker --property=ActiveEnterTimestamp --value 2>/dev/null || true
  fi
}

wait_for_docker_stable() {
  docker info >/dev/null
  local before_restarts after_restarts before_started after_started
  before_restarts="$(docker_restart_count)"
  before_started="$(docker_active_timestamp)"
  if [[ "$DOCKER_STABILITY_SECONDS" -gt 0 ]]; then
    sleep "$DOCKER_STABILITY_SECONDS"
  fi
  docker info >/dev/null
  after_restarts="$(docker_restart_count)"
  after_started="$(docker_active_timestamp)"
  if [[ -n "$before_restarts" && -n "$after_restarts" && "$before_restarts" != "$after_restarts" ]]; then
    echo "ERROR: Docker restarted during the stability window (NRestarts $before_restarts -> $after_restarts)." >&2
    exit 1
  fi
  if [[ -n "$before_started" && -n "$after_started" && "$before_started" != "$after_started" ]]; then
    echo "ERROR: Docker restarted during the stability window (ActiveEnterTimestamp changed)." >&2
    exit 1
  fi
}

k3d_container_names() {
  docker ps -a --format '{{.Names}}' |
    grep -E "^k3d-${CLUSTER_NAME}-(server-[0-9]+|agent-[0-9]+|serverlb)$" |
    sort || true
}

expected_k3d_container_count() {
  echo $((SERVERS + AGENTS + 1))
}

k3d_container_fingerprint() {
  local name
  while IFS= read -r name; do
    [[ -n "$name" ]] || continue
    docker inspect "$name" \
      --format '{{.Name}} {{.State.Status}} started={{.State.StartedAt}} finished={{.State.FinishedAt}} exit={{.State.ExitCode}} oom={{.State.OOMKilled}}'
  done < <(k3d_container_names)
}

capture_k3d_diagnostics() {
  local dir="$1"
  mkdir -p "$dir"
  date -u +"%Y-%m-%dT%H:%M:%SZ" >"$dir/captured-at.txt"
  docker ps -a --format 'table {{.Names}}\t{{.Status}}\t{{.Image}}' \
    >"$dir/docker-ps-k3d.txt" 2>&1 || true
  k3d_container_fingerprint >"$dir/k3d-container-fingerprint.txt" 2>&1 || true
  for name in $(k3d_container_names); do
    docker logs --tail=250 "$name" >"$dir/${name}.log" 2>&1 || true
    docker inspect "$name" >"$dir/${name}.inspect.json" 2>&1 || true
  done
  kubectl get nodes -o wide >"$dir/kubectl-nodes.txt" 2>&1 || true
  kubectl -n "$NAMESPACE" get pods -o wide >"$dir/kubectl-pods.txt" 2>&1 || true
  kubectl -n "$NAMESPACE" get events --sort-by=.lastTimestamp \
    >"$dir/kubectl-events.txt" 2>&1 || true
  while read -r pod container; do
    [[ -n "${pod:-}" && -n "${container:-}" ]] || continue
    kubectl -n "$NAMESPACE" logs "$pod" -c "$container" --tail=250 \
      >"$dir/pod-${pod}-${container}.log" 2>&1 || true
    kubectl -n "$NAMESPACE" logs "$pod" -c "$container" --previous --tail=250 \
      >"$dir/pod-${pod}-${container}.previous.log" 2>&1 || true
  done < <(
    kubectl -n "$NAMESPACE" get pods -o json 2>/dev/null |
      python3 -c 'import json,sys; data=json.load(sys.stdin); [print(item["metadata"]["name"], container["name"]) for item in data.get("items", []) for container in item.get("spec", {}).get("containers", [])]' 2>/dev/null || true
  )
  dmesg -T 2>/dev/null |
    grep -Ei 'oom|killed process|out of memory|docker|containerd|ext4|i/o error|unmounting filesystem|mounted filesystem' |
    tail -200 >"$dir/dmesg-indicators.txt" 2>&1 || true
}

ensure_k3d_containers_running() {
  if ! k3d cluster get "$CLUSTER_NAME" >/dev/null 2>&1; then
    return 0
  fi

  local attempt running_count expected_count
  expected_count="$(expected_k3d_container_count)"
  for attempt in {1..60}; do
    running_count="$(
      docker ps --format '{{.Names}}' |
        grep -E "^k3d-${CLUSTER_NAME}-(server-[0-9]+|agent-[0-9]+|serverlb)$" |
        wc -l |
        tr -d ' '
    )"
    if [[ "$running_count" == "$expected_count" ]]; then
      return 0
    fi
    if ((attempt == 1 || attempt % 10 == 0)); then
      echo "k3d backing containers are not all running ($running_count/$expected_count); starting cluster '$CLUSTER_NAME'..."
      k3d cluster start "$CLUSTER_NAME" >/dev/null || true
    fi
    sleep 2
  done

  echo "ERROR: k3d backing containers did not all reach Running." >&2
  docker ps -a --filter "name=k3d-${CLUSTER_NAME}" >&2 || true
  capture_k3d_diagnostics "$DIAGNOSTIC_DIR/k3d-containers-not-running" || true
  exit 1
}

create_k3d_cluster() {
  echo "Creating k3d cluster '$CLUSTER_NAME' (${SERVERS} server, ${AGENTS} agents)..."
  k3d cluster create "$CLUSTER_NAME" \
    --servers "$SERVERS" \
    --agents "$AGENTS" \
    --port "30030:30030@loadbalancer" \
    --wait
}

recreate_k3d_cluster_for_kubelet_proxy() {
  local reason="$1"
  echo "WARNING: kubelet proxy is failing with stale node certificate state: $reason" >&2
  echo "Recreating local k3d cluster '$CLUSTER_NAME' before workload apply." >&2
  capture_k3d_diagnostics "$DIAGNOSTIC_DIR/kubelet-proxy-stale-cert-before-recreate" || true
  k3d cluster delete "$CLUSTER_NAME" >/dev/null 2>&1 || true
  create_k3d_cluster
}

wait_for_k3d_node_stability() {
  local seconds="$1" label="${2:-k3d}" attempt before after
  if [[ "$seconds" -le 0 ]]; then
    return 0
  fi

  for attempt in {1..3}; do
    wait_for_docker_stable
    ensure_k3d_containers_running
    before="$(k3d_container_fingerprint)"
    echo "Validating k3d backing node stability for $label (${seconds}s, attempt $attempt/3)..."
    sleep "$seconds"
    wait_for_docker_stable
    ensure_k3d_containers_running
    after="$(k3d_container_fingerprint)"
    if [[ "$before" == "$after" ]]; then
      return 0
    fi

    echo "WARNING: k3d backing container state changed during '$label' stability window." >&2
    capture_k3d_diagnostics "$DIAGNOSTIC_DIR/k3d-stability-${label//[^A-Za-z0-9_.-]/-}-attempt-$attempt" || true
    k3d cluster start "$CLUSTER_NAME" >/dev/null || true
    sleep 10
  done

  echo "ERROR: k3d backing containers did not stay stable for '$label'." >&2
  capture_k3d_diagnostics "$DIAGNOSTIC_DIR/k3d-stability-${label//[^A-Za-z0-9_.-]/-}-failed" || true
  exit 1
}

ensure_cluster_started() {
  if ! k3d cluster get "$CLUSTER_NAME" >/dev/null 2>&1; then
    return 0
  fi
  if ! kubectl get nodes >/dev/null 2>&1; then
    echo "Kubernetes API is not reachable; starting k3d cluster '$CLUSTER_NAME'..."
    k3d cluster start "$CLUSTER_NAME" >/dev/null
  fi
}

wait_for_cluster_api() {
  local attempt
  ensure_k3d_containers_running
  for attempt in {1..120}; do
    if kubectl get nodes >/dev/null 2>&1; then
      kubectl wait --for=condition=Ready node --all --timeout=180s >/dev/null
      return 0
    fi
    if ((attempt == 1 || attempt % 15 == 0)); then
      echo "Kubernetes API is not reachable yet; recovering k3d cluster '$CLUSTER_NAME'..."
      k3d cluster start "$CLUSTER_NAME" >/dev/null || true
    fi
    sleep 2
  done
  echo "ERROR: Kubernetes API did not become reachable." >&2
  docker ps -a --filter "name=k3d-${CLUSTER_NAME}" >&2 || true
  capture_k3d_diagnostics "$DIAGNOSTIC_DIR/kubernetes-api-unreachable" || true
  exit 1
}

wait_for_kubelet_proxy() {
  local deadline node failed output cert_mismatch last_error pass
  echo "Validating API-server to kubelet proxy on every k3d node..."
  for pass in 1 2; do
    deadline=$((SECONDS + KUBELET_PROXY_TIMEOUT_SECONDS))
    cert_mismatch=0
    last_error=""
    while ((SECONDS < deadline)); do
      failed=0
      while IFS= read -r node; do
        [[ -n "$node" ]] || continue
        if ! output="$(kubectl get --raw "/api/v1/nodes/${node}/proxy/healthz" 2>&1)"; then
          failed=1
          last_error="${node}: ${output}"
          if grep -Fq "certificate is valid for" <<<"$output"; then
            cert_mismatch=1
          fi
          break
        fi
      done < <(kubectl get nodes -o jsonpath='{range .items[*]}{.metadata.name}{"\n"}{end}' 2>/dev/null || true)
      if [[ "$failed" == "0" ]]; then
        return 0
      fi
      sleep 3
    done

    if [[ "$cert_mismatch" == "1" &&
      "$ALLOW_KUBELET_PROXY_CLUSTER_RECREATE" == "1" &&
      "$KUBELET_PROXY_CLUSTER_RECREATE_USED" == "0" ]]; then
      KUBELET_PROXY_CLUSTER_RECREATE_USED=1
      recreate_k3d_cluster_for_kubelet_proxy "$last_error"
      wait_for_cluster_api
      continue
    fi
    break
  done

  echo "ERROR: kubelet proxy did not become healthy on every node." >&2
  capture_k3d_diagnostics "$DIAGNOSTIC_DIR/kubelet-proxy-unhealthy" || true
  exit 1
}

validate_cluster_gate() {
  local label="$1"
  echo "Validating cluster gate: $label"
  wait_for_k3d_node_stability "$K3D_NODE_STABILITY_SECONDS" "$label"
  wait_for_cluster_api
  wait_for_kubelet_proxy
}

write_conduit_pod_fingerprint() {
  local json_path="$1" fingerprint_path="$2"
  if ! kubectl -n "$NAMESPACE" get pods -l app.kubernetes.io/part-of=veritas-conduit -o json \
    >"$json_path"; then
    return 1
  fi
  python3 - "$json_path" "$EXPECTED_CONDUIT_POD_COUNT" >"$fingerprint_path" <<'PY'
import json
import sys

path, expected_count_raw = sys.argv[1:]
expected_count = int(expected_count_raw)
data = json.load(open(path))
roles = {"authority", "receiver", "bridge", "creator"}
rows = []
blocking = []

for item in sorted(data.get("items", []), key=lambda pod: pod["metadata"]["name"]):
    meta = item.get("metadata", {})
    labels = meta.get("labels", {})
    role = labels.get("veritas-role", "")
    if role not in roles:
        continue

    name = meta.get("name", "")
    status = item.get("status", {})
    phase = status.get("phase", "")
    deleting = bool(meta.get("deletionTimestamp"))
    pod_ready = any(
        condition.get("type") == "Ready" and condition.get("status") == "True"
        for condition in status.get("conditions", [])
    )
    container_bits = []
    for container in status.get("containerStatuses", []):
        state = container.get("state", {})
        last_state = container.get("lastState", {})
        state_name = next(iter(state.keys()), "unknown")
        reason = state.get(state_name, {}).get("reason", "")
        last_name = next(iter(last_state.keys()), "none")
        last_reason = last_state.get(last_name, {}).get("reason", "")
        container_bits.append(
            "{name}:ready={ready}:restarts={restarts}:state={state}:reason={reason}:last={last}:last_reason={last_reason}:image_id={image_id}".format(
                name=container.get("name", ""),
                ready=container.get("ready", False),
                restarts=container.get("restartCount", 0),
                state=state_name,
                reason=reason,
                last=last_name,
                last_reason=last_reason,
                image_id=container.get("imageID", ""),
            )
        )
        if not container.get("ready", False) or state_name != "running":
            blocking.append(
                f"{name}/{container.get('name', '')}: state={state_name} reason={reason} ready={container.get('ready', False)}"
            )

    if deleting or phase != "Running" or not pod_ready:
        blocking.append(
            f"{name}: phase={phase} ready={pod_ready} deleting={deleting}"
        )

    rows.append(
        f"{name}|role={role}|phase={phase}|pod_ip={status.get('podIP', '')}|ready={pod_ready}|"
        + "|".join(container_bits)
    )

if len(rows) != expected_count:
    blocking.append(f"expected {expected_count} Conduit workload pods, found {len(rows)}")

if blocking:
    print("Conduit workload readiness blockers:", file=sys.stderr)
    for item in blocking:
        print(f"  - {item}", file=sys.stderr)
    sys.exit(2)

for row in rows:
    print(row)
PY
}

wait_for_conduit_workload_stability() {
  local label="$1" attempt before_json before_fp after_json after_fp
  if [[ "$WORKLOAD_STABILITY_SECONDS" -le 0 ]]; then
    return 0
  fi

  for attempt in {1..3}; do
    validate_cluster_gate "before workload stability $label"
    echo "Waiting for Conduit workload pods to be Ready for $label..."
    if ! kubectl -n "$NAMESPACE" wait \
      --for=condition=Ready \
      pod \
      -l app.kubernetes.io/part-of=veritas-conduit \
      --timeout="${WORKLOAD_READY_TIMEOUT_SECONDS}s" >/dev/null; then
      echo "Conduit workload pods did not all become Ready for $label (attempt $attempt/3)." >&2
      capture_k3d_diagnostics "$DIAGNOSTIC_DIR/workload-ready-${label//[^A-Za-z0-9_.-]/-}-attempt-$attempt" || true
      k3d cluster start "$CLUSTER_NAME" >/dev/null || true
      sleep 15
      continue
    fi

    before_json="$BUILD_ARTIFACT_DIR/workload-${label//[^A-Za-z0-9_.-]/-}-before-$attempt.json"
    before_fp="$BUILD_ARTIFACT_DIR/workload-${label//[^A-Za-z0-9_.-]/-}-before-$attempt.txt"
    after_json="$BUILD_ARTIFACT_DIR/workload-${label//[^A-Za-z0-9_.-]/-}-after-$attempt.json"
    after_fp="$BUILD_ARTIFACT_DIR/workload-${label//[^A-Za-z0-9_.-]/-}-after-$attempt.txt"
    if ! write_conduit_pod_fingerprint "$before_json" "$before_fp"; then
      capture_k3d_diagnostics "$DIAGNOSTIC_DIR/workload-fingerprint-${label//[^A-Za-z0-9_.-]/-}-before-$attempt" || true
      sleep 10
      continue
    fi

    echo "Validating Conduit workload stability for $label (${WORKLOAD_STABILITY_SECONDS}s, attempt $attempt/3)..."
    sleep "$WORKLOAD_STABILITY_SECONDS"
    validate_cluster_gate "after workload stability window $label"

    if ! kubectl -n "$NAMESPACE" wait \
      --for=condition=Ready \
      pod \
      -l app.kubernetes.io/part-of=veritas-conduit \
      --timeout="${WORKLOAD_READY_TIMEOUT_SECONDS}s" >/dev/null; then
      echo "Conduit workload pods lost readiness during $label stability window." >&2
      capture_k3d_diagnostics "$DIAGNOSTIC_DIR/workload-ready-${label//[^A-Za-z0-9_.-]/-}-after-$attempt" || true
      k3d cluster start "$CLUSTER_NAME" >/dev/null || true
      sleep 15
      continue
    fi

    if ! write_conduit_pod_fingerprint "$after_json" "$after_fp"; then
      capture_k3d_diagnostics "$DIAGNOSTIC_DIR/workload-fingerprint-${label//[^A-Za-z0-9_.-]/-}-after-$attempt" || true
      sleep 10
      continue
    fi

    if cmp -s "$before_fp" "$after_fp"; then
      return 0
    fi

    echo "Conduit workload state changed during $label stability window." >&2
    echo "Before:" >&2
    cat "$before_fp" >&2 || true
    echo "After:" >&2
    cat "$after_fp" >&2 || true
    capture_k3d_diagnostics "$DIAGNOSTIC_DIR/workload-stability-${label//[^A-Za-z0-9_.-]/-}-attempt-$attempt" || true
    sleep 15
  done

  echo "ERROR: Conduit workload pods did not stay stable for $label." >&2
  capture_k3d_diagnostics "$DIAGNOSTIC_DIR/workload-stability-${label//[^A-Za-z0-9_.-]/-}-failed" || true
  exit 1
}

import_images() {
  k3d image import \
    "$AUTHORITY_IMAGE" \
    "$RECEIVER_IMAGE" \
    "$BRIDGE_IMAGE" \
    "$CREATOR_IMAGE" \
    -c "$CLUSTER_NAME"
}

verify_images_imported_to_k3d() {
  local nodes node image repo tag missing=0
  nodes="$(docker ps --format '{{.Names}}' | grep -E "^k3d-${CLUSTER_NAME}-(server|agent)-" || true)"
  if [[ -z "$nodes" ]]; then
    echo "No k3d server/agent containers are running for image import verification." >&2
    return 1
  fi

  for node in $nodes; do
    for image in "$AUTHORITY_IMAGE" "$RECEIVER_IMAGE" "$BRIDGE_IMAGE" "$CREATOR_IMAGE"; do
      repo="${image%:*}"
      tag="${image##*:}"
      if ! docker exec "$node" crictl images 2>/dev/null | grep -F "$repo" | grep -F "$tag" >/dev/null; then
        echo "Image $image is not present in k3d node $node." >&2
        missing=1
      fi
    done
  done

  [[ "$missing" -eq 0 ]]
}

import_images_with_retry() {
  local attempt
  for attempt in {1..3}; do
    wait_for_docker_stable
    ensure_cluster_started
    wait_for_cluster_api
    if import_images && verify_images_imported_to_k3d; then
      wait_for_cluster_api
      return 0
    fi
    echo "Image import failed (attempt $attempt/3); checking Docker and k3d before retry..." >&2
    docker ps -a --filter "name=k3d-${CLUSTER_NAME}" >&2 || true
    k3d cluster start "$CLUSTER_NAME" >/dev/null || true
    sleep 10
  done
  echo "ERROR: failed to import images into k3d after retries." >&2
  exit 1
}

kubectl_apply_with_retry() {
  local attempt
  for attempt in {1..3}; do
    validate_cluster_gate "before kubectl apply"
    if kubectl apply -k "$VERSIONED_OVERLAY_DIR"; then
      validate_cluster_gate "after kubectl apply"
      return 0
    fi
    echo "kubectl apply failed (attempt $attempt/3); waiting for cluster API before retry..." >&2
    capture_k3d_diagnostics "$DIAGNOSTIC_DIR/kubectl-apply-attempt-$attempt" || true
    sleep 10
  done
  echo "ERROR: kubectl apply failed after retries." >&2
  exit 1
}

rollout_status_with_retry() {
  local resource="$1" timeout="$2" attempt
  for attempt in {1..3}; do
    validate_cluster_gate "before rollout ${resource//\//-}"
    if kubectl -n "$NAMESPACE" rollout status "$resource" --timeout="$timeout"; then
      validate_cluster_gate "after rollout ${resource//\//-}"
      return 0
    fi
    echo "rollout status failed for $resource (attempt $attempt/3); recovering k3d before retry..." >&2
    capture_k3d_diagnostics "$DIAGNOSTIC_DIR/rollout-${resource//\//-}-attempt-$attempt" || true
    k3d cluster start "$CLUSTER_NAME" >/dev/null || true
    import_images_with_retry
    sleep 15
  done

  echo "ERROR: rollout did not complete for $resource." >&2
  capture_k3d_diagnostics "$DIAGNOSTIC_DIR/rollout-${resource//\//-}-failed" || true
  exit 1
}

run_smoke_with_retry() {
  local attempt
  for attempt in $(seq 1 "$SMOKE_ATTEMPTS"); do
    validate_cluster_gate "before smoke attempt $attempt"
    if "$SCRIPT_DIR/k8s-smoke.sh" --send-dummy; then
      validate_cluster_gate "after smoke attempt $attempt"
      wait_for_conduit_workload_stability "after smoke attempt $attempt"
      return 0
    fi
    echo "k8s smoke validation failed (attempt $attempt/$SMOKE_ATTEMPTS); recovering k3d before retry..." >&2
    capture_k3d_diagnostics "$DIAGNOSTIC_DIR/k8s-smoke-attempt-$attempt" || true
    k3d cluster start "$CLUSTER_NAME" >/dev/null || true
    sleep 20
  done

  echo "ERROR: k8s smoke validation failed after $SMOKE_ATTEMPTS attempt(s)." >&2
  capture_k3d_diagnostics "$DIAGNOSTIC_DIR/k8s-smoke-failed" || true
  exit 1
}

run_postgres_validation_with_retry() {
  local attempt
  for attempt in {1..2}; do
    validate_cluster_gate "before postgres validation attempt $attempt"
    if "$SCRIPT_DIR/k8s-test-publisher-postgres.sh"; then
      validate_cluster_gate "after postgres validation attempt $attempt"
      wait_for_conduit_workload_stability "after postgres validation attempt $attempt"
      return 0
    fi
    echo "Cargo Postgres validation failed (attempt $attempt/2); recovering k3d before retry..." >&2
    capture_k3d_diagnostics "$DIAGNOSTIC_DIR/postgres-validation-attempt-$attempt" || true
    k3d cluster start "$CLUSTER_NAME" >/dev/null || true
    sleep 20
  done

  echo "ERROR: Cargo Postgres validation failed after retries." >&2
  capture_k3d_diagnostics "$DIAGNOSTIC_DIR/postgres-validation-failed" || true
  exit 1
}

delete_legacy_exit_bridge_deployment() {
  if kubectl -n "$NAMESPACE" get deployment/exit-bridge >/dev/null 2>&1; then
    echo "Removing legacy exit-bridge Deployment before applying the StatefulSet topology..."
    kubectl -n "$NAMESPACE" delete deployment/exit-bridge --wait=true --timeout=120s
  fi
}

prepare_versioned_overlay() {
  local base_resource_path
  mkdir -p "$VERSIONED_OVERLAY_DIR"
  base_resource_path="$(
    python3 - "$VERSIONED_OVERLAY_DIR" "$ROOT_DIR/infra/k8s/conduit/base" <<'PY'
import os
import sys

print(os.path.relpath(sys.argv[2], sys.argv[1]).replace("\\", "/"))
PY
  )"
  cp "$OVERLAY_DIR/password.txt" "$VERSIONED_OVERLAY_DIR/password.txt"
  write_deployment_build_patch \
    publisher-authority \
    publisher-authority \
    "$AUTHORITY_IMAGE" \
    publisher-authority \
    "$VERSIONED_OVERLAY_DIR/authority-build-patch.yaml"
  write_deployment_build_patch \
    publisher-receiver \
    publisher-receiver \
    "$RECEIVER_IMAGE" \
    publisher-receiver \
    "$VERSIONED_OVERLAY_DIR/receiver-build-patch.yaml"
  write_deployment_build_patch \
    exit-bridge \
    exit-bridge \
    "$BRIDGE_IMAGE" \
    exit-bridge \
    "$VERSIONED_OVERLAY_DIR/bridge-build-patch.yaml" \
    StatefulSet
  write_deployment_build_patch \
    creator-host \
    creator-runner \
    "$CREATOR_IMAGE" \
    creator-host \
    "$VERSIONED_OVERLAY_DIR/creator-host-build-patch.yaml"
  write_deployment_build_patch \
    creator-new \
    creator-runner \
    "$CREATOR_IMAGE" \
    creator-new \
    "$VERSIONED_OVERLAY_DIR/creator-new-build-patch.yaml"
  cat >"$VERSIONED_OVERLAY_DIR/kustomization.yaml" <<EOF
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
namespace: $NAMESPACE
resources:
  - $base_resource_path
images:
  - name: veritas/publisher-authority
    newName: veritas/publisher-authority
    newTag: $BUILD_VERSION
  - name: veritas/publisher-receiver
    newName: veritas/publisher-receiver
    newTag: $BUILD_VERSION
  - name: veritas/exit-bridge
    newName: veritas/exit-bridge
    newTag: $BUILD_VERSION
  - name: veritas/creator-runner
    newName: veritas/creator-runner
    newTag: $BUILD_VERSION
patches:
  - path: authority-build-patch.yaml
  - path: receiver-build-patch.yaml
  - path: bridge-build-patch.yaml
  - path: creator-host-build-patch.yaml
  - path: creator-new-build-patch.yaml
generatorOptions:
  disableNameSuffixHash: true
secretGenerator:
  - name: postgres-credentials
    behavior: replace
    literals:
      - POSTGRES_DB=veritas_conduit
      - POSTGRES_USER=veritas
      - GBN_BRIDGE_POSTGRES_DATABASE=veritas_conduit
      - GBN_BRIDGE_POSTGRES_USER=veritas
    files:
      - POSTGRES_PASSWORD=password.txt
      - GBN_BRIDGE_POSTGRES_PASSWORD=password.txt
EOF
}

write_deployment_build_patch() {
  local deployment="$1" container="$2" image="$3" component="$4" output="$5" kind="${6:-Deployment}"
  cat >"$output" <<EOF
apiVersion: apps/v1
kind: $kind
metadata:
  name: $deployment
  labels:
    veritas.dev/build-version: "$BUILD_LABEL"
  annotations:
    veritas.dev/build-version: "$BUILD_VERSION"
    veritas.dev/build-created: "$BUILD_CREATED"
    veritas.dev/build-source: "$BUILD_SOURCE"
spec:
  template:
    metadata:
      labels:
        veritas.dev/build-version: "$BUILD_LABEL"
        veritas.dev/component: "$component"
      annotations:
        veritas.dev/build-version: "$BUILD_VERSION"
        veritas.dev/build-created: "$BUILD_CREATED"
        veritas.dev/build-source: "$BUILD_SOURCE"
        veritas.dev/image: "$image"
        veritas.dev/rollout-requested-at: "$BUILD_CREATED"
    spec:
      containers:
        - name: $container
          image: "$image"
          env:
            - name: VERITAS_CONDUIT_BUILD_VERSION
              value: "$BUILD_VERSION"
            - name: VERITAS_CONDUIT_BUILD_SOURCE
              value: "$BUILD_SOURCE"
            - name: VERITAS_CONDUIT_BUILD_CREATED
              value: "$BUILD_CREATED"
            - name: VERITAS_CONDUIT_IMAGE
              value: "$image"
EOF
}

build_metadata_patch() {
  local container="$1" image="$2" component="$3"
  python3 - "$BUILD_VERSION" "$BUILD_LABEL" "$BUILD_CREATED" "$BUILD_SOURCE" "$container" "$image" "$component" <<'PY'
import json
import sys

version, label, created, source, container, image, component = sys.argv[1:]
print(json.dumps({
    "metadata": {
        "labels": {
            "veritas.dev/build-version": label,
        },
        "annotations": {
            "veritas.dev/build-version": version,
            "veritas.dev/build-created": created,
            "veritas.dev/build-source": source,
        },
    },
    "spec": {
        "template": {
            "metadata": {
                "labels": {
                    "veritas.dev/build-version": label,
                    "veritas.dev/component": component,
                },
                "annotations": {
                    "veritas.dev/build-version": version,
                    "veritas.dev/build-created": created,
                    "veritas.dev/build-source": source,
                    "veritas.dev/image": image,
                    "veritas.dev/rollout-requested-at": created,
                },
            },
            "spec": {
                "containers": [
                    {
                        "name": container,
                        "image": image,
                        "env": [
                            {"name": "VERITAS_CONDUIT_BUILD_VERSION", "value": version},
                            {"name": "VERITAS_CONDUIT_BUILD_SOURCE", "value": source},
                            {"name": "VERITAS_CONDUIT_BUILD_CREATED", "value": created},
                            {"name": "VERITAS_CONDUIT_IMAGE", "value": image},
                        ],
                    },
                ],
            },
        },
    },
}))
PY
}

set_workload_build() {
  local resource="$1" container="$2" image="$3" component="$4"
  kubectl -n "$NAMESPACE" patch "$resource" \
    --type strategic \
    -p "$(build_metadata_patch "$container" "$image" "$component")"
}

set_versioned_deployments() {
  echo "Pinning deployments to build version $BUILD_VERSION..."
  set_workload_build deployment/publisher-authority publisher-authority "$AUTHORITY_IMAGE" publisher-authority
  set_workload_build deployment/publisher-receiver publisher-receiver "$RECEIVER_IMAGE" publisher-receiver
  set_workload_build statefulset/exit-bridge exit-bridge "$BRIDGE_IMAGE" exit-bridge
  set_workload_build deployment/creator-host creator-runner "$CREATOR_IMAGE" creator-host
  set_workload_build deployment/creator-new creator-runner "$CREATOR_IMAGE" creator-new
}

prune_old_local_images() {
  [[ "$PRUNE_OLD_IMAGES" == "1" ]] || return 0
  echo "Pruning older local Conduit image tags..."
  local repo image
  for repo in veritas/publisher-authority veritas/publisher-receiver veritas/exit-bridge veritas/creator-runner; do
    while IFS= read -r image; do
      [[ -z "$image" || "$image" == *":<none>" ]] && continue
      case "$image" in
        "$AUTHORITY_IMAGE"|"$RECEIVER_IMAGE"|"$BRIDGE_IMAGE"|"$CREATOR_IMAGE") continue ;;
      esac
      docker image rm "$image" >/dev/null 2>&1 || true
    done < <(docker images "$repo" --format '{{.Repository}}:{{.Tag}}')
  done
}

prune_old_k3d_images() {
  [[ "$PRUNE_OLD_K3D_IMAGES" == "1" ]] || return 0
  echo "Pruning older imported Conduit images from k3d nodes..."
  local node
  while IFS= read -r node; do
    docker exec "$node" sh -lc "
      set -eu
      command -v ctr >/dev/null 2>&1 || exit 0
      ctr -n k8s.io images ls -q |
        grep -E '^docker.io/veritas/(publisher-authority|publisher-receiver|exit-bridge|creator-runner):' |
        grep -v ':${BUILD_VERSION}$' |
        xargs -r ctr -n k8s.io images rm >/dev/null 2>&1 || true
    " >/dev/null 2>&1 || true
  done < <(docker ps --format '{{.Names}}' | grep -E "^k3d-${CLUSTER_NAME}-(server|agent)-")
}

write_build_summary() {
  mkdir -p "$BUILD_ARTIFACT_DIR"
  cat >"$BUILD_ARTIFACT_DIR/build.env" <<EOF
VERITAS_CONDUIT_BUILD_VERSION=$BUILD_VERSION
VERITAS_CONDUIT_BUILD_CREATED=$BUILD_CREATED
VERITAS_CONDUIT_BUILD_SOURCE=$BUILD_SOURCE
VERITAS_AUTHORITY_IMAGE=$AUTHORITY_IMAGE
VERITAS_RECEIVER_IMAGE=$RECEIVER_IMAGE
VERITAS_BRIDGE_IMAGE=$BRIDGE_IMAGE
VERITAS_CREATOR_IMAGE=$CREATOR_IMAGE
VERITAS_VERSIONED_OVERLAY_DIR=$VERSIONED_OVERLAY_DIR
EOF
}

wait_for_current_image_set() {
  echo "Waiting for stale Conduit pods to terminate..."
  local attempt pod_json
  pod_json="$BUILD_ARTIFACT_DIR/pods-version-check.json"
  for attempt in {1..60}; do
    if ! kubectl -n "$NAMESPACE" get pods -l app.kubernetes.io/part-of=veritas-conduit -o json \
      >"$pod_json"; then
      echo "Pod image-set check could not reach Kubernetes API (attempt $attempt/60); recovering k3d..."
      k3d cluster start "$CLUSTER_NAME" >/dev/null || true
      sleep 5
      continue
    fi
    if python3 - "$pod_json" "$AUTHORITY_IMAGE" "$RECEIVER_IMAGE" "$BRIDGE_IMAGE" "$CREATOR_IMAGE" "$BUILD_LABEL" "$BUILD_VERSION" <<'PY'
import json
import sys

path, authority_image, receiver_image, bridge_image, creator_image, build_label, build_version = sys.argv[1:]
expected = {
    "authority": authority_image,
    "receiver": receiver_image,
    "bridge": bridge_image,
    "creator": creator_image,
}
data = json.load(open(path))
blocking = []
for item in data.get("items", []):
    meta = item.get("metadata", {})
    labels = meta.get("labels", {})
    role = labels.get("veritas-role", "")
    if role not in expected:
        continue
    name = meta.get("name", "")
    if meta.get("deletionTimestamp"):
        blocking.append(f"{name}: waiting for terminating stale pod to be removed")
        continue
    phase = item.get("status", {}).get("phase", "")
    if phase != "Running":
        blocking.append(f"{name}: phase is {phase!r}, not Running")
        continue
    images = {container.get("image", "") for container in item.get("spec", {}).get("containers", [])}
    if expected[role] not in images:
        blocking.append(f"{name}: expected image {expected[role]}, got {sorted(images)}")
    if labels.get("veritas.dev/build-version", "") != build_label:
        blocking.append(f"{name}: missing build label {build_label}")
    annotations = meta.get("annotations", {})
    if annotations.get("veritas.dev/build-version", "") != build_version:
        blocking.append(f"{name}: missing full build-version annotation")
if blocking:
    print("\n".join(blocking), file=sys.stderr)
    raise SystemExit(1)
PY
    then
      return 0
    fi
    sleep 2
  done

  echo "ERROR: stale or unversioned Conduit pods remained after rollout." >&2
  kubectl -n "$NAMESPACE" get pods -l app.kubernetes.io/part-of=veritas-conduit -o wide >&2 || true
  exit 1
}

print_running_versions() {
  mkdir -p "$BUILD_ARTIFACT_DIR"
  kubectl -n "$NAMESPACE" get pods -l app.kubernetes.io/part-of=veritas-conduit -o json \
    >"$BUILD_ARTIFACT_DIR/running-pods.json"
  python3 - "$BUILD_ARTIFACT_DIR/running-pods.json" <<'PY'
import json
import sys

data = json.load(open(sys.argv[1]))
print("Running Conduit pod image versions:")
print("POD\tROLE\tBUILD\tIMAGE\tIMAGE_ID")
for item in sorted(data.get("items", []), key=lambda pod: pod["metadata"]["name"]):
    meta = item.get("metadata", {})
    labels = meta.get("labels", {})
    annotations = meta.get("annotations", {})
    role = labels.get("veritas-role", "")
    if role not in {"authority", "receiver", "bridge", "creator"}:
        continue
    if meta.get("deletionTimestamp") or item.get("status", {}).get("phase") != "Running":
        continue
    spec = item.get("spec", {})
    status = item.get("status", {})
    containers = spec.get("containers", [])
    statuses = {status.get("name"): status for status in status.get("containerStatuses", [])}
    for container in containers:
        name = container.get("name", "")
        image = container.get("image", "")
        image_id = statuses.get(name, {}).get("imageID", "")
        print("\t".join([
            meta.get("name", ""),
            labels.get("veritas-role", ""),
            annotations.get("veritas.dev/build-version", labels.get("veritas.dev/build-version", "")),
            image,
            image_id,
        ]))
PY
}

print_topology_summary() {
  mkdir -p "$BUILD_ARTIFACT_DIR"
  kubectl -n "$NAMESPACE" get pods -l app.kubernetes.io/part-of=veritas-conduit -o json \
    >"$BUILD_ARTIFACT_DIR/topology-pods.json"
  python3 - "$BUILD_ARTIFACT_DIR/topology-pods.json" <<'PY'
import json
import sys

data = json.load(open(sys.argv[1]))
roles = {
    "creator-host": [],
    "creator-new": [],
    "exit-bridge": [],
    "publisher-authority": [],
    "publisher-receiver": [],
}
for item in data.get("items", []):
    meta = item.get("metadata", {})
    labels = meta.get("labels", {})
    name = meta.get("name", "")
    app = labels.get("app.kubernetes.io/name", "")
    status = item.get("status", {})
    ip = status.get("podIP", "")
    ready = all(
        condition.get("type") != "Ready" or condition.get("status") == "True"
        for condition in status.get("conditions", [])
    )
    if app in roles:
        roles[app].append((name, ip, ready))

def ready_count(app):
    return sum(1 for _, _, ready in roles[app] if ready)

def first_ip(app):
    return next((ip for _, ip, _ in roles[app] if ip), "pending")

print("Conduit topology summary:")
print(f"  creator-host  : {ready_count('creator-host')}/1 Ready  {first_ip('creator-host')}:9090")
print(f"  creator-new   : {ready_count('creator-new')}/1 Ready   {first_ip('creator-new')}:9090")
print(f"  exit-bridge   : {ready_count('exit-bridge')}/10 Ready")
publisher_ready = ready_count("publisher-authority") + ready_count("publisher-receiver")
print(f"  publisher     : {publisher_ready}/2 Ready   authority={first_ip('publisher-authority')}:9090 receiver={first_ip('publisher-receiver')}:9090")
PY
}

for dep in docker k3d kubectl python3; do
  command -v "$dep" >/dev/null 2>&1 || {
    echo "ERROR: '$dep' is required. Run infra/scripts/bootstrap-k8s.sh inside WSL2 first." >&2
    exit 1
  }
done

mkdir -p "$DIAGNOSTIC_DIR"
trap 'status=$?; if [[ "$status" -ne 0 ]]; then capture_k3d_diagnostics "$DIAGNOSTIC_DIR/final-failure" || true; fi' EXIT

wait_for_docker_stable

if [[ ! -f "$OVERLAY_DIR/password.txt" ]]; then
  echo "Generating dev-only Postgres password at $OVERLAY_DIR/password.txt"
  umask 077
  python3 - <<'PY' > "$OVERLAY_DIR/password.txt"
import secrets
import string

alphabet = string.ascii_letters + string.digits
print("".join(secrets.choice(alphabet) for _ in range(32)))
PY
fi

if ! k3d cluster get "$CLUSTER_NAME" >/dev/null 2>&1; then
  create_k3d_cluster
else
  echo "Using existing k3d cluster '$CLUSTER_NAME'."
  ensure_cluster_started
fi
validate_cluster_gate "initial cluster start"
ALLOW_KUBELET_PROXY_CLUSTER_RECREATE=0

echo "Conduit build version: $BUILD_VERSION"
echo "Build source: $BUILD_SOURCE"
write_build_summary
prepare_versioned_overlay

echo "Building local Conduit images..."
docker build -f "$ROOT_DIR/Dockerfile.publisher-authority" \
  --build-arg "VERITAS_BUILD_VERSION=$BUILD_VERSION" \
  --build-arg "VERITAS_BUILD_SOURCE=$BUILD_SOURCE" \
  --build-arg "VERITAS_BUILD_CREATED=$BUILD_CREATED" \
  -t "$AUTHORITY_IMAGE" "$ROOT_DIR"
docker build -f "$ROOT_DIR/Dockerfile.publisher-receiver" \
  --build-arg "VERITAS_BUILD_VERSION=$BUILD_VERSION" \
  --build-arg "VERITAS_BUILD_SOURCE=$BUILD_SOURCE" \
  --build-arg "VERITAS_BUILD_CREATED=$BUILD_CREATED" \
  -t "$RECEIVER_IMAGE" "$ROOT_DIR"
docker build -f "$ROOT_DIR/Dockerfile.bridge" \
  --build-arg "VERITAS_BUILD_VERSION=$BUILD_VERSION" \
  --build-arg "VERITAS_BUILD_SOURCE=$BUILD_SOURCE" \
  --build-arg "VERITAS_BUILD_CREATED=$BUILD_CREATED" \
  -t "$BRIDGE_IMAGE" "$ROOT_DIR"
docker build -f "$ROOT_DIR/Dockerfile.creator-runner" \
  --build-arg "VERITAS_BUILD_VERSION=$BUILD_VERSION" \
  --build-arg "VERITAS_BUILD_SOURCE=$BUILD_SOURCE" \
  --build-arg "VERITAS_BUILD_CREATED=$BUILD_CREATED" \
  -t "$CREATOR_IMAGE" "$ROOT_DIR"
validate_cluster_gate "after local image builds"

prune_old_local_images

echo "Importing local images into k3d..."
import_images_with_retry
validate_cluster_gate "after image import"

echo "Applying Conduit manifests..."
delete_legacy_exit_bridge_deployment
kubectl_apply_with_retry

validate_cluster_gate "before image pinning"
set_versioned_deployments
validate_cluster_gate "after image pinning"

echo "Waiting for Conduit pods..."
rollout_status_with_retry statefulset/postgres "$POSTGRES_ROLLOUT_TIMEOUT"
rollout_status_with_retry deployment/publisher-authority "$DEPLOYMENT_ROLLOUT_TIMEOUT"
rollout_status_with_retry deployment/publisher-receiver "$DEPLOYMENT_ROLLOUT_TIMEOUT"
rollout_status_with_retry statefulset/exit-bridge "$EXIT_BRIDGE_ROLLOUT_TIMEOUT"
rollout_status_with_retry deployment/creator-host "$DEPLOYMENT_ROLLOUT_TIMEOUT"
rollout_status_with_retry deployment/creator-new "$DEPLOYMENT_ROLLOUT_TIMEOUT"
wait_for_current_image_set
validate_cluster_gate "after image-set verification"
wait_for_conduit_workload_stability "post-rollout"

kubectl -n "$NAMESPACE" get pods,svc,statefulset,deployment
print_running_versions
print_topology_summary
prune_old_k3d_images
validate_cluster_gate "after k3d image pruning"
wait_for_conduit_workload_stability "after k3d image pruning"

if [[ "$RUN_SMOKE" == "1" ]]; then
  run_smoke_with_retry
else
  echo "Skipping k8s smoke validation because VERITAS_K8S_RUN_SMOKE=$RUN_SMOKE."
fi
wait_for_k3d_node_stability "$POST_SMOKE_STABILITY_SECONDS" "post-smoke settle"
wait_for_cluster_api
wait_for_kubelet_proxy
wait_for_conduit_workload_stability "post-smoke settle"

if [[ "$RUN_CARGO_PERSISTENCE" == "1" ]]; then
  run_postgres_validation_with_retry
else
  echo "Skipping Cargo Postgres validation because VERITAS_K8S_RUN_CARGO_PERSISTENCE=$RUN_CARGO_PERSISTENCE."
fi
wait_for_k3d_node_stability "$POST_SMOKE_STABILITY_SECONDS" "final settle"
wait_for_cluster_api
wait_for_kubelet_proxy
wait_for_conduit_workload_stability "final settle"

echo "Conduit local Kubernetes topology is up."
echo "Cluster:   $CLUSTER_NAME"
echo "Namespace: $NAMESPACE"
echo "Build:     $BUILD_VERSION"
echo "Images:"
echo "  authority: $AUTHORITY_IMAGE"
echo "  receiver:  $RECEIVER_IMAGE"
echo "  bridge:    $BRIDGE_IMAGE"
echo "  creator:   $CREATOR_IMAGE"
echo "Build metadata: $BUILD_ARTIFACT_DIR/build.env"
echo "Rendered overlay: $VERSIONED_OVERLAY_DIR"
