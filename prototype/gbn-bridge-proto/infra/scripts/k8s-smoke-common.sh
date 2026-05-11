#!/usr/bin/env bash
# Shared helpers for local Conduit Kubernetes smoke tests.

SMOKE_RETRY_ATTEMPTS="${VERITAS_K8S_SMOKE_RETRY_ATTEMPTS:-6}"
SMOKE_RETRY_DELAY_SECONDS="${VERITAS_K8S_SMOKE_RETRY_DELAY_SECONDS:-5}"
SMOKE_DOCKER_STABILITY_SECONDS="${VERITAS_K8S_DOCKER_STABILITY_SECONDS:-10}"
SMOKE_K3D_NODE_STABILITY_SECONDS="${VERITAS_K8S_K3D_NODE_STABILITY_SECONDS:-30}"
SMOKE_FLANNEL_TIMEOUT_SECONDS="${VERITAS_K8S_FLANNEL_TIMEOUT_SECONDS:-180}"
SMOKE_WORKLOAD_STABILITY_SECONDS="${VERITAS_K8S_WORKLOAD_STABILITY_SECONDS:-30}"
SMOKE_DIAGNOSTICS_COLLECTED=0

smoke_log() {
  printf '%s\n' "$*" >&2
}

smoke_require_deps() {
  local dep
  for dep in kubectl python3 curl; do
    command -v "$dep" >/dev/null 2>&1 || {
      echo "ERROR: '$dep' is required." >&2
      exit 1
    }
  done
}

smoke_collect_diagnostics() {
  [[ "${SMOKE_DIAGNOSTICS_COLLECTED:-0}" == "1" ]] && return 0
  SMOKE_DIAGNOSTICS_COLLECTED=1

  local dir="${ARTIFACT_DIR:-target/k8s-smoke-artifacts}/diagnostics"
  mkdir -p "$dir"
  {
    echo "# Smoke Diagnostics"
    date -Is
    echo
    echo "namespace=${NAMESPACE:-}"
    echo "admin_transport=${SMOKE_ADMIN_TRANSPORT:-unset}"
    echo "docker_stability_seconds=${SMOKE_DOCKER_STABILITY_SECONDS:-}"
    echo "k3d_node_stability_seconds=${SMOKE_K3D_NODE_STABILITY_SECONDS:-}"
    echo "flannel_timeout_seconds=${SMOKE_FLANNEL_TIMEOUT_SECONDS:-}"
    echo "workload_stability_seconds=${SMOKE_WORKLOAD_STABILITY_SECONDS:-}"
  } >"$dir/summary.txt"

  kubectl cluster-info >"$dir/kubectl-cluster-info.txt" 2>&1 || true
  kubectl get nodes -o wide >"$dir/kubectl-nodes.txt" 2>&1 || true
  kubectl get namespace "${NAMESPACE:-veritas}" >"$dir/kubectl-namespace.txt" 2>&1 || true
  kubectl -n "${NAMESPACE:-veritas}" get pods,svc,statefulset,deployment -o wide \
    >"$dir/kubectl-veritas-objects.txt" 2>&1 || true
  kubectl -n "${NAMESPACE:-veritas}" get events --sort-by=.lastTimestamp \
    >"$dir/kubectl-veritas-events.txt" 2>&1 || true
  kubectl -n "${OBS_NS:-observability}" get pods,svc -o wide \
    >"$dir/kubectl-observability-objects.txt" 2>&1 || true
  if command -v docker >/dev/null 2>&1; then
    docker info >"$dir/docker-info.txt" 2>&1 || true
    docker ps -a --no-trunc >"$dir/docker-ps.txt" 2>&1 || true
    docker ps -a --no-trunc --filter "name=k3d-${VERITAS_K3D_CLUSTER:-veritas}" \
      >"$dir/docker-k3d-ps.txt" 2>&1 || true
  fi
  if command -v systemctl >/dev/null 2>&1; then
    systemctl status docker --no-pager -l >"$dir/docker-systemctl-status.txt" 2>&1 || true
    systemctl show docker --property=NRestarts --property=ActiveEnterTimestamp \
      >"$dir/docker-systemctl-show.txt" 2>&1 || true
  fi
  if command -v journalctl >/dev/null 2>&1; then
    journalctl -u docker --since=-30min --no-pager -n 300 \
      >"$dir/docker-journal.txt" 2>&1 || true
  fi

  smoke_log "Diagnostics captured in $dir"
}

smoke_fail() {
  smoke_log "ERROR: $*"
  smoke_collect_diagnostics
  exit 1
}

smoke_retry() {
  local attempts="${1:-$SMOKE_RETRY_ATTEMPTS}" delay="${2:-$SMOKE_RETRY_DELAY_SECONDS}"
  shift 2
  local attempt
  for ((attempt = 1; attempt <= attempts; attempt++)); do
    if "$@"; then
      return 0
    fi
    if ((attempt < attempts)); then
      sleep "$delay"
    fi
  done
  return 1
}

smoke_now_ms() {
  python3 -c 'import time; print(int(time.time() * 1000))'
}

smoke_json_field() {
  local field="$1"
  python3 -c "import json,sys; print(json.load(sys.stdin).get('$field',''))"
}

smoke_json_metric_field() {
  local field="$1"
  python3 -c "import json,sys; data=json.load(sys.stdin); print(data.get('snapshot', {}).get('$field', 0))"
}

smoke_artifact_dir() {
  local kind="$1"
  local root="${VERITAS_K8S_SMOKE_ARTIFACT_ROOT:-target/k8s-smoke-artifacts}"
  local run_id="${VERITAS_K8S_SMOKE_RUN_ID:-$(date +%Y%m%d-%H%M%S)-$$}"
  ARTIFACT_DIR="${ARTIFACT_DIR:-${root}/${kind}/${run_id}}"
  mkdir -p "$ARTIFACT_DIR"
  echo "$ARTIFACT_DIR"
}

smoke_archive_report() {
  local report_prefix="$1" source_path="${2:-$ARTIFACT_DIR/report.md}"
  [[ -f "$source_path" ]] || {
    smoke_log "Report archive skipped; source report missing: $source_path"
    return 0
  }

  local repo_root report_root run_id dest_path
  if [[ -n "${VERITAS_REPO_ROOT:-}" ]]; then
    repo_root="$VERITAS_REPO_ROOT"
  elif [[ -n "${ROOT_DIR:-}" ]]; then
    repo_root="$(cd "$ROOT_DIR/../.." && pwd)"
  else
    repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
  fi
  report_root="${VERITAS_K8S_SMOKE_REPORT_ROOT:-$repo_root/docs/prototyping/Conduit/Full-Implementation-Plan-Pass3/Test-Reports}"
  run_id="$(basename "$ARTIFACT_DIR")"
  if [[ "$run_id" == "bootstrap" || "$run_id" == "route" ]]; then
    local parent_run_id
    parent_run_id="$(basename "$(dirname "$ARTIFACT_DIR")")"
    if [[ -n "$parent_run_id" && "$parent_run_id" != "." && "$parent_run_id" != "/" ]]; then
      run_id="${parent_run_id}-${run_id}"
    fi
  fi
  dest_path="$report_root/${report_prefix}-${run_id}.md"
  mkdir -p "$report_root"
  cp "$source_path" "$dest_path"
  smoke_log "Tracked evidence report: $dest_path"
}

