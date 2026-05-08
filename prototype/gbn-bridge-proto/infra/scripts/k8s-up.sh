#!/usr/bin/env bash
# Bring up the local Conduit topology in k3d.
set -euo pipefail

CLUSTER_NAME="${VERITAS_K3D_CLUSTER:-veritas}"
NAMESPACE="${VERITAS_K8S_NAMESPACE:-veritas}"
SERVERS="${VERITAS_K3D_SERVERS:-1}"
AGENTS="${VERITAS_K3D_AGENTS:-2}"
RUN_SMOKE="${VERITAS_K8S_RUN_SMOKE:-1}"
RUN_CARGO_PERSISTENCE="${VERITAS_K8S_RUN_CARGO_PERSISTENCE:-1}"
DOCKER_STABILITY_SECONDS="${VERITAS_K8S_DOCKER_STABILITY_SECONDS:-10}"
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
BUILD_ARTIFACT_DIR="${VERITAS_K8S_BUILD_ARTIFACT_DIR:-$ROOT_DIR/target/k8s-builds/$BUILD_VERSION}"
VERSIONED_OVERLAY_DIR="$BUILD_ARTIFACT_DIR/kustomize"

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
  for attempt in {1..90}; do
    if kubectl get nodes >/dev/null 2>&1; then
      kubectl wait --for=condition=Ready node --all --timeout=180s >/dev/null
      return 0
    fi
    sleep 2
  done
  echo "ERROR: Kubernetes API did not become reachable." >&2
  docker ps -a --filter "name=k3d-${CLUSTER_NAME}" >&2 || true
  exit 1
}

import_images() {
  k3d image import \
    "$AUTHORITY_IMAGE" \
    "$RECEIVER_IMAGE" \
    "$BRIDGE_IMAGE" \
    -c "$CLUSTER_NAME"
}

import_images_with_retry() {
  local attempt
  for attempt in {1..3}; do
    wait_for_docker_stable
    ensure_cluster_started
    wait_for_cluster_api
    if import_images; then
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
    wait_for_cluster_api
    if kubectl apply -k "$VERSIONED_OVERLAY_DIR"; then
      return 0
    fi
    echo "kubectl apply failed (attempt $attempt/3); waiting for cluster API before retry..." >&2
    sleep 10
  done
  echo "ERROR: kubectl apply failed after retries." >&2
  exit 1
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
    "$VERSIONED_OVERLAY_DIR/bridge-build-patch.yaml"
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
patches:
  - path: authority-build-patch.yaml
  - path: receiver-build-patch.yaml
  - path: bridge-build-patch.yaml
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
  local deployment="$1" container="$2" image="$3" component="$4" output="$5"
  cat >"$output" <<EOF
apiVersion: apps/v1
kind: Deployment
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

set_deployment_build() {
  local deployment="$1" container="$2" image="$3" component="$4"
  kubectl -n "$NAMESPACE" patch "deployment/$deployment" \
    --type strategic \
    -p "$(build_metadata_patch "$container" "$image" "$component")"
}

set_versioned_deployments() {
  echo "Pinning deployments to build version $BUILD_VERSION..."
  set_deployment_build publisher-authority publisher-authority "$AUTHORITY_IMAGE" publisher-authority
  set_deployment_build publisher-receiver publisher-receiver "$RECEIVER_IMAGE" publisher-receiver
  set_deployment_build exit-bridge exit-bridge "$BRIDGE_IMAGE" exit-bridge
}

prune_old_local_images() {
  [[ "$PRUNE_OLD_IMAGES" == "1" ]] || return 0
  echo "Pruning older local Conduit image tags..."
  local repo image
  for repo in veritas/publisher-authority veritas/publisher-receiver veritas/exit-bridge; do
    while IFS= read -r image; do
      [[ -z "$image" || "$image" == *":<none>" ]] && continue
      case "$image" in
        "$AUTHORITY_IMAGE"|"$RECEIVER_IMAGE"|"$BRIDGE_IMAGE") continue ;;
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
        grep -E '^docker.io/veritas/(publisher-authority|publisher-receiver|exit-bridge):' |
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
VERITAS_VERSIONED_OVERLAY_DIR=$VERSIONED_OVERLAY_DIR
EOF
}

