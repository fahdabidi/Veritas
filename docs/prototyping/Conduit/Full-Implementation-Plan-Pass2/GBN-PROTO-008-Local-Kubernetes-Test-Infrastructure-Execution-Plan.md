# GBN-PROTO-008 - Local Kubernetes Test Infrastructure - Execution Plan

**Document ID:** GBN-PROTO-008
**Status:** Active - Phase 1 implemented locally; Phases 2 through 4 pending
**Last Updated:** 2026-05-07
**Related Docs:**
[GBN-PROTO-007 V2-V1 Parity Execution Plan](GBN-PROTO-007-Conduit-V2-V1-Parity-Execution-Plan.md),
[GBN-PROTO-006 Execution Plan](../Full-Implementation-Plan/GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md)

This Pass 2 sibling plan stands up a **local-only Kubernetes test environment** for Conduit
V2 development on a WSL2 Ubuntu workstation, removing the need to deploy to AWS for
operator-tooling iteration. AWS ECS / Fargate / CloudWatch were costing ~$5–40 per day per
running stack during prototype iteration; local Kubernetes costs $0 and reaches a working
5-pod topology in under 5 minutes.

GBN-PROTO-007 (the AWS parity track) stays valid. Crate-level work (admin endpoints,
creator library) is shared between the AWS and local-k8s tracks. Only the deployment-shape
phases diverge — Phase 3 (metrics emission) and Phase 5 (operator script) of GBN-PROTO-007
each get a sibling phase in GBN-PROTO-008 that uses Prometheus + `kubectl` instead of
CloudWatch + `aws ecs`.

## Status Trackers

- `[ ]` Pending
- `[/]` In Progress
- `[x]` Completed

| Phase | Title | Status |
|---|---|---|
| 1 | Local Kubernetes Cluster + Conduit Manifests | `[x]` |
| 2 | Observability Stack (Prometheus + Grafana + Loki + Tempo) | `[ ]` |
| 3 | Prometheus Metrics Emission (variant of GBN-PROTO-007 Phase 3) | `[ ]` |
| 4 | Local Kubernetes Operator Script (variant of GBN-PROTO-007 Phase 5) | `[ ]` |

---

## 1. Why Pivot to Local Kubernetes

GBN-PROTO-006 and GBN-PROTO-007 target AWS as the deployment surface (ECS Fargate +
CloudFormation + CloudWatch). That surface is correct for production but wrong for the
phase of work we are in:

| Concern | AWS surface today | Local k8s surface |
|---|---|---|
| Per-day cost of one running stack | ~$3–5 (one stack); ~$28 (one scale stack) | $0 |
| Time to first running cluster | ~10 min stack deploy | ~2 min `k3d cluster create` |
| Inner-loop iteration (code change → live in cluster) | ~5–8 min build/push/deploy | ~30 s build + `kubectl rollout restart` |
| Risk of orphan-stack cost overruns | Yes — proven in April 2026, see GBN-PROTO-007 cost diagnosis | None |
| Required network reachability | VPC, security groups, ECS exec | localhost / pod network |
| Observability stack | CloudWatch Metrics + Logs (paid) | Prometheus + Grafana + Loki + Tempo (free) |

The crate-level Conduit V2 implementation is identical between the two; only deployment
manifests, the operator script, and the metrics emission backend differ.

## 2. Deployment Surface Decisions

### 2.1 Local Kubernetes Runtime — k3d

User-confirmed: **k3d** (k3s in Docker). Selected over `kind` and `minikube` because:
- WSL2 + Docker Desktop is the user's existing environment.
- k3d's k3s base is the smallest production-realistic distribution (~50 MB memory per node).
- Multi-node clusters created in one command for testing scheduling behavior.
- Built-in load balancer maps cluster ports onto `localhost`.

### 2.2 Observability Stack — Full (Prometheus + Grafana + Loki + Promtail + Tempo)

User-confirmed: **full stack including distributed tracing**. Components:
- **Prometheus** — scrapes each pod's `/metrics` endpoint every 15 seconds.
- **Grafana** — dashboards and explore UI; pre-provisioned datasources for Prometheus,
  Loki, and Tempo.