smoke_shell_quote() {
  local value="$1"
  printf "'%s'" "$(printf '%s' "$value" | sed "s/'/'\\\\''/g")"
}

smoke_k3d_node_container() {
  local cluster="${VERITAS_K3D_CLUSTER:-veritas}"
  local candidate
  if [[ -n "${VERITAS_K3D_NODE_CONTAINER:-}" ]]; then
    candidate="$VERITAS_K3D_NODE_CONTAINER"
    if [[ "$(docker inspect -f '{{.State.Running}}' "$candidate" 2>/dev/null || true)" == "true" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  fi

  for candidate in "k3d-${cluster}-server-0" "k3d-${cluster}-agent-0" "k3d-${cluster}-agent-1"; do
    if [[ "$(docker inspect -f '{{.State.Running}}' "$candidate" 2>/dev/null || true)" == "true" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done

  docker ps --format '{{.Names}}' 2>/dev/null |
    awk -v prefix="k3d-${cluster}-" '$0 ~ ("^" prefix "(server|agent)-") { print; exit }'
}

smoke_count_running_k3d_nodes() {
  local cluster="${VERITAS_K3D_CLUSTER:-veritas}"
  docker ps --format '{{.Names}}' 2>/dev/null |
    awk -v prefix="k3d-${cluster}-" '$0 ~ ("^" prefix "(server|agent)-") { count++ } END { print count + 0 }'
}

smoke_running_k3d_node_names() {
  local cluster="${VERITAS_K3D_CLUSTER:-veritas}"
  docker ps --format '{{.Names}}' 2>/dev/null |
    awk -v prefix="k3d-${cluster}-" '$0 ~ ("^" prefix "(server|agent)-") { print }'
}

smoke_running_k3d_container_names() {
  local cluster="${VERITAS_K3D_CLUSTER:-veritas}"
  docker ps --format '{{.Names}}' 2>/dev/null |
    awk -v prefix="k3d-${cluster}-" '$0 ~ ("^" prefix "(server|agent)-") || $0 == (prefix "serverlb") { print }'
}

smoke_k3d_start_snapshot() {
  local names=()
  mapfile -t names < <(smoke_running_k3d_container_names)
  [[ "${#names[@]}" -gt 0 ]] || return 1
  docker inspect -f '{{.Name}} running={{.State.Running}} started={{.State.StartedAt}} restarting={{.State.Restarting}}' "${names[@]}" 2>/dev/null | sort
}

smoke_wait_k3d_containers_stable() {
  command -v docker >/dev/null 2>&1 || return 0
  command -v k3d >/dev/null 2>&1 || return 0

  local expected="${VERITAS_K3D_EXPECTED_NODE_CONTAINERS:-3}"
  local attempt running before after
  for ((attempt = 1; attempt <= 6; attempt++)); do
    running="$(smoke_count_running_k3d_nodes)"
    if [[ "$running" -lt "$expected" ]]; then
      smoke_log "Waiting for k3d node containers ($running/$expected running)..."
      sleep 5
      continue
    fi

    before="$(smoke_k3d_start_snapshot || true)"
    if [[ -z "$before" ]]; then
      sleep 5
      continue
    fi

    if [[ "$SMOKE_K3D_NODE_STABILITY_SECONDS" -gt 0 ]]; then
      sleep "$SMOKE_K3D_NODE_STABILITY_SECONDS"
    fi
    after="$(smoke_k3d_start_snapshot || true)"
    if [[ -n "$after" && "$before" == "$after" ]]; then
      return 0
    fi
    smoke_log "k3d container start state changed during stability window; retrying..."
  done

  smoke_fail "k3d node containers did not remain stable."
}

smoke_wait_flannel_ready() {
  command -v docker >/dev/null 2>&1 || return 0
  command -v kubectl >/dev/null 2>&1 || return 0

  local deadline=$((SECONDS + SMOKE_FLANNEL_TIMEOUT_SECONDS))
  local nodes=() node missing
  while ((SECONDS < deadline)); do
    kubectl wait --for=condition=Ready node --all --timeout=10s >/dev/null 2>&1 || {
      sleep 2
      continue
    }

    mapfile -t nodes < <(smoke_running_k3d_node_names)
    if [[ "${#nodes[@]}" -eq 0 ]]; then
      sleep 2
      continue
    fi

    missing=0
    for node in "${nodes[@]}"; do
      if ! docker exec "$node" sh -lc 'test -s /run/flannel/subnet.env' >/dev/null 2>&1; then
        missing=1
        break
      fi
    done
    if [[ "$missing" -eq 0 ]]; then
      return 0
    fi
    sleep 2
  done

  smoke_fail "k3d flannel subnet state did not become ready on every node."
}

smoke_docker_restart_count() {
  if command -v systemctl >/dev/null 2>&1; then
    systemctl show docker --property=NRestarts --value 2>/dev/null || true
  fi
}

smoke_docker_active_timestamp() {
  if command -v systemctl >/dev/null 2>&1; then
    systemctl show docker --property=ActiveEnterTimestamp --value 2>/dev/null || true
  fi
}

smoke_ensure_docker_stable() {
  command -v docker >/dev/null 2>&1 || return 0

  docker info >/dev/null 2>&1 || smoke_fail "Docker daemon is not reachable."

  local before_restarts after_restarts before_started after_started
  before_restarts="$(smoke_docker_restart_count)"
  before_started="$(smoke_docker_active_timestamp)"
  if [[ "$SMOKE_DOCKER_STABILITY_SECONDS" -gt 0 ]]; then
    sleep "$SMOKE_DOCKER_STABILITY_SECONDS"
  fi
  docker info >/dev/null 2>&1 || smoke_fail "Docker daemon stopped during the stability window."
  after_restarts="$(smoke_docker_restart_count)"
  after_started="$(smoke_docker_active_timestamp)"

  if [[ -n "$before_restarts" && -n "$after_restarts" && "$before_restarts" != "$after_restarts" ]]; then
    smoke_fail "Docker restarted during the smoke-test stability window (NRestarts $before_restarts -> $after_restarts)."
  fi
  if [[ -n "$before_started" && -n "$after_started" && "$before_started" != "$after_started" ]]; then
    smoke_fail "Docker restarted during the smoke-test stability window (ActiveEnterTimestamp changed)."
  fi
}

smoke_ensure_k3d_nodes_running() {
  local cluster="${VERITAS_K3D_CLUSTER:-veritas}"
  command -v k3d >/dev/null 2>&1 || return 0
  command -v docker >/dev/null 2>&1 || return 0
  k3d cluster get "$cluster" >/dev/null 2>&1 || return 0

  local running
  running="$(smoke_count_running_k3d_nodes)"
  if [[ "$running" -lt 1 ]]; then
    smoke_log "No running k3d node containers found; starting cluster '$cluster'..."
    k3d cluster start "$cluster" >/dev/null
  fi
  smoke_wait_k3d_containers_stable
}

smoke_select_admin_transport() {
  local requested="${VERITAS_K8S_ADMIN_TRANSPORT:-auto}"
  case "${SMOKE_ADMIN_TRANSPORT:-$requested}" in
    exec|docker)
      SMOKE_ADMIN_TRANSPORT="${SMOKE_ADMIN_TRANSPORT:-$requested}"
      return 0
      ;;
    auto)
      if command -v docker >/dev/null 2>&1 && [[ -n "$(smoke_k3d_node_container)" ]]; then
        SMOKE_ADMIN_TRANSPORT=docker
      else
        SMOKE_ADMIN_TRANSPORT=exec
      fi
      echo "Admin smoke transport: $SMOKE_ADMIN_TRANSPORT" >&2
      ;;
    *)
      echo "ERROR: VERITAS_K8S_ADMIN_TRANSPORT must be auto, exec, or docker." >&2
      exit 2
      ;;
  esac
}

smoke_admin_curl_exec() {
  local pod="$1" container="$2" method="$3" path="$4" body="${5:-}"
  local target inner
  target="http://127.0.0.1:${ADMIN_PORT:-9090}${path}"
  if [[ -n "$body" ]]; then
    inner="curl -sS -X $method -H 'Content-Type: application/json' $(smoke_shell_quote "$target") -d $(smoke_shell_quote "$body")"
  else
    inner="curl -sS -X $method $(smoke_shell_quote "$target")"
  fi
  kubectl -n "$NAMESPACE" exec "$pod" -c "$container" -- sh -lc "$inner"
}

smoke_admin_curl_docker() {
  local pod="$1" _container="$2" method="$3" path="$4" body="${5:-}"
  local node pod_ip target inner request_timeout
  request_timeout="${VERITAS_K8S_ADMIN_REQUEST_TIMEOUT_SECONDS:-300}"
  node="$(smoke_k3d_node_container)"
  if [[ -z "$node" ]]; then
    echo "ERROR: no running k3d node container found for Docker admin transport." >&2
    exit 1
  fi
  if ! docker exec "$node" sh -lc 'command -v wget >/dev/null 2>&1' >/dev/null; then
    echo "ERROR: k3d node container '$node' does not include wget." >&2
    exit 1
  fi

  pod_ip="${NODE_IP_BY_POD[$pod]:-}"
  if [[ -z "$pod_ip" ]]; then
    pod_ip="$(
      kubectl -n "$NAMESPACE" get pod "$pod" \
        -o jsonpath='{.status.podIP}'
    )"
  fi
  if [[ -z "$pod_ip" ]]; then
    echo "ERROR: could not resolve pod IP for $pod." >&2
    exit 1
  fi

  target="http://${pod_ip}:${ADMIN_PORT:-9090}${path}"
  case "$method" in
    GET)
      inner="wget -q -T $(smoke_shell_quote "$request_timeout") -O - $(smoke_shell_quote "$target")"
      ;;
    POST)
      inner="wget -q -T $(smoke_shell_quote "$request_timeout") -O - --header 'Content-Type: application/json' --post-data $(smoke_shell_quote "$body") $(smoke_shell_quote "$target")"
      ;;
    *)
      echo "ERROR: Docker admin transport only supports GET and POST, got '$method'." >&2
      exit 2
      ;;
  esac
  docker exec "$node" sh -lc "$inner"
}

