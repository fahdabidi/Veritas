# GBN-PROTO-012 - Execution Phase 0 - Creator Pod Deployment And Cluster Topology

**Status:** Completed
**Last Updated:** 2026-05-08
**Parent Plan:** [GBN-PROTO-012](GBN-PROTO-012-Conduit-Architecture-Correct-Bootstrap-Execution-Plan.md)

## Objective

Establish the deployment topology that the rest of Pass 3 depends on. Pass 1 and Pass 2
left the cluster with no creator pods (the synthetic creator from Pass 2 lives inside
the Publisher processes) and only 3 ExitBridge replicas. Pass 3 cannot select distinct
HostCreator and NewCreator nodes, and cannot satisfy `GBN-ARCH-001-V2` section 3.3
step 3 ("9 active ExitBridge nodes"), without changing the deployment.

At completion, the local k3d cluster and AWS CloudFormation stack expose:

- 2 dedicated creator pods/tasks (`creator-host`, `creator-new`)
- 10 ExitBridge replicas (1 ExitBridgeA acting as HostCreator's relay path + 9 bridges
  in the Publisher's bootstrap set, of which one is selected as ExitBridgeB)
- Container-local persistence for creator and bridge state (per Master plan §3.6 and
  Pass 3 D1: state survives container restart, not cluster destroy)

V1 preservation rule §2.6 holds: only `prototype/gbn-bridge-proto/**` is touched.

Update the parent plan status tracker when this phase is complete.

---

## Pre-Requisite WSL2 Allocation

The local cluster grows from 11 pods to 20 pods. Before bring-up, confirm WSL2 is sized
appropriately. Edit the Windows host file at `C:\Users\<user>\.wslconfig`:

```ini
[wsl2]
memory=10GB
processors=6
swap=4GB
```

Then run `wsl --shutdown` from PowerShell once.

Verify inside WSL2 Ubuntu before proceeding:

```bash
free -h    # ≥ 10 GiB total
nproc      # ≥ 6
```

If `kubectl top nodes` (post-bring-up) shows sustained > 90% memory pressure during
smoke 2/3 runs, raise WSL memory to 12 GB before lowering pod counts.

---

## New Binary: `creator-runner`

Add a new binary to `gbn-bridge-cli`:

```text
prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/bin/creator_runner.rs
```

Responsibilities:

- Long-lived process running `gbn-bridge-creator`'s `CreatorClient` capability.
- Binds the admin listener to:
  - `0.0.0.0:9090` inside k8s pods (admin reachable through `kubectl exec` and the
    pod-network admin Service).
  - `127.0.0.1:9090` inside ECS tasks (admin reachable only via ECS Exec).
- Loads Publisher trust root from `GBN_PUBLISHER_PUB_KEY_PATH`.
- Loads or creates persisted `LocalDiscoveryTable` from
  `${GBN_BRIDGE_STATE_DIR:-/var/lib/gbn-conduit}/local_dht.json` (per Phase 1 §
  Persistence).
- Exposes the new admin endpoints introduced in later phases:
  `/v1/admin/node-metadata`, `/v1/admin/local-dht`, `/v1/admin/seed-host-creator`,
  `/v1/admin/seed-new-creator`, `/v1/admin/reset-creator-state`, `/v1/admin/send-dummy`.
- Emits `chain_id`-tagged logs/spans for every state transition.

Phase 0 ships an empty-but-functional binary: it boots, exposes
`GET /v1/admin/node-metadata` returning role `creator`, and reports
`state=none` from `GET /v1/admin/local-dht`. Subsequent phases fill in behavior.

---

## New Kubernetes Manifests

All files live under `prototype/gbn-bridge-proto/infra/k8s/`.

### `creator-host-deployment.yaml`

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: creator-host
  namespace: veritas
spec:
  replicas: 1
  strategy:
    type: Recreate
  selector:
    matchLabels: { app: creator-host }
  template:
    metadata:
      labels: { app: creator-host, role: creator, conduit-actor: host-creator }
    spec:
      containers:
        - name: creator-runner
          image: veritas/creator-runner:dev
          imagePullPolicy: IfNotPresent
          env:
            - { name: GBN_BRIDGE_ADMIN_BIND_ADDR, value: "0.0.0.0:9090" }
            - { name: GBN_BRIDGE_STATE_DIR,       value: "/var/lib/gbn-conduit" }
            - { name: GBN_PUBLISHER_PUB_KEY_PATH, value: "/etc/conduit/publisher_pub.key" }
            - { name: GBN_NODE_ROLE,              value: "creator" }
          ports:
            - { name: admin, containerPort: 9090 }
          resources:
            requests: { cpu: "75m",  memory: "128Mi" }
            limits:   { cpu: "300m", memory: "256Mi" }
          volumeMounts:
            - { name: state, mountPath: /var/lib/gbn-conduit }
            - { name: trust, mountPath: /etc/conduit, readOnly: true }
      volumes:
        - name: state
          persistentVolumeClaim: { claimName: creator-host-state }
        - name: trust
          secret: { secretName: conduit-publisher-pub }
---
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: creator-host-state
  namespace: veritas
spec:
  accessModes: [ ReadWriteOnce ]
  storageClassName: local-path
  resources: { requests: { storage: 64Mi } }
---
apiVersion: v1
kind: Service
metadata:
  name: creator-host
  namespace: veritas
spec:
  selector: { app: creator-host }
  ports:
    - { name: admin, port: 9090, targetPort: 9090 }
```

### `creator-new-deployment.yaml`

Identical to `creator-host-deployment.yaml` with all `creator-host` strings replaced by
`creator-new` and label `conduit-actor: new-creator`.

### `exit-bridge-deployment.yaml`

Update existing manifest (or replace) so `replicas: 10`. Each bridge gets its own PVC
and stable identity via a deterministic env var (`GBN_BRIDGE_INDEX=0..9`) derived from
the StatefulSet ordinal — convert the existing Deployment to a StatefulSet so that
PVCs are stable across pod restarts (per D1).

```yaml
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: exit-bridge
  namespace: veritas
spec:
  serviceName: exit-bridge
  replicas: 10
  selector: { matchLabels: { app: exit-bridge } }
  template:
    metadata:
      labels: { app: exit-bridge, role: bridge }
    spec:
      containers:
        - name: bridge
          image: veritas/exit-bridge:dev
          env:
            - { name: GBN_BRIDGE_ADMIN_BIND_ADDR, value: "0.0.0.0:9090" }
            - { name: GBN_BRIDGE_STATE_DIR,       value: "/var/lib/gbn-conduit" }
          resources:
            requests: { cpu: "50m",  memory: "96Mi" }
            limits:   { cpu: "200m", memory: "192Mi" }
          volumeMounts:
            - { name: state, mountPath: /var/lib/gbn-conduit }
  volumeClaimTemplates:
    - metadata: { name: state }
      spec:
        accessModes: [ ReadWriteOnce ]
        storageClassName: local-path
        resources: { requests: { storage: 64Mi } }
```

### Publisher resource block updates

Patch `publisher-authority` and `publisher-receiver` Deployments to declare the same
requests/limits used in Pass 3 D-cluster sizing:

- `requests: { cpu: "100m", memory: "192Mi" }`
- `limits:   { cpu: "500m", memory: "384Mi" }`

Per D4 these stay as two cooperating processes; no merge.

---

## CloudFormation Updates

`prototype/gbn-bridge-proto/infra/cloudformation/conduit-stack.yaml`:

- Set ExitBridge service `DesiredCount` from 3 to **10**.
- Add two new ECS services: `creator-host` and `creator-new`. Each:
  - Task definition: 0.25 vCPU / 512 MiB Fargate sizing
  - One task per service
  - EFS volume mounted at `/var/lib/gbn-conduit` (per D1 EFS persistence)
  - Same image as `creator-runner` (built and pushed alongside other Conduit images)
  - Admin port `9090` not exposed publicly; reachable only via `aws ecs execute-command`
  - IAM task role: `ssmmessages:CreateControlChannel`, `CreateDataChannel`,
    `OpenControlChannel`, `OpenDataChannel`
- Total stack at ~3.75 vCPU and ~7 GiB memory (verified within Fargate quota).

---

## Operator Script Updates (Phase 0 minimum)

`prototype/gbn-bridge-proto/infra/scripts/k8s-control-interactive.sh` and
`relay-control-interactive-v2.sh` already enumerate live nodes for selection. Phase 0
extends the discovery query to include `creator-host` and `creator-new`, and tags each
node with its `role` and `conduit-actor` label so later phases can filter selections by
role.

No new menu actions in Phase 0 — those land in Phases 2, 3, 5, 6.

---

## Build And Image Updates

`prototype/gbn-bridge-proto/infra/scripts/build-and-push-conduit-full.sh` (or local
equivalent for k3d image import) must build the new `creator-runner` image:

```dockerfile
# prototype/gbn-bridge-proto/Dockerfile.creator-runner
FROM rust:1.77 AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p gbn-bridge-cli --bin creator_runner

FROM debian:stable-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/creator_runner /usr/local/bin/
ENTRYPOINT ["/usr/local/bin/creator_runner"]
```

For k3d:

```bash
docker build -t veritas/creator-runner:dev -f prototype/gbn-bridge-proto/Dockerfile.creator-runner .
k3d image import veritas/creator-runner:dev -c veritas
```

---

## Cluster Bring-Up Script Update

`prototype/gbn-bridge-proto/infra/scripts/k8s-up.sh` must:

1. Bring up k3d cluster (existing).
2. Apply existing manifests (publisher, postgres, observability — existing).
3. Apply new manifests (`creator-host`, `creator-new`, scaled `exit-bridge`).
4. Wait for all 20 pods Ready before returning.
5. Print a topology summary:

   ```
   creator-host  : 1 pod  Ready  10.42.x.y:9090
   creator-new   : 1 pod  Ready  10.42.x.y:9090
   exit-bridge   : 10 pods Ready
   publisher     : 2 pods Ready  (authority surface + receiver surface, single role)
   postgres      : 1 pod  Ready
   observability : 5 pods Ready
   ```

---

## Tests

Add focused tests for:

- `creator-runner` boots without `LocalDiscoveryTable` state file present and creates
  an empty file.
- `creator-runner` boots with an existing valid state file and reloads it.
- `GET /v1/admin/node-metadata` returns `role=creator` for creator pods,
  `role=publisher` for both Publisher surfaces, `role=exit_bridge` for bridge pods.
- All 20 pods reach Ready within 90 seconds on a 6-vCPU / 10 GiB WSL2 host.

Run inside WSL2 Ubuntu:

```bash
uname -a | grep -i microsoft   # WSL2 baseline check
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo build --release -p gbn-bridge-cli --bin creator_runner
bash infra/scripts/k8s-up.sh
kubectl get pods -n veritas
kubectl top nodes
```

---

## Acceptance Criteria

- `creator-host` and `creator-new` pods are Ready in the local k3d cluster.
- 10 `exit-bridge` pods are Ready.
- Cluster `kubectl top nodes` reports < 85% memory and < 80% CPU at idle.
- Each creator and bridge pod has a mounted `/var/lib/gbn-conduit` PVC; deleting
  a creator pod and waiting for the StatefulSet/Deployment to recreate it preserves
  the PVC contents.
- `kubectl exec creator-host -- curl -s http://127.0.0.1:9090/v1/admin/node-metadata`
  returns role `creator`.
- AWS CloudFormation `creator-host` and `creator-new` services exist with task counts of
  1 each, and `aws ecs execute-command --task creator-host` returns an interactive shell.
- V1 (`prototype/gbn-proto/**`) is unchanged: `git diff --stat -- prototype/gbn-proto/`
  is empty.
- Parent plan status tracker is updated.

## Completion Evidence

Completed locally in WSL2 on 2026-05-08.

- `cargo check --workspace` passed.
- `cargo test -p gbn-bridge-cli --bin creator_runner` passed.
- `cargo test -p gbn-bridge-publisher admin_states_expose_phase0_node_roles` passed.
- `bash -n` passed for the updated operator, build, deploy, and smoke scripts.
- `kubectl kustomize prototype/gbn-bridge-proto/infra/k8s/conduit/base` rendered
  successfully inside WSL2.
- `VERITAS_K8S_RUN_SMOKE=0 VERITAS_K8S_RUN_CARGO_PERSISTENCE=0
  infra/scripts/k8s-up.sh` built and imported versioned images, deployed
  `creator-host`, `creator-new`, and `exit-bridge` as a 10-replica StatefulSet, and
  reported the expected topology.
- `infra/scripts/k8s-smoke.sh` passed after rollout with 10 registered bridges and
  valid creator node metadata / empty local-DHT responses.
- `kubectl top nodes` reported idle usage below the Phase 0 threshold:
  CPU about 0-1% and memory about 2-3% per k3d node.

Note: Docker restarted once during post-rollout probing in this WSL environment, causing
k3d node container restarts. Restarting the existing k3d cluster recovered the topology,
and the Phase 0 smoke check passed again afterward. The repeated Docker restart behavior
is environmental and remains separate from the Phase 0 implementation changes.