- **Loki** — log aggregation (replaces CloudWatch Logs).
- **Promtail** (or Grafana Alloy) — log shipper running as a DaemonSet; tails container
  logs and pushes to Loki.
- **Tempo** — distributed tracing backend; Conduit V2 services emit spans tagged with
  `chain_id` so the existing trace propagation work is observable end-to-end.

Selected as a single track (rather than incremental: metrics-only first, then logs, then
traces) because it makes the GBN-PROTO-007 Phase 4 SendDummy correlation story
trivial — the operator types a `chain_id` into Grafana's Tempo search box and sees the
full creator → bridge → receiver call graph.

### 2.3 Track Independence Decision

GBN-PROTO-008 does **not** retire GBN-PROTO-007. The two tracks share crates and admin
endpoints. The split:

| Concern | GBN-PROTO-007 (AWS) | GBN-PROTO-008 (local k8s) |
|---|---|---|
| Phase 1 (admin endpoints) | shared crate work; identical | identical |
| Phase 2 (command injection) | shared crate work; identical | identical |
| Phase 3 (metrics) | CloudWatch emission | **Prometheus `/metrics` endpoint** |
| Phase 4 (creator lib) | shared crate work; identical | identical |
| Phase 5 (operator script) | `aws ecs execute-command` | **`kubectl exec`** in a separate script |
| Deployment manifests | `infra/cloudformation/` | new `infra/k8s/` |

A single Rust binary built from `gbn-bridge-cli` runs unchanged on both ECS and k3d. The
difference is which deployment file the operator chooses and which observability backend
they read from.

---

## 3. Execution Rules

### 3.1 Workstation Requirements

The local-k8s track assumes a developer workstation with:
- WSL2 Ubuntu (≥ 22.04 recommended).
- Docker Desktop with WSL2 integration enabled, **or** Docker Engine installed natively
  inside the WSL distro.
- ≥ 8 GB RAM available to WSL (k3d cluster + observability stack uses ~3 GB).
- ≥ 20 GB free disk for image layers + Loki / Tempo retention.
- Outbound HTTPS to:
  - Docker Hub (`registry-1.docker.io`)
  - k3d / k3s release artifacts (`github.com`, `releases.k3s.io`)
  - Helm chart repos (`prometheus-community.github.io`, `grafana.github.io`)
  - Crates registry for Rust deps if rebuilding locally.

If any of these are missing, the relevant Phase 1 setup step must stop and prompt the
operator.

### 3.2 Image Build Rule

Container images for `publisher-authority`, `publisher-receiver`, `exit-bridge` are built
from the **same Dockerfiles** used by GBN-PROTO-007. The local track uses local image tags
(e.g., `veritas/publisher-authority:dev`) loaded into the k3d cluster via
`k3d image import`. No ECR push needed for local iteration.

### 3.3 No-AWS-Calls Rule

The local track must function with zero AWS API calls. No `aws-config`, no `aws-sdk-*`,
no implicit credential chain at runtime. The metrics emission code (Phase 3 of this plan)
must compile and run without AWS SDK deps; if the AWS variant is also wanted in the same
binary, gate the AWS code path behind a Cargo feature flag.

### 3.4 chain_id Trace Continuity Rule

The same `chain_id` propagation that GBN-PROTO-006 Phase 7 enforced across services must
also flow into Tempo spans. Each service's tracing layer adds `chain_id` as a span
attribute; Grafana's Tempo Explore panel can then filter by `chain_id` and reconstruct
the full distributed call tree.

### 3.5 V1 Preservation Rule

V1 paths remain untouched. The local-k8s track lives entirely under
`prototype/gbn-bridge-proto/infra/k8s/` and adjacent script directories. V1's
`prototype/gbn-proto/infra/scripts/` is read-only for reference.

### 3.6 PR Granularity Rule

Same as GBN-PROTO-007: one PR per phase. Phases 1 → 2 → 3 → 4 of this plan land in order.
Phase 3 of this plan can land independently of GBN-PROTO-007 Phase 3 (they don't conflict;
they touch different code paths gated by feature flags or env vars).