smoke_admin_curl() {
  local requested="${VERITAS_K8S_ADMIN_TRANSPORT:-auto}"
  local output status attempt transport
  smoke_select_admin_transport
  for ((attempt = 1; attempt <= SMOKE_RETRY_ATTEMPTS; attempt++)); do
    transport="$SMOKE_ADMIN_TRANSPORT"
    case "$transport" in
      docker) output="$(smoke_admin_curl_docker "$@" 2>&1)" && status=0 || status=$? ;;
      exec) output="$(smoke_admin_curl_exec "$@" 2>&1)" && status=0 || status=$? ;;
      *) smoke_fail "unsupported admin transport '$transport'" ;;
    esac
    if [[ "$status" -eq 0 ]]; then
      printf '%s' "$output"
      return 0
    fi

    if [[ "$requested" == "auto" && "$transport" == "exec" ]] &&
      printf '%s' "$output" | grep -E 'x509|error dialing backend|tls: failed|container not found|No agent available' >/dev/null 2>&1 &&
      command -v docker >/dev/null 2>&1 && [[ -n "$(smoke_k3d_node_container)" ]]; then
      smoke_log "kubectl exec admin transport failed; falling back to Docker-network admin transport."
      SMOKE_ADMIN_TRANSPORT=docker
      continue
    fi

    if ((attempt < SMOKE_RETRY_ATTEMPTS)); then
      smoke_log "Admin request failed via $transport (attempt $attempt/$SMOKE_RETRY_ATTEMPTS); retrying..."
      sleep "$SMOKE_RETRY_DELAY_SECONDS"
    fi
  done

  smoke_log "$output"
  smoke_fail "admin request failed after $SMOKE_RETRY_ATTEMPTS attempt(s): pod=$1 container=$2 method=$3 path=$4"
}

smoke_admin_curl_once() {
  local requested="${VERITAS_K8S_ADMIN_TRANSPORT:-auto}"
  local output status transport
  smoke_select_admin_transport

  transport="$SMOKE_ADMIN_TRANSPORT"
  case "$transport" in
    docker) output="$(smoke_admin_curl_docker "$@" 2>&1)" && status=0 || status=$? ;;
    exec) output="$(smoke_admin_curl_exec "$@" 2>&1)" && status=0 || status=$? ;;
    *) smoke_fail "unsupported admin transport '$transport'" ;;
  esac
  if [[ "$status" -eq 0 ]]; then
    printf '%s' "$output"
    return 0
  fi

  if [[ "$requested" == "auto" && "$transport" == "exec" ]] &&
    printf '%s' "$output" | grep -E 'x509|error dialing backend|tls: failed|container not found|No agent available' >/dev/null 2>&1 &&
    command -v docker >/dev/null 2>&1 && [[ -n "$(smoke_k3d_node_container)" ]]; then
    smoke_log "kubectl exec admin transport failed; falling back to Docker-network admin transport."
    SMOKE_ADMIN_TRANSPORT=docker
    output="$(smoke_admin_curl_docker "$@" 2>&1)" && status=0 || status=$?
    if [[ "$status" -eq 0 ]]; then
      printf '%s' "$output"
      return 0
    fi
  fi

  smoke_log "$output"
  smoke_fail "non-idempotent admin request failed without retry: pod=$1 container=$2 method=$3 path=$4"
}