wait_for_current_image_set() {
  echo "Waiting for stale Conduit pods to terminate..."
  local attempt pod_json
  pod_json="$BUILD_ARTIFACT_DIR/pods-version-check.json"
  for attempt in {1..60}; do
    kubectl -n "$NAMESPACE" get pods -l app.kubernetes.io/part-of=veritas-conduit -o json \
      >"$pod_json"
    if python3 - "$pod_json" "$AUTHORITY_IMAGE" "$RECEIVER_IMAGE" "$BRIDGE_IMAGE" "$BUILD_LABEL" "$BUILD_VERSION" <<'PY'
import json
import sys

path, authority_image, receiver_image, bridge_image, build_label, build_version = sys.argv[1:]
expected = {
    "authority": authority_image,
    "receiver": receiver_image,
    "bridge": bridge_image,
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
    if role not in {"authority", "receiver", "bridge"}:
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

for dep in docker k3d kubectl python3; do
  command -v "$dep" >/dev/null 2>&1 || {
    echo "ERROR: '$dep' is required. Run infra/scripts/bootstrap-k8s.sh inside WSL2 first." >&2
    exit 1
  }
done

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
  echo "Creating k3d cluster '$CLUSTER_NAME' (${SERVERS} server, ${AGENTS} agents)..."
  k3d cluster create "$CLUSTER_NAME" \
    --servers "$SERVERS" \
    --agents "$AGENTS" \
    --port "30030:30030@loadbalancer" \
    --wait
else
  echo "Using existing k3d cluster '$CLUSTER_NAME'."
  ensure_cluster_started
fi
wait_for_cluster_api

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

prune_old_local_images

echo "Importing local images into k3d..."
import_images_with_retry

echo "Applying Conduit manifests..."
kubectl_apply_with_retry

set_versioned_deployments

echo "Waiting for Conduit pods..."
kubectl -n "$NAMESPACE" rollout status statefulset/postgres --timeout=180s
kubectl -n "$NAMESPACE" rollout status deployment/publisher-authority --timeout=180s
kubectl -n "$NAMESPACE" rollout status deployment/publisher-receiver --timeout=180s
kubectl -n "$NAMESPACE" rollout status deployment/exit-bridge --timeout=180s
wait_for_current_image_set

kubectl -n "$NAMESPACE" get pods,svc,statefulset,deployment
print_running_versions
prune_old_k3d_images

if [[ "$RUN_SMOKE" == "1" ]]; then
  "$SCRIPT_DIR/k8s-smoke.sh" --send-dummy
else
  echo "Skipping k8s smoke validation because VERITAS_K8S_RUN_SMOKE=$RUN_SMOKE."
fi

if [[ "$RUN_CARGO_PERSISTENCE" == "1" ]]; then
  "$SCRIPT_DIR/k8s-test-publisher-postgres.sh"
else
  echo "Skipping Cargo Postgres validation because VERITAS_K8S_RUN_CARGO_PERSISTENCE=$RUN_CARGO_PERSISTENCE."
fi

echo "Conduit local Kubernetes topology is up."
echo "Cluster:   $CLUSTER_NAME"
echo "Namespace: $NAMESPACE"
echo "Build:     $BUILD_VERSION"
echo "Images:"
echo "  authority: $AUTHORITY_IMAGE"
echo "  receiver:  $RECEIVER_IMAGE"
echo "  bridge:    $BRIDGE_IMAGE"
echo "Build metadata: $BUILD_ARTIFACT_DIR/build.env"
echo "Rendered overlay: $VERSIONED_OVERLAY_DIR"