### 3.7 V1 Regression Rule

Every GBN-PROTO-008 phase must finish with the V1 cargo test suite passing on the V1
workspace, even though no V1 files are modified. Phases 3 and 4 in particular touch
shared workspace `Cargo.toml` deps and feature flags, so a clean V1 build is the
guardrail against accidental regressions to the protected V1 baseline. The protected
path list from [GBN-PROTO-007 §3.2](GBN-PROTO-007-Conduit-V2-V1-Parity-Execution-Plan.md)
applies verbatim to this track.

---

## 4. Locked Decisions

### 4.1 Cluster Topology

A k3d cluster with **1 server + 2 agents** by default. The Conduit topology fits onto 3
nodes comfortably, exercises the scheduler enough to catch placement bugs, and stays
under 2 GB RAM. Single-node mode (`--agents 0`) is documented as a fallback for
constrained workstations.

### 4.2 Image Pull Strategy

Local images are built and imported via `k3d image import`. `imagePullPolicy: IfNotPresent`
on every Deployment so the cluster doesn't try to fetch from a remote registry.

### 4.3 Postgres Deployment Shape

PostgreSQL runs as a `StatefulSet` with a 1 Gi `PersistentVolumeClaim` backed by k3d's
default `local-path` provisioner. Credentials live in a Kubernetes `Secret`; the Conduit
authority Pod reads them via env-var-from-secret. **No** RDS analog or Aurora.

### 4.4 Service Discovery

Each Conduit service has a Kubernetes `Service` of type `ClusterIP`. Internal DNS
(`publisher-authority.veritas.svc.cluster.local`, etc.) replaces Cloud Map. The bridge
container's `GBN_BRIDGE_AUTHORITY_URL` env var points at the cluster-DNS hostname.

### 4.5 Observability Namespace

All observability components install into a single namespace: `observability`. Grafana
exposes a NodePort or `kubectl port-forward` for local access.

### 4.6 Persistence Retention

- Loki retention: 7 days (sufficient for iteration, prevents disk filling).
- Tempo retention: 24 hours (traces are large; recent is enough for debugging).
- Prometheus retention: 15 days (default).

### 4.7 Emission Cadence

Prometheus scrape interval: 15 seconds (vs CloudWatch's 60 s). Faster feedback for inner
loop. Conduit services expose `/metrics` over HTTP — no push, no AWS SDK.

---

## 5. Phase Summaries

### Phase 1 — Local Kubernetes Cluster + Conduit Manifests
[GBN-PROTO-008-Execution-Phase1-Local-Kubernetes-Cluster-And-Conduit-Manifests.md](GBN-PROTO-008-Execution-Phase1-Local-Kubernetes-Cluster-And-Conduit-Manifests.md)

Install `k3d` on WSL2; create the cluster; build and import the 3 Conduit container
images; write Kubernetes manifests under `prototype/gbn-bridge-proto/infra/k8s/conduit/`
(namespace, configmap, secret, postgres statefulset, 3 deployments + services, ingress);
write a single bring-up script `infra/scripts/k8s-up.sh` and tear-down `k8s-down.sh`.

### Phase 2 — Observability Stack
[GBN-PROTO-008-Execution-Phase2-Observability-Stack.md](GBN-PROTO-008-Execution-Phase2-Observability-Stack.md)

Install `kube-prometheus-stack`, `loki-stack`, and `tempo` Helm charts into the
`observability` namespace. Pre-provision Grafana datasources and a Conduit overview
dashboard. Configure Prometheus to scrape `/metrics` on every Conduit pod via a
`ServiceMonitor` or pod annotation. Configure Promtail to ship `chain_id`-tagged logs to
Loki. Configure Tempo to receive OTLP spans from Conduit binaries.

### Phase 3 — Prometheus Metrics Emission
[GBN-PROTO-008-Execution-Phase3-Prometheus-Metrics-Emission.md](GBN-PROTO-008-Execution-Phase3-Prometheus-Metrics-Emission.md)

Variant of GBN-PROTO-007 Phase 3. Replace the CloudWatch push design with a `/metrics`
HTTP endpoint on each binary's existing public port (8080 for authority, 8081 for
receiver, an added internal port for bridge) using the `prometheus` Rust crate. Same
underlying counters (`AuthorityMetricsSnapshot`, new `ReceiverMetricsSnapshot`,
`BridgeMetricsSnapshot`); different export mechanism. Add OpenTelemetry tracing layer
that emits spans with `chain_id` attribute to the Tempo OTLP collector.