smoke_admin_curl_try_once() {
  local requested="${VERITAS_K8S_ADMIN_TRANSPORT:-auto}"
  local output status transport
  smoke_select_admin_transport

  transport="$SMOKE_ADMIN_TRANSPORT"
  case "$transport" in
    docker) output="$(smoke_admin_curl_docker "$@" 2>&1)" && status=0 || status=$? ;;
    exec) output="$(smoke_admin_curl_exec "$@" 2>&1)" && status=0 || status=$? ;;
    *) echo "unsupported admin transport '$transport'"; return 2 ;;
  esac
  if [[ "$status" -eq 0 ]]; then
    printf '%s' "$output"
    return 0
  fi

  if [[ "$requested" == "auto" && "$transport" == "exec" ]] &&
    printf '%s' "$output" | grep -E 'x509|error dialing backend|tls: failed|container not found|No agent available' >/dev/null 2>&1 &&
    command -v docker >/dev/null 2>&1 && [[ -n "$(smoke_k3d_node_container)" ]]; then
    smoke_log "kubectl exec admin transport failed; falling back to Docker-network admin transport."
    SMOKE_ADMIN_TRANSPORT=docker
    output="$(smoke_admin_curl_docker "$@" 2>&1)" && status=0 || status=$?
    if [[ "$status" -eq 0 ]]; then
      printf '%s' "$output"
      return 0
    fi
  fi

  printf '%s' "$output"
  return "$status"
}

smoke_pod_for_selector() {
  local selector="$1"
  kubectl -n "$NAMESPACE" get pod -l "$selector" -o json |
    python3 -c 'import json,sys
data=json.load(sys.stdin)
for item in data.get("items", []):
    if item.get("metadata", {}).get("deletionTimestamp"):
        continue
    statuses=item.get("status", {}).get("containerStatuses", [])
    ready=bool(statuses) and all(s.get("ready", False) for s in statuses)
    if item.get("status", {}).get("phase") == "Running" and ready:
        print(item.get("metadata", {}).get("name", ""))
        break'
}

smoke_check_rollouts() {
  echo "Checking rollout status in namespace '$NAMESPACE'..."
  smoke_ensure_cluster_api
  smoke_retry 6 5 kubectl get namespace "$NAMESPACE" >/dev/null ||
    smoke_fail "namespace '$NAMESPACE' is not available."
  smoke_retry 3 10 kubectl -n "$NAMESPACE" rollout status statefulset/postgres --timeout=120s ||
    smoke_fail "postgres rollout did not complete."
  smoke_retry 3 10 kubectl -n "$NAMESPACE" rollout status deployment/publisher-authority --timeout=120s ||
    smoke_fail "publisher-authority rollout did not complete."
  smoke_retry 3 10 kubectl -n "$NAMESPACE" rollout status deployment/publisher-receiver --timeout=120s ||
    smoke_fail "publisher-receiver rollout did not complete."
  smoke_retry 3 10 kubectl -n "$NAMESPACE" rollout status statefulset/exit-bridge --timeout=180s ||
    smoke_fail "exit-bridge rollout did not complete."
  smoke_retry 3 10 kubectl -n "$NAMESPACE" rollout status deployment/creator-host --timeout=120s ||
    smoke_fail "creator-host rollout did not complete."
  smoke_retry 3 10 kubectl -n "$NAMESPACE" rollout status deployment/creator-new --timeout=120s ||
    smoke_fail "creator-new rollout did not complete."
  smoke_wait_conduit_workload_stability
}

smoke_ensure_cluster_api() {
  smoke_ensure_docker_stable
  smoke_ensure_k3d_nodes_running

  if smoke_retry 6 5 kubectl get namespace "$NAMESPACE" >/dev/null 2>&1; then
    smoke_wait_nodes_ready
    smoke_wait_flannel_ready
    return 0
  fi

  local cluster="${VERITAS_K3D_CLUSTER:-veritas}"
  if command -v k3d >/dev/null 2>&1 && k3d cluster get "$cluster" >/dev/null 2>&1; then
    echo "Kubernetes API is not reachable; starting existing k3d cluster '$cluster'..."
    k3d cluster start "$cluster" >/dev/null
    smoke_ensure_docker_stable
    for _ in {1..90}; do
      if kubectl get namespace "$NAMESPACE" >/dev/null 2>&1; then
        smoke_wait_nodes_ready
        smoke_wait_flannel_ready
        return 0
      fi
      sleep 2
    done
  fi

  smoke_fail "Kubernetes API is not reachable for namespace '$NAMESPACE'."
}

smoke_wait_nodes_ready() {
  kubectl wait --for=condition=Ready node --all --timeout=180s >/dev/null 2>&1 ||
    smoke_fail "not all k3d nodes became Ready."
}

smoke_conduit_pod_restart_snapshot() {
  kubectl -n "$NAMESPACE" get pods -l app.kubernetes.io/part-of=veritas-conduit -o json |
    python3 -c 'import json,sys
data=json.load(sys.stdin)
rows=[]
for item in data.get("items", []):
    name=item.get("metadata", {}).get("name", "")
    phase=item.get("status", {}).get("phase", "")
    statuses=item.get("status", {}).get("containerStatuses", [])
    ready=all(s.get("ready", False) for s in statuses) if statuses else False
    restarts=sum(int(s.get("restartCount", 0)) for s in statuses)
    ids=",".join(sorted(str(s.get("containerID", "")) for s in statuses))
    rows.append(f"{name} phase={phase} ready={ready} restarts={restarts} ids={ids}")
print("\n".join(sorted(rows)))'
}

