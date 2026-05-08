# GBN-PROTO-010 - Local Kubernetes Creator To Publisher Route Smoke Test Plan

**Document ID:** GBN-PROTO-010
**Status:** Implemented - `k8s-smoke-route.sh`
**Last Updated:** 2026-05-08
**Related Docs:**
[GBN-PROTO-009 Local Kubernetes Discovery Smoke Test Plan](GBN-PROTO-009-Local-Kubernetes-Discovery-Smoke-Test-Plan.md),
[GBN-PROTO-008 Local Kubernetes Test Infrastructure](GBN-PROTO-008-Local-Kubernetes-Test-Infrastructure-Execution-Plan.md),
[GBN-PROTO-007 V2-V1 Parity Execution Plan](GBN-PROTO-007-Conduit-V2-V1-Parity-Execution-Plan.md)

This plan defines the second automated local Kubernetes smoke test for Conduit V2. It
selects one or more local nodes as the creator actor, creates a route through an assigned
bridge to the publisher receiver, sends a dummy message, and proves the message was
accepted, persisted, logged, and traced.

---

## 1. Goal

Automate the creator-to-publisher data path:

1. Select a Conduit pod as the creator actor.
2. Ask the publisher authority for a bridge assignment.
3. Send a dummy payload through the assigned bridge to the publisher receiver.
4. Verify the receiver accepted and persisted the frame.
5. Verify Prometheus, Loki, and Tempo can debug the route by `chain_id`.

This test intentionally depends on the discovery smoke test contract from GBN-PROTO-009:
all bridge discovery/registry assertions should already be green before route assertions
run.

---

## 2. Implemented Test Command

Implemented script:

```bash
bash prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-route.sh \
  --namespace veritas \
  --creator-selector veritas-role=authority \
  --message-size 256 \
  --require-observability
```

Supported selector modes:

- `--creator-selector veritas-role=authority`
- `--creator-selector veritas-role=receiver`
- `--creator-selector veritas-role=bridge`
- `--all-creators` to run authority, receiver, and every bridge pod, matching the current
  `k8s-smoke.sh --send-dummy` coverage.

---

## 3. Required Test Surface

Existing surfaces:

- `POST /v1/admin/send-dummy` on authority, receiver, and bridge pods.
- `GET /v1/admin/frames?chain_id=...` on authority.
- `GET /v1/admin/metrics` on authority, receiver, and bridge pods.
- Prometheus, Loki, and Tempo local services from GBN-PROTO-008.

No new Conduit HTTP route is required for the first version of this test. The plan should
reuse `send-dummy` and add stronger assertions plus artifacts around the route it creates.

---

## 4. Assertions

### 4.1 Preflight

- Namespace exists.
- Conduit Deployments/StatefulSet are Ready.
- Authority registry has at least the expected bridge count.
- `GET /v1/admin/metrics` works for the selected creator pod, the authority pod, receiver
  pod, and all bridge pods.
- If `--require-observability` is set, Prometheus, Loki, and Tempo readiness checks pass
  before sending traffic.

### 4.2 Route Creation And Message Delivery

For each selected creator:

1. Record baseline metrics:
   - authority `issued_catalogs`
   - receiver `frames_accepted`
   - receiver `bytes_ingested`
   - bridge `frames_forwarded` for all bridges
2. Call:

```http
POST /v1/admin/send-dummy
{"size": 256}
```

3. Assert response:
   - `chain_id` is non-empty and unique.
   - `assigned_bridge_id` is non-empty.
   - `elapsed_ms` is present and below the configured timeout.
4. Assert authority frame persistence:
   - `GET /v1/admin/frames?chain_id=<chain_id>` returns at least one frame.
   - every returned frame has `chain_id=<chain_id>`.
   - at least one frame has `via_bridge_id=<assigned_bridge_id>`.
5. Assert metrics increased:
   - receiver `frames_accepted` increases by at least 1.
   - receiver `bytes_ingested` increases by at least `--message-size`.
   - the assigned bridge's `frames_forwarded` increases by at least 1.
   - authority `issued_catalogs` increases by at least 1 or is already higher in a reused
     cluster and the per-chain evidence is present.

### 4.3 Observability Evidence

For each returned `chain_id`:

- Loki query returns logs from at least:
  - creator pod
  - assigned bridge pod
  - receiver pod
- Tempo query returns spans tagged with that `chain_id`. The expected route evidence is:
  - creator/bootstrap or admin send-dummy span
  - bridge upload/forward span
  - receiver ingestion span
- Prometheus query returns:
  - `conduit_receiver_frames_accepted_total`
  - `conduit_bridge_frames_forwarded_total`
  - `conduit_authority_issued_catalogs_total`

If exact Tempo span names are not stable yet, the first implementation should assert
`chain_id` match and at least three service names. If service names are not available, fail
with the raw Tempo result so instrumentation gaps are explicit.

---

## 5. Evidence Artifacts

Write artifacts under:

```text
prototype/gbn-bridge-proto/target/k8s-smoke-artifacts/route/<run-id>/
```

Required files:

- `pods.json`
- `bridges.json`
- `send-dummy-results.json`
- `frames-by-chain-id.json`
- `admin-metrics-before.json`
- `admin-metrics-after.json`
- `prometheus-route-counters.json`
- `loki-chain-id-hits.json`
- `tempo-chain-id-hits.json`
- `trace-summary.md`

The summary must include one row per creator:

| creator_pod | chain_id | assigned_bridge_id | frames | loki_hits | tempo_hits |
|---|---|---|---|---|---|

---

## 6. Implementation Tasks

1. `infra/scripts/k8s-smoke-route.sh` implements the route assertions.
2. `infra/scripts/k8s-smoke-common.sh` owns shared Kubernetes/admin/observability helpers.
3. Direct Prometheus/Loki/Tempo query helpers are shared by the tracing, discovery, and
   route smoke tests.
4. Shell JSON parsing remains simple Python one-liners; no separate helper tests are
   required yet.
5. `k8s-smoke.sh --send-dummy` remains in place for broad compatibility; this new script
   is the stricter route-specific test.

---

## 7. Acceptance Criteria

- `bash -n infra/scripts/k8s-smoke-route.sh` passes.
- Against a fresh GBN-PROTO-008 local cluster, one selected creator can send a dummy
  message and the script exits 0.
- With `--all-creators`, authority, receiver, and every bridge pod can each send a dummy
  message and produce separate `chain_id` evidence.
- Receiver persistence, assigned bridge forwarding, Loki logs, and Tempo spans are all
  verified for each `chain_id`.
- On failure, the script prints the failed assertion and writes the artifact bundle.

---

## 8. Out Of Scope

- Discovery-only assertions. Those belong to GBN-PROTO-009.
- Load, throughput, packet loss, bridge restart, and failover testing.
- AWS ECS execution.
- Manual Grafana inspection as a required pass/fail step. Grafana links may be printed,
  but this test must be machine-verifiable.