### Phase 4 — Local Kubernetes Operator Script
[GBN-PROTO-008-Execution-Phase4-Local-Kubernetes-Operator-Script.md](GBN-PROTO-008-Execution-Phase4-Local-Kubernetes-Operator-Script.md)

Variant of GBN-PROTO-007 Phase 5. New script
`prototype/gbn-bridge-proto/infra/scripts/k8s-control-interactive.sh` mirroring the V1
operator panel but using `kubectl exec` instead of `aws ecs execute-command`. Same menu
items (Status / DumpBridges / DumpFrames / SendDummy / TriggerCommand / etc.). LiveMetrics
opens Grafana via `kubectl port-forward` and prints the URL. Trace collection after
SendDummy queries Tempo via its HTTP API for a `chain_id` filter and prints the result.

---

## 6. Out Of Scope

- Production EKS deployment (still GBN-PROTO-007's responsibility).
- Multi-cluster federation.
- Service mesh (Istio / Linkerd) — premature; Conduit's three pods don't need one.
- Hardening (NetworkPolicies, PodSecurityStandards, mTLS between pods) — deferred.
- Persistence guarantees beyond what the `local-path` provisioner gives.
- Modifying `prototype/gbn-proto/**` (V1 stays frozen).
- Modifying the top-level `README.md`.
- Replacing GBN-PROTO-007 Phase 3 (CloudWatch). The AWS path remains valid behind a
  feature flag.

---

## 7. Validation Strategy

After all four GBN-PROTO-008 phases land:

1. Fresh WSL2 Ubuntu shell. Run `bash prototype/gbn-bridge-proto/infra/scripts/k8s-up.sh`.
2. Within ~5 minutes, the cluster is up, all 5 Conduit pods are `Ready`, and Grafana is
   reachable at `http://localhost:3000` (default creds documented in Phase 2).
3. Run `bash prototype/gbn-bridge-proto/infra/scripts/k8s-control-interactive.sh`.
4. Walk every menu item; each succeeds or prints a meaningful error.
5. SendDummy from each of the 5 pods (Authority, Receiver, all 3 Bridges):
   - returns a chain_id
   - Grafana → Tempo Explore for that chain_id shows a 3-hop trace
     (creator-pod → bridge-pod → receiver-pod)
   - Grafana → Loki Explore for `{namespace="veritas"} |= "<chain_id>"` shows the trace
     events in chronological order
6. Grafana → Prometheus dashboard shows non-zero `Veritas/Conduit` counters within 30s
   of metric activity.
7. Tear down: `bash prototype/gbn-bridge-proto/infra/scripts/k8s-down.sh`. Cluster is
   removed and disk is reclaimed.
8. Total inner-loop time (code change → tested in cluster): under 60 seconds.
9. Total AWS spend during the entire validation: $0.

---

## 8. Migration Path Back To AWS

When the operator wants to deploy the same Conduit code to EKS or back to ECS Fargate:

- The container images are unchanged.
- GBN-PROTO-007 Phase 3 (CloudWatch metrics) and Phase 5 (`relay-control-interactive-v2.sh`)
  are still in tree.
- The operator builds and pushes images to ECR, then runs the existing
  `deploy-conduit-full.sh`.
- For EKS specifically, the Kubernetes manifests from GBN-PROTO-008 Phase 1 can be reused
  with minimal changes (mostly: `local-path` storage class → EBS, `ClusterIP` services →
  ALB Ingress).
- Both observability stacks coexist: in EKS you can install the same Helm charts; in ECS
  you fall back to CloudWatch.

The two plans are not either/or — they are different deployment targets for the same
binaries.