smoke_wait_conduit_workload_stability() {
  kubectl -n "$NAMESPACE" wait --for=condition=Ready pod -l app.kubernetes.io/part-of=veritas-conduit --timeout=240s >/dev/null 2>&1 ||
    smoke_fail "Conduit pods did not become Ready."

  local before after
  before="$(smoke_conduit_pod_restart_snapshot)"
  if [[ "$SMOKE_WORKLOAD_STABILITY_SECONDS" -gt 0 ]]; then
    sleep "$SMOKE_WORKLOAD_STABILITY_SECONDS"
  fi
  kubectl -n "$NAMESPACE" wait --for=condition=Ready pod -l app.kubernetes.io/part-of=veritas-conduit --timeout=120s >/dev/null 2>&1 ||
    smoke_fail "Conduit pods lost readiness during stability window."
  after="$(smoke_conduit_pod_restart_snapshot)"

  if [[ "$before" != "$after" ]]; then
    {
      echo "# Before"
      printf '%s\n' "$before"
      echo "# After"
      printf '%s\n' "$after"
    } >&2
    smoke_fail "Conduit pods restarted or changed container IDs during stability window."
  fi
}

smoke_discover_nodes() {
  local attempt
  for ((attempt = 1; attempt <= 36; attempt++)); do
    AUTHORITY_POD="$(smoke_pod_for_selector 'veritas-role=authority')"
    RECEIVER_POD="$(smoke_pod_for_selector 'veritas-role=receiver')"
    CREATOR_HOST_POD="$(smoke_pod_for_selector 'app.kubernetes.io/name=creator-host')"
    CREATOR_NEW_POD="$(smoke_pod_for_selector 'app.kubernetes.io/name=creator-new')"
    mapfile -t BRIDGE_PODS < <(
      kubectl -n "$NAMESPACE" get pods -l veritas-role=bridge -o json |
        python3 -c 'import json,sys
data=json.load(sys.stdin)
names=[]
for item in data.get("items", []):
    if item.get("metadata", {}).get("deletionTimestamp"):
        continue
    statuses=item.get("status", {}).get("containerStatuses", [])
    ready=bool(statuses) and all(s.get("ready", False) for s in statuses)
    if item.get("status", {}).get("phase") == "Running" and ready:
        names.append(item.get("metadata", {}).get("name", ""))
print("\n".join(sorted(names)))'
    )

    if [[ -n "$AUTHORITY_POD" && -n "$RECEIVER_POD" && -n "$CREATOR_HOST_POD" && -n "$CREATOR_NEW_POD" && "${#BRIDGE_PODS[@]}" -ge "$EXPECTED_BRIDGES" ]]; then
      break
    fi

    if ((attempt == 36)); then
      smoke_log "Expected authority, receiver, creator-host, creator-new, and $EXPECTED_BRIDGES bridge pods."
      kubectl -n "$NAMESPACE" get pods -o wide >&2 || true
      smoke_fail "Conduit pod discovery did not stabilize."
    fi
    sleep 5
  done

  NODE_CHECKS=(
    "$AUTHORITY_POD:publisher-authority:authority"
    "$RECEIVER_POD:publisher-receiver:receiver"
    "$CREATOR_HOST_POD:creator-runner:creator"
    "$CREATOR_NEW_POD:creator-runner:creator"
  )
  local pod
  for pod in "${BRIDGE_PODS[@]}"; do
    NODE_CHECKS+=("$pod:exit-bridge:bridge")
  done

  kubectl -n "$NAMESPACE" get pods -o json >"$ARTIFACT_DIR/pods.json"
  declare -gA NODE_IP_BY_POD=()
  while IFS=$'\t' read -r pod_ip_name pod_ip; do
    [[ -n "$pod_ip_name" && -n "$pod_ip" ]] && NODE_IP_BY_POD["$pod_ip_name"]="$pod_ip"
  done < <(
    python3 -c 'import json,sys
data=json.load(open(sys.argv[1]))
for item in data.get("items", []):
    meta=item.get("metadata", {})
    status=item.get("status", {})
    if meta.get("deletionTimestamp") or status.get("phase") != "Running":
        continue
    print("{}\t{}".format(meta.get("name", ""), status.get("podIP", "")))' \
      "$ARTIFACT_DIR/pods.json"
  )
  echo "Discovered ${#NODE_CHECKS[@]} Conduit node(s)."
}

smoke_check_admin_metrics() {
  local check pod container
  for check in "${NODE_CHECKS[@]}"; do
    pod="${check%%:*}"
    container="${check#*:}"
    container="${container%%:*}"
    smoke_admin_curl "$pod" "$container" GET /v1/admin/metrics >/dev/null
  done
}

smoke_wait_for_bridge_registry() {
  echo "Waiting for bridge registry..."
  local count
  for _ in {1..36}; do
    smoke_admin_curl "$AUTHORITY_POD" publisher-authority GET /v1/admin/bridges \
      >"$ARTIFACT_DIR/bridges.json"
    count="$(
      python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1])).get("bridges", [])))' \
        "$ARTIFACT_DIR/bridges.json"
    )"
    if [[ "$count" -ge "$EXPECTED_BRIDGES" ]]; then
      echo "Registered bridges: $count"
      return 0
    fi
    sleep 5
  done
  echo "ERROR: only ${count:-0} bridge(s) registered; expected $EXPECTED_BRIDGES." >&2
  cat "$ARTIFACT_DIR/bridges.json" >&2
  smoke_fail "bridge registry did not reach expected size."
}

smoke_start_observability() {
  OBS_NS="${OBS_NS:-observability}"
  PROM_PORT="${PROM_PORT:-$(smoke_choose_port 19090)}"
  LOKI_PORT="${LOKI_PORT:-$(smoke_choose_port 13100)}"
  TEMPO_PORT="${TEMPO_PORT:-$(smoke_choose_port 13200)}"
  mkdir -p "$ARTIFACT_DIR/port-forward"

  kubectl -n "$OBS_NS" port-forward svc/kube-prom-prometheus "$PROM_PORT:9090" \
    >"$ARTIFACT_DIR/port-forward/prometheus.log" 2>&1 &
  PROM_PF_PID=$!
  kubectl -n "$OBS_NS" port-forward svc/loki "$LOKI_PORT:3100" \
    >"$ARTIFACT_DIR/port-forward/loki.log" 2>&1 &
  LOKI_PF_PID=$!
  kubectl -n "$OBS_NS" port-forward svc/tempo "$TEMPO_PORT:3200" \
    >"$ARTIFACT_DIR/port-forward/tempo.log" 2>&1 &
  TEMPO_PF_PID=$!

  smoke_wait_tcp "$PROM_PORT" prometheus
  smoke_wait_tcp "$LOKI_PORT" loki
  smoke_wait_tcp "$TEMPO_PORT" tempo

  PROM_URL="http://127.0.0.1:${PROM_PORT}"
  LOKI_URL="http://127.0.0.1:${LOKI_PORT}"
  TEMPO_URL="http://127.0.0.1:${TEMPO_PORT}"

  smoke_retry 12 5 curl -fsS "$PROM_URL/-/ready" >/dev/null ||
    smoke_fail "Prometheus did not become ready."
  smoke_retry 12 5 curl -fsS "$LOKI_URL/ready" >/dev/null ||
    smoke_fail "Loki did not become ready."
  smoke_retry 12 5 curl -fsS "$TEMPO_URL/ready" >/dev/null ||
    smoke_fail "Tempo did not become ready."
}

