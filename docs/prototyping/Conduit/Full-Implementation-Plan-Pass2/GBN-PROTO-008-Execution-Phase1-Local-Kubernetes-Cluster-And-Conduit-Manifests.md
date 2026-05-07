# GBN-PROTO-008 - Execution Phase 1 Detailed Plan: Local Kubernetes Cluster + Conduit Manifests

**Status:** Implemented locally — live k3d bring-up and smoke run pending a WSL2 Docker/k3d session
**Primary Goal:** install `k3d` on the developer's WSL2 Ubuntu workstation, create a
3-node local Kubernetes cluster, build and import the three Conduit V2 container images
locally, and write Kubernetes manifests for the full Conduit topology so a single command
brings up the equivalent of the AWS `gbn-conduit-full-dev` stack on `localhost`.
**Source Plan:** [GBN-PROTO-008 Execution Plan](GBN-PROTO-008-Local-Kubernetes-Test-Infrastructure-Execution-Plan.md)
**AWS Equivalent:** [conduit-full-stack.yaml](../../../prototype/gbn-bridge-proto/infra/cloudformation/conduit-full-stack.yaml)
(this phase produces the local-k8s analog)

---

## 1. Current Repo Findings

| Item | Current Value | Why It Matters |
|---|---|---|
| Container images | three Dockerfiles exist: [Dockerfile.publisher-authority](../../../prototype/gbn-bridge-proto/Dockerfile.publisher-authority), [Dockerfile.publisher-receiver](../../../prototype/gbn-bridge-proto/Dockerfile.publisher-receiver), [Dockerfile.bridge](../../../prototype/gbn-bridge-proto/Dockerfile.bridge) | Phase 1 reuses them as-is; no Dockerfile changes |
| Docker compose for local | [docker-compose.conduit-e2e.yml](../../../prototype/gbn-bridge-proto/docker-compose.conduit-e2e.yml) | useful reference for env vars and service wiring; not used directly |
| Existing infra directory | `prototype/gbn-bridge-proto/infra/` has `cloudformation/` and `scripts/` only | Phase 1 adds `infra/k8s/` |
| AWS stack outputs and inputs | the CFN template parameters at [conduit-full-stack.yaml:12-90](../../../prototype/gbn-bridge-proto/infra/cloudformation/conduit-full-stack.yaml#L12-L90) | template for what env vars the manifests must supply |
| Existing scripts | `deploy-conduit-full.sh`, `smoke-conduit-full.sh`, `teardown-conduit-full.sh` | Phase 1 adds k8s-shaped siblings |
| WSL Docker integration | assumed enabled | required for k3d |

---

## 1.1 GBN-PROTO-007 Validation Gaps Covered By This Phase

Phase 1 creates the local infrastructure needed to finish the GBN-PROTO-007 parity checks
without deploying ECS/Fargate:

| Deferred GBN-PROTO-007 Check | Local-Kubernetes Coverage |
|---|---|
| Deploy a full five-node topology | `k8s-up.sh` creates one authority pod, one receiver pod, three bridge pods, and one Postgres pod |
| Local Postgres availability for persistence/failover tests | `postgres` StatefulSet + PVC + generated dev credentials |
| Host-side Cargo tests that need Postgres | `k8s-test-publisher-postgres.sh` port-forwards the Kubernetes Postgres service and exports `GBN_BRIDGE_POSTGRES_*` plus `GBN_BRIDGE_TEST_POSTGRES_URL` |
| Admin endpoint validation on every node | `k8s-smoke.sh` curls `127.0.0.1:9090/v1/admin/metrics` inside every Conduit pod |
| Bridge registration before end-to-end tests | `k8s-smoke.sh` waits for the authority admin bridge registry to reach the expected bridge count |
| `SendDummy` from authority, receiver, and all bridges | `k8s-smoke.sh --send-dummy` posts to `/v1/admin/send-dummy` from every Conduit pod |
| `chain_id` persistence and trace evidence | `k8s-smoke.sh --send-dummy` verifies the generated `chain_id` appears in authority frames and recent pod logs |

Phase 2 through Phase 4 add the local Prometheus/Loki/Tempo and kubectl operator surfaces
that replace the AWS CloudWatch/ECS operator checks.

---

## 2. Review Summary

| Gap | Why It Matters | Resolution For Phase 1 |
|---|---|---|
| No local Kubernetes runtime installed | cannot run a cluster | install `k3d` and `kubectl`; document the exact apt / curl steps |
| No k8s manifests for Conduit | cannot deploy V2 binaries to a cluster | add `infra/k8s/conduit/` with a complete manifest set |
| Postgres deployment not yet defined for k8s | authority binary requires it | add Postgres `StatefulSet` + Service + PVC + Secret |
| Image tagging convention undecided for local | local images must not collide with ECR-pushed dev images | settle on `veritas/<binary>:dev-<short-sha>` tag with a `:local` floating tag for the bring-up script |
| No bring-up automation | manual `kubectl apply` is error-prone | add `infra/scripts/k8s-up.sh` and `k8s-down.sh` |

---

## 3. Scope Lock

### In Scope

- documented installation steps for `k3d` and `kubectl` on WSL2 Ubuntu (no automation
  beyond a one-shot `bootstrap-k8s.sh` script)
- `infra/k8s/conduit/` manifest tree:
  - `namespace.yaml` — `veritas` namespace
  - `postgres-secret.yaml` — random-generated DB credentials
  - `postgres-statefulset.yaml` + `postgres-service.yaml` + `postgres-pvc.yaml`
  - `authority-deployment.yaml` + `authority-service.yaml`
  - `receiver-deployment.yaml` + `receiver-service.yaml`
  - `bridge-deployment.yaml` (3 replicas) + `bridge-service.yaml`
  - `authority-config.yaml` — non-secret env vars (Cloud Map → ClusterIP DNS)
- new scripts:
  - `infra/scripts/bootstrap-k8s.sh` — installs k3d + kubectl if missing
  - `infra/scripts/k8s-up.sh` — `k3d cluster create` + `docker build` + `k3d image import` + `kubectl apply -k infra/k8s/conduit`
  - `infra/scripts/k8s-smoke.sh` — validates Postgres, admin endpoints, bridge registration, and SendDummy against the local topology
  - `infra/scripts/k8s-test-publisher-postgres.sh` — port-forwards Kubernetes Postgres and runs the Postgres-backed publisher persistence test
  - `infra/scripts/k8s-down.sh` — `k3d cluster delete`
- Kustomize overlay structure (`base/` + `dev/` overlay) so future overlays are easy
- README section in [infra/README-infra.md](../../../prototype/gbn-bridge-proto/infra/README-infra.md)
  documenting the local-k8s flow

### Out Of Scope

- observability stack install (Phase 2 of this plan)
- Prometheus metrics emission code in V2 binaries (Phase 3 of this plan)
- operator script (Phase 4 of this plan)
- Helm chart packaging of Conduit itself (Kustomize is sufficient for now)
- Ingress / cert-manager / external-dns
- NetworkPolicies, PodSecurityStandards, RBAC tightening
- HorizontalPodAutoscaler for bridges
- Modifying the existing CloudFormation template

---

## 4. Preflight Gates

1. WSL2 Ubuntu shell available with `docker version` succeeding.
2. ≥ 8 GB RAM available to the WSL distro (`free -g`).
3. ≥ 20 GB free disk in the WSL filesystem.
4. Outbound HTTPS to `github.com`, `registry-1.docker.io`, `raw.githubusercontent.com`.
5. The three Conduit Dockerfiles still build successfully (smoke build before Phase 1
   begins).
6. `git status` clean on `main` at the GBN-PROTO-008 starting commit.

---

## 5. File-by-File Specification

### 5.1 New file: `prototype/gbn-bridge-proto/infra/scripts/bootstrap-k8s.sh`

```bash
#!/usr/bin/env bash
# Installs k3d + kubectl on WSL2 Ubuntu if not already present. Idempotent.
set -euo pipefail

if ! command -v k3d >/dev/null 2>&1; then
  echo "Installing k3d ..."
  curl -fsSL https://raw.githubusercontent.com/k3d-io/k3d/main/install.sh | bash
fi

if ! command -v kubectl >/dev/null 2>&1; then
  echo "Installing kubectl ..."
  curl -fsSLo kubectl "https://dl.k8s.io/release/$(curl -fsSL https://dl.k8s.io/release/stable.txt)/bin/linux/amd64/kubectl"
  chmod +x kubectl
  sudo mv kubectl /usr/local/bin/kubectl
fi

if ! command -v helm >/dev/null 2>&1; then
  echo "Installing helm ..."
  curl -fsSL https://raw.githubusercontent.com/helm/helm/main/scripts/get-helm-3 | bash
fi

k3d version
kubectl version --client
helm version
echo "Bootstrap complete."
```

### 5.2 New file: `prototype/gbn-bridge-proto/infra/scripts/k8s-up.sh`

```bash
#!/usr/bin/env bash
# Brings up the local Conduit topology end-to-end.
# Idempotent — re-running is safe.
set -euo pipefail

CLUSTER_NAME="${VERITAS_K3D_CLUSTER:-veritas}"
NAMESPACE="${VERITAS_K8S_NAMESPACE:-veritas}"
SERVERS="${VERITAS_K3D_SERVERS:-1}"
AGENTS="${VERITAS_K3D_AGENTS:-2}"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# 1. Ensure k3d cluster exists.
if ! k3d cluster list "$CLUSTER_NAME" >/dev/null 2>&1; then
  echo "Creating k3d cluster '$CLUSTER_NAME' (${SERVERS}s ${AGENTS}a) ..."
  k3d cluster create "$CLUSTER_NAME" \
    --servers "$SERVERS" --agents "$AGENTS" \
    --port "8080:80@loadbalancer" \
    --port "3000:3000@loadbalancer" \
    --wait
fi

# 2. Build and import images.
echo "Building images ..."
docker build -f "$ROOT_DIR/Dockerfile.publisher-authority" \
  -t veritas/publisher-authority:dev "$ROOT_DIR"
docker build -f "$ROOT_DIR/Dockerfile.publisher-receiver" \
  -t veritas/publisher-receiver:dev "$ROOT_DIR"
docker build -f "$ROOT_DIR/Dockerfile.bridge" \
  -t veritas/exit-bridge:dev "$ROOT_DIR"

echo "Importing images into k3d ..."
k3d image import \
  veritas/publisher-authority:dev \
  veritas/publisher-receiver:dev \
  veritas/exit-bridge:dev \
  -c "$CLUSTER_NAME"

# 3. Apply manifests via Kustomize.
echo "Applying Kustomize overlay ..."
kubectl apply -k "$ROOT_DIR/infra/k8s/conduit/overlays/dev"

# 4. Wait for the topology to be ready.
echo "Waiting for pods ..."
kubectl -n "$NAMESPACE" rollout status statefulset/postgres --timeout=120s
kubectl -n "$NAMESPACE" rollout status deployment/publisher-authority --timeout=120s
kubectl -n "$NAMESPACE" rollout status deployment/publisher-receiver  --timeout=120s
kubectl -n "$NAMESPACE" rollout status deployment/exit-bridge          --timeout=120s

kubectl -n "$NAMESPACE" get pods,svc,statefulset,deployment

echo "Conduit is up. Cluster: $CLUSTER_NAME, Namespace: $NAMESPACE"
echo "Run: bash $ROOT_DIR/infra/scripts/k8s-control-interactive.sh"
```

### 5.3 New file: `prototype/gbn-bridge-proto/infra/scripts/k8s-down.sh`

```bash
#!/usr/bin/env bash
set -euo pipefail
CLUSTER_NAME="${VERITAS_K3D_CLUSTER:-veritas}"

if k3d cluster list "$CLUSTER_NAME" >/dev/null 2>&1; then
  read -r -p "Delete k3d cluster '$CLUSTER_NAME'? [y/N]: " confirm
  if [[ "${confirm,,}" == "y" ]]; then
    k3d cluster delete "$CLUSTER_NAME"
    echo "Cluster deleted."
  fi
else
  echo "No cluster named '$CLUSTER_NAME' found."
fi
```

### 5.4 New manifest tree: `prototype/gbn-bridge-proto/infra/k8s/conduit/`

Use Kustomize with a `base` + `overlays/dev` layout so future overlays (e.g., `overlays/ci`)
slot in cleanly.

#### `base/kustomization.yaml`

```yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
namespace: veritas
resources:
  - namespace.yaml
  - postgres-secret.yaml
  - postgres-pvc.yaml
  - postgres-statefulset.yaml
  - postgres-service.yaml
  - authority-config.yaml
  - authority-deployment.yaml
  - authority-service.yaml
  - receiver-deployment.yaml
  - receiver-service.yaml
  - bridge-deployment.yaml
  - bridge-service.yaml
```

#### `base/namespace.yaml`

```yaml
apiVersion: v1
kind: Namespace
metadata:
  name: veritas
  labels:
    app.kubernetes.io/part-of: veritas-conduit
```

#### `base/postgres-secret.yaml`

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: postgres-credentials
type: Opaque
stringData:
  POSTGRES_DB: conduit
  POSTGRES_USER: conduit
  POSTGRES_PASSWORD: dev-only-replace-via-overlay
```

The `dev` overlay generates a random password at apply time via a Kustomize secretGenerator
or expects the operator to override. Document this in README.

#### `base/postgres-pvc.yaml`

```yaml
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: postgres-data
spec:
  accessModes: [ReadWriteOnce]
  resources:
    requests:
      storage: 1Gi
  storageClassName: local-path
```

#### `base/postgres-statefulset.yaml`

```yaml
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: postgres
spec:
  serviceName: postgres
  replicas: 1
  selector:
    matchLabels: { app: postgres }
  template:
    metadata:
      labels: { app: postgres }
    spec:
      containers:
        - name: postgres
          image: postgres:16-alpine
          ports:
            - containerPort: 5432
          envFrom:
            - secretRef: { name: postgres-credentials }
          volumeMounts:
            - name: data
              mountPath: /var/lib/postgresql/data
              subPath: pgdata
          readinessProbe:
            exec:
              command: ["pg_isready", "-U", "conduit"]
            periodSeconds: 5
      volumes:
        - name: data
          persistentVolumeClaim:
            claimName: postgres-data
```

#### `base/postgres-service.yaml`

```yaml
apiVersion: v1
kind: Service
metadata:
  name: postgres
spec:
  selector: { app: postgres }
  ports:
    - port: 5432
      targetPort: 5432
  clusterIP: None    # headless for StatefulSet
```

#### `base/authority-config.yaml`

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: conduit-config
data:
  GBN_BRIDGE_AUTHORITY_URL: "http://publisher-authority.veritas.svc.cluster.local:8080"
  GBN_BRIDGE_RECEIVER_URL:  "http://publisher-receiver.veritas.svc.cluster.local:8081"
  GBN_BRIDGE_PUNCH_PORT: "443"
  GBN_BRIDGE_BATCH_WINDOW_MS: "500"
  GBN_BRIDGE_STACK_ENV: "dev-local"
  RUST_LOG: "info"
```

#### `base/authority-deployment.yaml`

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: publisher-authority
spec:
  replicas: 1
  selector:
    matchLabels: { app: publisher-authority }
  template:
    metadata:
      labels:
        app: publisher-authority
        veritas-role: authority
      annotations:
        prometheus.io/scrape: "true"
        prometheus.io/path: "/metrics"
        prometheus.io/port: "8080"
    spec:
      containers:
        - name: publisher-authority
          image: veritas/publisher-authority:dev
          imagePullPolicy: IfNotPresent
          ports:
            - { name: http,  containerPort: 8080 }
            - { name: admin, containerPort: 9090 }
          envFrom:
            - configMapRef: { name: conduit-config }
            - secretRef:    { name: postgres-credentials }
          env:
            - name: GBN_BRIDGE_POSTGRES_HOST
              value: postgres.veritas.svc.cluster.local
            - name: GBN_BRIDGE_POSTGRES_PORT
              value: "5432"
          readinessProbe:
            httpGet: { path: /readyz, port: 8080 }
            initialDelaySeconds: 5
          livenessProbe:
            httpGet: { path: /healthz, port: 8080 }
            initialDelaySeconds: 10
```

#### `base/authority-service.yaml`

```yaml
apiVersion: v1
kind: Service
metadata:
  name: publisher-authority
  labels:
    veritas-role: authority
spec:
  selector: { app: publisher-authority }
  ports:
    - { name: http,  port: 8080, targetPort: http }
```

(Admin port 9090 deliberately omitted — operator reaches it via `kubectl exec` per the
GBN-PROTO-007 admin isolation rule.)

#### `base/receiver-deployment.yaml` and `base/receiver-service.yaml`

Same shape as authority, with `app: publisher-receiver`, image
`veritas/publisher-receiver:dev`, public port 8081, role `receiver`. Only difference is
no Postgres direct connection (receiver typically calls authority or its own subset of
storage; confirm in implementation).

#### `base/bridge-deployment.yaml`

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: exit-bridge
spec:
  replicas: 3   # mirrors AWS DesiredBridgeCount default
  selector:
    matchLabels: { app: exit-bridge }
  template:
    metadata:
      labels:
        app: exit-bridge
        veritas-role: bridge
      annotations:
        prometheus.io/scrape: "true"
        prometheus.io/path: "/metrics"
        prometheus.io/port: "9090"   # bridge exposes /metrics on the admin port
    spec:
      containers:
        - name: exit-bridge
          image: veritas/exit-bridge:dev
          imagePullPolicy: IfNotPresent
          ports:
            - { name: udp-punch, containerPort: 4443, protocol: UDP }
            - { name: admin,     containerPort: 9090 }
          envFrom:
            - configMapRef: { name: conduit-config }
          env:
            - name: GBN_BRIDGE_NODE_ID
              valueFrom:
                fieldRef: { fieldPath: metadata.name }
            - name: GBN_BRIDGE_INGRESS_HOST
              valueFrom:
                fieldRef: { fieldPath: status.podIP }
```

#### `base/bridge-service.yaml`

```yaml
apiVersion: v1
kind: Service
metadata:
  name: exit-bridge
spec:
  selector: { app: exit-bridge }
  ports:
    - { name: udp-punch, port: 4443, targetPort: udp-punch, protocol: UDP }
  type: ClusterIP
```

#### `overlays/dev/kustomization.yaml`

```yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
namespace: veritas
resources:
  - ../../base
secretGenerator:
  - name: postgres-credentials
    behavior: replace
    literals:
      - POSTGRES_DB=conduit
      - POSTGRES_USER=conduit
    files:
      - POSTGRES_PASSWORD=password.txt    # operator generates locally; gitignored
```

A `.gitignore` entry for `infra/k8s/conduit/overlays/dev/password.txt` is required.

### 5.5 Modify: `prototype/gbn-bridge-proto/infra/README-infra.md`

Add a new section "Local Kubernetes Test Environment" with:
- prerequisites
- bootstrap command
- bring-up command
- pod inspection examples
- tear-down
- pointer to the operator script (Phase 4)

---

## 6. Module And Asset Ownership Locked In Phase 1

| Asset | Responsibility |
|---|---|
| `infra/k8s/conduit/base/` | declarative topology — namespace, secret, configmap, deployments, services, statefulset |
| `infra/k8s/conduit/overlays/dev/` | dev-specific overrides: random password, possibly resource caps |
| `infra/scripts/bootstrap-k8s.sh` | install missing toolchain (k3d / kubectl / helm) |
| `infra/scripts/k8s-up.sh` | end-to-end bring-up |
| `infra/scripts/k8s-smoke.sh` | local validation for Postgres, admin endpoints, bridge registration, SendDummy, and `chain_id` evidence |
| `infra/scripts/k8s-test-publisher-postgres.sh` | host-side Cargo persistence validation against Kubernetes Postgres |
| `infra/scripts/k8s-down.sh` | tear-down |
| `infra/k8s/observability/` | (created in Phase 2 of this plan) — observability stack manifests / values |

---

## 7. Implementation Notes

Implementation adjustments made while landing Phase 1:

1. Bridge pods use Kubernetes Downward API values:
   - `GBN_BRIDGE_NODE_ID` from `metadata.name`
   - `GBN_BRIDGE_INGRESS_HOST` from `status.podIP`

   This replaces the draft's ECS `auto` metadata path, which is not available in k3d.
2. Bridge UDP uses container port `4443` instead of privileged port `443` so the
   non-root `exit-bridge` image can bind reliably in Kubernetes.
3. The local ConfigMap sets `GBN_BRIDGE_CLOUDWATCH_ENABLED=false`. The binaries now honor
   that flag, so local k3d execution does not touch the AWS credential chain while the
   Prometheus path is still pending Phase 3.
4. `k8s-up.sh` generates `overlays/dev/password.txt` if it is missing, applies the dev
   Kustomize overlay, waits for rollouts, and then runs `k8s-smoke.sh --send-dummy`.
5. `k8s-smoke.sh` exists specifically to close the GBN-PROTO-007 validation gap locally:
   it verifies the full topology, local Postgres, localhost admin endpoints, bridge
   registration, `SendDummy` from all Conduit pods, authority frame persistence, and recent
   pod-log `chain_id` evidence.
6. `k8s-test-publisher-postgres.sh` addresses the existing host-side
   `persistence_flow` `ConnectionRefused` blocker by port-forwarding the Kubernetes
   Postgres service, exporting the matching `GBN_BRIDGE_POSTGRES_*` variables plus
   `GBN_BRIDGE_TEST_POSTGRES_URL`, and running
   `cargo test -p gbn-bridge-publisher --test persistence_flow`.

---

## 8. Validation

Completed static/local validation in the current Windows-hosted shell:

1. `bash -n prototype/gbn-bridge-proto/infra/scripts/bootstrap-k8s.sh prototype/gbn-bridge-proto/infra/scripts/k8s-up.sh prototype/gbn-bridge-proto/infra/scripts/k8s-down.sh prototype/gbn-bridge-proto/infra/scripts/k8s-smoke.sh prototype/gbn-bridge-proto/infra/scripts/k8s-test-publisher-postgres.sh`
   passed.
2. PyYAML parsed every manifest under `prototype/gbn-bridge-proto/infra/k8s/conduit`.
3. `cargo fmt --all --check` passed in `prototype/gbn-bridge-proto`.
4. `cargo check --workspace` passed in `prototype/gbn-bridge-proto`.
5. `cargo test -p gbn-bridge-publisher metrics_emitter` passed.
6. `git diff --check` passed with only Windows LF/CRLF warnings.
7. V1 protected-path diff was clean.

Deferred live WSL2 validation because this PowerShell environment does not have `docker`,
`k3d`, or `kubectl` on PATH:

`cargo test -p gbn-bridge-publisher` was also attempted in this shell and still reaches
the known `persistence_flow` `ConnectionRefused` failure because no local Postgres is
listening on the host. The Kubernetes-backed replacement for that check is
`k8s-test-publisher-postgres.sh`, which requires the live k3d cluster and port-forward.

1. Fresh WSL2 shell. Run `bash prototype/gbn-bridge-proto/infra/scripts/bootstrap-k8s.sh`.
   `k3d`, `kubectl`, and `helm` are installed if missing; idempotent on rerun.
2. Run `bash prototype/gbn-bridge-proto/infra/scripts/k8s-up.sh`. Within ~5 minutes:
   - `kubectl -n veritas get pods` shows 6 pods Running (1 postgres, 1 authority, 1 receiver, 3 bridges).
   - `kubectl -n veritas logs deployment/publisher-authority --tail=50` shows the service started cleanly.
3. `kubectl -n veritas exec deploy/publisher-authority -- curl -sS http://127.0.0.1:9090/v1/admin/metrics`
   returns JSON (Phase 1 of GBN-PROTO-007's admin endpoint, which must already be in place
   for this test to pass — note dependency).
4. `bash prototype/gbn-bridge-proto/infra/scripts/k8s-smoke.sh --send-dummy`
   validates Postgres, admin endpoints, bridge registration, SendDummy from all Conduit
   pods, persisted frames, and recent `chain_id` logs.
5. `bash prototype/gbn-bridge-proto/infra/scripts/k8s-test-publisher-postgres.sh`
   validates the host-side publisher persistence test against Kubernetes Postgres.
6. Run `bash prototype/gbn-bridge-proto/infra/scripts/k8s-down.sh`, confirm with `y`,
   cluster is gone, `docker ps` shows no `k3d-veritas-*` containers.
7. Update this document with the live k3d output once the WSL2 run completes.

---

## 9. Open Questions Carried Into Implementation

1. **Receiver Postgres dependency** — receiver does not need direct Postgres connectivity;
   its deployment does not include the Postgres secret.
2. **Bridge UDP exposure** — `Service` type `ClusterIP` is used here. If the operator
   wants to send UDP packets from outside the cluster (e.g., from the WSL host) into a
   bridge, type `NodePort` plus a k3d port mapping is needed. Defer until Phase 4
   exercises this.
3. **`local-path` storage class behavior under WSL2** — confirm that `local-path` PVCs
   survive a `k3d cluster stop`/`k3d cluster start` cycle. If not, document the limit.
4. **Image versioning beyond `:dev`** — for reproducibility across team workstations,
   adopt `:dev-<git-short-sha>` tags. Defer if the team is one developer for now.
5. **Kustomize secretGenerator vs raw Secret** — the base keeps a placeholder Secret so
   the base is readable, and the dev overlay replaces it with a generated local password.
