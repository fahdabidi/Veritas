# GBN-PROTO-009 - Local Kubernetes Discovery Smoke Test Plan

**Document ID:** GBN-PROTO-009
**Status:** Implemented - `POST /v1/admin/discovery-probe` and `k8s-smoke-discovery.sh`
**Last Updated:** 2026-05-08
**Related Docs:**
[GBN-PROTO-008 Local Kubernetes Test Infrastructure](GBN-PROTO-008-Local-Kubernetes-Test-Infrastructure-Execution-Plan.md),
[GBN-PROTO-007 V2-V1 Parity Execution Plan](GBN-PROTO-007-Conduit-V2-V1-Parity-Execution-Plan.md)

This plan defines the first automated local Kubernetes smoke test for Conduit V2. It
validates that the local stack comes up, every expected node is reachable, bridge discovery
state is populated, and the observability stack can be used to debug discovery failures.

V1 used a DHT/gossip mental model. V2 is intentionally centralized around the publisher
authority, so this test treats the authority bridge registry and creator catalog/bootstrap
assignment as the V2 equivalent of a populated DHT table. If a future V2 build reintroduces
true peer-local DHT tables, this plan should be extended with per-node DHT dump assertions
instead of inventing a fake DHT surface.

---

## 1. Goal

Automate a local k8s discovery smoke test that proves:

1. The k3d Conduit stack is running: Postgres, authority, receiver, and 3 bridge pods.
2. The authority has registered all expected bridge pods as active, non-revoked bridges.
3. Every test actor can query discovery state and receive a valid bridge assignment without
   sending payload data.
4. Prometheus, Loki, and Tempo contain enough evidence to debug discovery registration,
   catalog issuance, and bootstrap assignment by `chain_id`.

---

## 2. Implemented Test Command

Implemented script:

```bash
bash prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-discovery.sh \
  --namespace veritas \
  --expected-bridges 3 \
  --require-observability
```

The script must be non-interactive and suitable for CI or a local preflight. It may assume
GBN-PROTO-008 has already created the cluster. A `--bring-up` flag can be added later, but
the first implementation should keep cluster lifecycle separate from assertions.

---

## 3. Required Test Surface

Existing surfaces:

- `kubectl rollout status` for pod readiness.
- `GET /readyz` on authority and receiver public listeners.
- `GET /v1/admin/metrics` on authority, receiver, and bridge pods.
- `GET /v1/admin/bridges` on authority.
- Prometheus, Loki, and Tempo local services from GBN-PROTO-008.

Implemented extended surface:

- Localhost-only admin route on each Conduit service:
  `POST /v1/admin/discovery-probe`
- The route must use the same creator configuration already used by
  `POST /v1/admin/send-dummy`, but stop after discovery/bootstrap assignment. It must not
  open a bridge upload session or forward a frame.
- Response shape:

```json
{
  "chain_id": "discovery-probe-...",
  "actor_id": "publisher-authority-...",
  "assigned_bridge_id": "exit-bridge-...",
  "bridge_address": "10.x.y.z:4443",
  "known_bridge_count": 3,
  "known_bridge_ids": ["exit-bridge-a", "exit-bridge-b", "exit-bridge-c"],
  "elapsed_ms": 12
}
```

If the current creator library cannot return `known_bridge_ids` without a small extension,
add a lightweight `DiscoveryProbeResult` that exposes the bridge set returned by the
authority. Do not infer the set from pod names in the script.

---

## 4. Assertions

### 4.1 Infrastructure

- Namespace exists.
- `postgres` StatefulSet is Ready.
- `publisher-authority`, `publisher-receiver`, and `exit-bridge` Deployments are Ready.
- Exactly 1 authority pod, 1 receiver pod, and at least `--expected-bridges` bridge pods
  are Running.
- All admin `/metrics` endpoints return valid JSON.

### 4.2 Authority Registry

`GET /v1/admin/bridges` must return at least `--expected-bridges` active bridge records.
For each current bridge pod:

- `bridge_id` equals the bridge pod name.
- `revoked_reason` is null.
- `current_lease.lease_expiry_ms` is in the future.
- `ingress_endpoints` includes a reachable pod IP and UDP punch port.
- `capabilities` includes the expected bridge capability set.

The script should fail with a compact diff showing missing, stale, or extra active bridge
records.

### 4.3 Discovery Probe From Every Actor

Run `POST /v1/admin/discovery-probe` from:

- authority pod
- receiver pod
- each bridge pod

For each response:

- `chain_id` is non-empty and unique for this run.
- `assigned_bridge_id` is present in the active authority registry.
- `known_bridge_count >= --expected-bridges`.
- `known_bridge_ids` includes every currently Running bridge pod.
- `bridge_address` resolves inside the cluster.
- no payload frame is persisted for that `chain_id`.

### 4.4 Observability Evidence

For every discovery `chain_id`:

- Loki returns recent log lines with that `chain_id`.
- Tempo search returns at least one trace/span for that `chain_id`.
- Prometheus shows:
  - `conduit_authority_successful_registrations_total >= --expected-bridges`
  - `conduit_authority_issued_catalogs_total` increases by the number of discovery probes
    or otherwise reaches at least that value after a fresh cluster
  - `up{namespace="veritas"}` returns all Conduit pod targets as `1`

If Tempo is healthy but trace search returns no matching span, the test should fail with
the Loki excerpts and the Tempo tag list so the missing instrumentation is visible.

---

## 5. Evidence Artifacts

Write artifacts under:

```text
prototype/gbn-bridge-proto/target/k8s-smoke-artifacts/discovery/<run-id>/
```

Required files:

- `pods.json`
- `bridges.json`
- `discovery-probes.json`
- `prometheus-up.json`
- `prometheus-authority-counters.json`
- `loki-chain-id-hits.json`
- `tempo-chain-id-hits.json`
- `trace-summary.md`

The script should print the artifact directory at the end, even on failure.

---

## 6. Implementation Tasks

1. `DiscoveryProbeResult` is in the creator/admin path.
2. `POST /v1/admin/discovery-probe` is mounted by the shared admin module for authority,
   receiver, and bridge binaries.
3. Focused admin tests cover the new route.
4. `infra/scripts/k8s-smoke-discovery.sh` implements the local k8s assertions.
5. `infra/scripts/k8s-smoke-common.sh` owns shared Prometheus/Loki/Tempo query helpers.
6. The test remains opt-in and is not part of default `k8s-up.sh` bring-up.

---

## 7. Acceptance Criteria

- `bash -n infra/scripts/k8s-smoke-discovery.sh` passes.
- Cargo tests covering `DiscoveryProbeResult` and the admin route pass.
- Against a fresh GBN-PROTO-008 local cluster, the discovery smoke script exits 0.
- On failure, the script prints the failed assertion and writes the artifact bundle.
- The test does not send dummy payload data and does not create receiver-ingested frames.

---

## 8. Out Of Scope

- Real V1 DHT/gossip behavior. V2 does not implement that model.
- Public admin exposure or authentication changes.
- AWS ECS execution. This plan is local-k8s only.
- Load, failover, or rolling-restart behavior. Those should become separate tests after
  the discovery and route smoke tests are stable.