smoke_stop_observability() {
  local pid
  for pid in "${PROM_PF_PID:-}" "${LOKI_PF_PID:-}" "${TEMPO_PF_PID:-}"; do
    [[ -n "$pid" ]] && kill "$pid" >/dev/null 2>&1 || true
  done
}

smoke_wait_tcp() {
  local port="$1" name="$2" attempt
  for attempt in {1..30}; do
    local pid=""
    case "$name" in
      prometheus) pid="${PROM_PF_PID:-}" ;;
      loki) pid="${LOKI_PF_PID:-}" ;;
      tempo) pid="${TEMPO_PF_PID:-}" ;;
    esac
    if [[ -n "$pid" ]] && ! kill -0 "$pid" >/dev/null 2>&1; then
      echo "ERROR: $name port-forward exited before opening 127.0.0.1:$port." >&2
      [[ -f "$ARTIFACT_DIR/port-forward/${name}.log" ]] && cat "$ARTIFACT_DIR/port-forward/${name}.log" >&2
      smoke_fail "$name port-forward failed."
    fi
    if (echo >"/dev/tcp/127.0.0.1/${port}") >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "ERROR: $name port-forward did not open on 127.0.0.1:$port." >&2
  [[ -f "$ARTIFACT_DIR/port-forward/${name}.log" ]] && cat "$ARTIFACT_DIR/port-forward/${name}.log" >&2
  smoke_fail "$name port-forward did not open."
}

smoke_choose_port() {
  local preferred="$1"
  if ! (echo >"/dev/tcp/127.0.0.1/${preferred}") >/dev/null 2>&1; then
    printf '%s\n' "$preferred"
    return 0
  fi
  python3 - <<'PY'
import socket

sock = socket.socket()
sock.bind(("127.0.0.1", 0))
print(sock.getsockname()[1])
sock.close()
PY
}

smoke_prom_query() {
  local query="$1" output="$2"
  curl -fsS -G --data-urlencode "query=$query" "$PROM_URL/api/v1/query" >"$output"
}

smoke_loki_query() {
  local query="$1" output="$2"
  curl -fsS -G --data-urlencode "query=$query" "$LOKI_URL/loki/api/v1/query" >"$output"
}

smoke_loki_query_range() {
  local query="$1" output="$2" start_ns="$3" end_ns="$4" limit="${5:-5000}"
  curl -fsS -G \
    --data-urlencode "query=$query" \
    --data-urlencode "start=$start_ns" \
    --data-urlencode "end=$end_ns" \
    --data-urlencode "limit=$limit" \
    "$LOKI_URL/loki/api/v1/query_range" >"$output"
}

smoke_tempo_query_chain() {
  local chain_id="$1" output="$2"
  local limit="${VERITAS_TEMPO_SEARCH_LIMIT:-200}"
  curl -fsS -G --data-urlencode "q={ .chain_id = \"$chain_id\" }" \
    --data-urlencode "limit=$limit" \
    "$TEMPO_URL/api/search" >"$output" 2>/dev/null || printf '{"traces":[]}\n' >"$output"
}

smoke_tempo_query_chain_service() {
  local chain_id="$1" service_name="$2" output="$3"
  local limit="${VERITAS_TEMPO_SEARCH_LIMIT:-200}"
  curl -fsS -G --data-urlencode "q={ resource.service.name = \"$service_name\" && .chain_id = \"$chain_id\" }" \
    --data-urlencode "limit=$limit" \
    "$TEMPO_URL/api/search" >"$output" 2>/dev/null || printf '{"traces":[]}\n' >"$output"
}

smoke_loki_query_chain_actor() {
  local chain_id="$1" actor_id="$2" output="$3"
  local query end_ns
  query="{namespace=\"$NAMESPACE\"} |= \"$chain_id\" |= \"$actor_id\""
  if [[ -n "${SMOKE_LOKI_QUERY_START_NS:-}" ]]; then
    end_ns="$(date +%s%N)"
    smoke_loki_query_range "$query" "$output" "$SMOKE_LOKI_QUERY_START_NS" "$end_ns"
  else
    smoke_loki_query "$query" "$output"
  fi
}

smoke_pod_list_by_role() {
  local role="$1"
  kubectl -n "$NAMESPACE" get pods -l "veritas-role=$role" -o json |
    python3 -c 'import json,sys
data=json.load(sys.stdin)
print(json.dumps([
    {
        "pod": item.get("metadata", {}).get("name", ""),
        "phase": item.get("status", {}).get("phase", ""),
        "pod_ip": item.get("status", {}).get("podIP", ""),
    }
    for item in data.get("items", [])
    if not item.get("metadata", {}).get("deletionTimestamp")
], sort_keys=True))'
}

smoke_bootstrap_session_query() {
  local chain_id="$1" bootstrap_session_id="$2" output="$3" path
  if [[ -n "$bootstrap_session_id" ]]; then
    path="/v1/admin/bootstrap-session?bootstrap_session_id=${bootstrap_session_id}"
  else
    path="/v1/admin/bootstrap-session?chain_id=${chain_id}"
  fi
  smoke_admin_curl "$AUTHORITY_POD" publisher-authority GET "$path" >"$output"
}

smoke_frames_by_chain_id() {
  local chain_id="$1" output="$2" limit="${3:-10}"
  smoke_admin_curl "$AUTHORITY_POD" publisher-authority GET "/v1/admin/frames?chain_id=${chain_id}&limit=${limit}" >"$output"
}

smoke_creator_upload_session() {
  local session_id="$1" output="$2"
  smoke_admin_curl "$CREATOR_NEW_POD" creator-runner GET "/v1/admin/upload-sessions/${session_id}" >"$output"
}

smoke_creator_upload_dispatch_plan() {
  local session_id="$1" output="$2"
  smoke_admin_curl "$CREATOR_NEW_POD" creator-runner GET "/v1/admin/upload-sessions/${session_id}/dispatch-plan" >"$output"
}

smoke_received_upload_session() {
  local session_id="$1" output="$2"
  smoke_admin_curl "$AUTHORITY_POD" publisher-authority GET "/v1/admin/received-upload-sessions/${session_id}" >"$output"
}

smoke_received_dummy_frame() {
  local chain_id="$1" output="$2"
  smoke_admin_curl "$AUTHORITY_POD" publisher-authority GET "/v1/admin/received-dummy-frames/${chain_id}" >"$output"
}

smoke_collect_dht_evidence() {
  local chain_id="$1" label="$2" dir safe bridge_metadata bridge_id
  dir="$ARTIFACT_DIR/dht-evidence/$label"
  mkdir -p "$dir/bridge-local-dht" "$dir/bridge-node-metadata" "$dir/publisher-bridge-entry"

  smoke_admin_curl "$AUTHORITY_POD" publisher-authority GET "/v1/admin/publisher-dht?chain_id=${chain_id}" \
    >"$dir/publisher-dht.json"
  if ! smoke_admin_curl "$AUTHORITY_POD" publisher-authority GET /v1/admin/local-dht \
    >"$dir/publisher-local-dht.json"; then
    printf '{"state":"unavailable","role":"publisher"}\n' >"$dir/publisher-local-dht.json"
  fi
  smoke_admin_curl "$CREATOR_HOST_POD" creator-runner GET /v1/admin/local-dht \
    >"$dir/creator-host-local-dht.json"
  smoke_admin_curl "$CREATOR_NEW_POD" creator-runner GET /v1/admin/local-dht \
    >"$dir/creator-new-local-dht.json"

  local pod
  for pod in "${BRIDGE_PODS[@]}"; do
    safe="$(printf '%s' "$pod" | tr -c 'A-Za-z0-9_.-' '_')"
    if smoke_admin_curl "$pod" exit-bridge GET /v1/admin/node-metadata \
      >"$dir/bridge-node-metadata/${safe}.json"; then
      bridge_metadata="$dir/bridge-node-metadata/${safe}.json"
      bridge_id="$(python3 - "$bridge_metadata" <<'PY'
import json
import sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
print(data.get("actor_id") or data.get("node_id") or "")
PY
)"
      if [[ -n "$bridge_id" ]]; then
        smoke_admin_curl "$AUTHORITY_POD" publisher-authority GET "/v1/admin/bridges/${bridge_id}/dht-entry" \
          >"$dir/publisher-bridge-entry/${bridge_id}.json"
      fi
    else
      printf '{"error":"node_metadata_unavailable","pod":%s}\n' "$(python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$pod")" \
        >"$dir/bridge-node-metadata/${safe}.json"
    fi
    if ! smoke_admin_curl "$pod" exit-bridge GET /v1/admin/local-dht \
      >"$dir/bridge-local-dht/${safe}.json"; then
      printf '{"state":"unavailable","role":"bridge","pod":%s}\n' "$(python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$pod")" \
        >"$dir/bridge-local-dht/${safe}.json"
    fi
  done

  python3 - "$dir" "$chain_id" "$EXPECTED_BRIDGES" <<'PY'
import glob
import json
import pathlib
import sys
import time

root = pathlib.Path(sys.argv[1])
chain_id = sys.argv[2]
expected = int(sys.argv[3])
now_ms = int(time.time() * 1000)

def fail(code, **detail):
    raise SystemExit(json.dumps({"failure": code, **detail}, sort_keys=True))

publisher = json.load(open(root / "publisher-dht.json", encoding="utf-8"))
creator_new = json.load(open(root / "creator-new-local-dht.json", encoding="utf-8"))
creator_host = json.load(open(root / "creator-host-local-dht.json", encoding="utf-8"))
publisher_entries = publisher.get("bridge_dht_entries") or []
publisher_ids = {entry.get("bridge_id") for entry in publisher_entries if entry.get("bridge_id")}
creator_entries = creator_new.get("bridge_entries") or []
creator_ids = {entry.get("bridge_id") for entry in creator_entries if entry.get("bridge_id")}
active_creator_ids = {
    entry.get("bridge_id")
    for entry in creator_entries
    if entry.get("bridge_id")
    and entry.get("active") is True
    and entry.get("reachability_class") != "relay_only"
    and int(entry.get("entry_expiry_ms") or 0) > now_ms
    and int(entry.get("lease_expiry_ms") or 0) > now_ms
}
tunnel_ids = {
    tunnel.get("peer_id")
    for tunnel in creator_new.get("active_tunnels") or []
    if tunnel.get("peer_role") == "exit_bridge"
}
bridge_metadata_ids = set()
for path in glob.glob(str(root / "bridge-node-metadata" / "*.json")):
    data = json.load(open(path, encoding="utf-8"))
    actor_id = data.get("actor_id") or data.get("node_id")
    if actor_id:
        bridge_metadata_ids.add(actor_id)
per_entry_ids = set()
for path in glob.glob(str(root / "publisher-bridge-entry" / "*.json")):
    data = json.load(open(path, encoding="utf-8"))
    bridge = data.get("bridge") or data
    bridge_id = bridge.get("bridge_id")
    if bridge_id:
        per_entry_ids.add(bridge_id)

if publisher.get("chain_id") != chain_id:
    fail("publisher_dht_chain_id_mismatch", actual=publisher.get("chain_id"), expected=chain_id)
if len(publisher_ids) != expected:
    fail("publisher_dht_count_mismatch", actual=len(publisher_ids), expected=expected, ids=sorted(publisher_ids))
if len(creator_ids) != expected:
    fail("creator_dht_count_mismatch", actual=len(creator_ids), expected=expected, ids=sorted(creator_ids))
if publisher_ids != creator_ids:
    fail("publisher_creator_dht_mismatch", publisher=sorted(publisher_ids), creator=sorted(creator_ids))
if active_creator_ids != publisher_ids:
    fail("creator_active_dht_mismatch", active=sorted(active_creator_ids), expected=sorted(publisher_ids))
if not publisher_ids.issubset(tunnel_ids):
    fail("creator_active_tunnel_mismatch", missing=sorted(publisher_ids - tunnel_ids))
if bridge_metadata_ids and bridge_metadata_ids != publisher_ids:
    fail("bridge_metadata_dht_mismatch", metadata=sorted(bridge_metadata_ids), publisher=sorted(publisher_ids))
if per_entry_ids != publisher_ids:
    fail("publisher_per_bridge_entry_mismatch", per_entry=sorted(per_entry_ids), publisher=sorted(publisher_ids))
publisher_entry = creator_new.get("publisher_entry") or {}
if not publisher_entry.get("encryption_pub_key"):
    fail("creator_missing_publisher_encryption_key")
if not creator_new.get("creator_entry"):
    fail("creator_missing_self_dht_entry")
if not creator_new.get("host_creator_entry"):
    fail("creator_missing_host_dht_entry")
if creator_new.get("self_onboarding_state") != "onboarded":
    fail("creator_not_onboarded", state=creator_new.get("self_onboarding_state"))

summary = {
    "chain_id": chain_id,
    "publisher_dht_entry_count": len(publisher_ids),
    "creator_new_dht_entry_count": len(creator_ids),
    "creator_new_active_bridge_count": len(active_creator_ids),
    "creator_new_active_tunnel_count": len(tunnel_ids & publisher_ids),
    "bridge_metadata_count": len(bridge_metadata_ids),
    "publisher_per_bridge_entry_count": len(per_entry_ids),
    "publisher_bridge_ids": sorted(publisher_ids),
    "creator_new_bridge_ids": sorted(creator_ids),
    "bridge_metadata_ids": sorted(bridge_metadata_ids),
    "host_creator_state": creator_host.get("host_role_state"),
    "new_creator_state": creator_new.get("self_onboarding_state"),
    "publisher_encryption_key_present": True,
}
with open(root / "dht-summary.json", "w", encoding="utf-8") as handle:
    json.dump(summary, handle, indent=2, sort_keys=True)
    handle.write("\n")
PY
}

smoke_collect_chainid_log_evidence() {
  local chain_id="$1" label="$2"
  shift 2
  local dir="$ARTIFACT_DIR/chainid-evidence/$label"
  mkdir -p "$dir/bridges"

  kubectl -n "$NAMESPACE" logs "$CREATOR_NEW_POD" -c creator-runner --since=30m 2>/dev/null |
    grep -F "$chain_id" >"$dir/creator-new.log" || true
  kubectl -n "$NAMESPACE" logs "$AUTHORITY_POD" -c publisher-authority --since=30m 2>/dev/null |
    grep -F "$chain_id" >"$dir/publisher-authority.log" || true
  kubectl -n "$NAMESPACE" logs "$RECEIVER_POD" -c publisher-receiver --since=30m 2>/dev/null |
    grep -F "$chain_id" >"$dir/publisher-receiver.log" || true

  local bridge_id safe
  for bridge_id in "$@"; do
    [[ -n "$bridge_id" ]] || continue
    safe="$(printf '%s' "$bridge_id" | tr -c 'A-Za-z0-9_.-' '_')"
    kubectl -n "$NAMESPACE" logs "$bridge_id" -c exit-bridge --since=30m 2>/dev/null |
      grep -F "$chain_id" >"$dir/bridges/${safe}.log" || true
  done

  python3 - "$dir" "$chain_id" "$@" <<'PY'
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
chain_id = sys.argv[2]
bridges = [value for value in sys.argv[3:] if value]

def line_count(path):
    if not path.exists():
        return 0
    return sum(1 for line in path.read_text(errors="ignore").splitlines() if chain_id in line)

creator = line_count(root / "creator-new.log")
authority = line_count(root / "publisher-authority.log")
receiver = line_count(root / "publisher-receiver.log")
bridge_counts = {}
for bridge in bridges:
    safe = "".join(ch if ch.isalnum() or ch in "_.-" else "_" for ch in bridge)
    bridge_counts[bridge] = line_count(root / "bridges" / f"{safe}.log")
missing = []
if creator == 0:
    missing.append("creator-new")
if authority + receiver == 0:
    missing.append("publisher")
for bridge, count in bridge_counts.items():
    if count == 0:
        missing.append(bridge)
if missing:
    raise SystemExit(json.dumps({"failure": "missing_chainid_log_evidence", "chain_id": chain_id, "missing": missing}, sort_keys=True))
summary = {
    "chain_id": chain_id,
    "creator_new_log_lines": creator,
    "publisher_authority_log_lines": authority,
    "publisher_receiver_log_lines": receiver,
    "bridge_log_lines": bridge_counts,
}
with open(root / "chainid-summary.json", "w", encoding="utf-8") as handle:
    json.dump(summary, handle, indent=2, sort_keys=True)
    handle.write("\n")
PY
}

smoke_json_result_count() {
  python3 -c 'import json,sys; data=json.load(open(sys.argv[1])); print(len(data.get("data", {}).get("result", data.get("traces", []))))' "$1"
}

smoke_loki_hit_count() {
  python3 -c 'import json,sys; data=json.load(open(sys.argv[1])); print(sum(len(s.get("values", [])) for s in data.get("data", {}).get("result", [])))' "$1"
}

smoke_tempo_hit_count() {
  python3 -c 'import json,sys; data=json.load(open(sys.argv[1])); print(len(data.get("traces", data.get("data", {}).get("traces", []))))' "$1"
}

smoke_tempo_tags() {
  local output="$1"
  curl -fsS "$TEMPO_URL/api/search/tags" >"$output"
}

smoke_wait_loki_hits() {
  local chain_id="$1" output="$2" min_hits="${3:-1}" query
  query="{namespace=\"$NAMESPACE\"} |= \"$chain_id\""
  for _ in {1..24}; do
    smoke_loki_query "$query" "$output"
    if [[ "$(smoke_loki_hit_count "$output")" -ge "$min_hits" ]]; then
      return 0
    fi
    sleep 5
  done
  return 1
}

smoke_wait_tempo_hits() {
  local chain_id="$1" output="$2" min_hits="${3:-1}"
  for _ in {1..24}; do
    smoke_tempo_query_chain "$chain_id" "$output"
    if [[ "$(smoke_tempo_hit_count "$output")" -ge "$min_hits" ]]; then
      return 0
    fi
    sleep 5
  done
  return 1
}

smoke_wait_tempo_tag() {
  local tag="$1" output="$2"
  for _ in {1..24}; do
    smoke_tempo_tags "$output" && smoke_assert_tempo_tag "$tag" "$output" >/dev/null 2>&1 && return 0
    sleep 5
  done
  smoke_assert_tempo_tag "$tag" "$output"
}

smoke_wait_prom_result_count() {
  local query="$1" output="$2" min_count="$3"
  for _ in {1..24}; do
    smoke_prom_query "$query" "$output"
    if [[ "$(smoke_json_result_count "$output")" -ge "$min_count" ]]; then
      return 0
    fi
    sleep 5
  done
  return 1
}

smoke_assert_tempo_tag() {
  local tag="$1" file="$2"
  python3 - "$tag" "$file" <<'PY'
import json
import sys

tag, path = sys.argv[1:3]
data = json.load(open(path))
names = set(data.get("tagNames", []))
for scope in data.get("scopes", []):
    for item in scope.get("tags", []):
        names.add(item.get("name", item) if isinstance(item, dict) else str(item))
if tag not in names:
    raise SystemExit(f"missing Tempo tag {tag!r}")
PY
}

smoke_assert_json_array_contains_all() {
  local json_file="$1" field="$2"
  shift 2
  python3 - "$json_file" "$field" "$@" <<'PY'
import json
import sys

path, field, *expected = sys.argv[1:]
data = json.load(open(path))
values = set(data.get(field, []))
missing = [item for item in expected if item not in values]
if missing:
    raise SystemExit(f"{field} missing expected value(s): {', '.join(missing)}")
PY
}
