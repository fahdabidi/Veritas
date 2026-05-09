# GBN-PROTO-012 - Execution Phase 7 - Smoke 1 Tracing Suite Implementation (Pass 3 Successor To GBN-PROTO-011)

**Document ID:** GBN-PROTO-012-Smoke-1
**Status:** Pending
**Last Updated:** 2026-05-08
**Phase:** 7 (Smoke 1 — Tracing Suite Implementation)
**Supersedes (in scope only):** [GBN-PROTO-011 Local Kubernetes Tracing Smoke Test Plan](../Full-Implementation-Plan-Pass2/GBN-PROTO-011-Local-Kubernetes-Tracing-Smoke-Test-Plan.md)
**Parent Plan:** [GBN-PROTO-012](GBN-PROTO-012-Conduit-Architecture-Correct-Bootstrap-Execution-Plan.md)
**Depends On:** Phase 0 (cluster topology + creator pods + `echo-chain-id` endpoint)

This is **Smoke 1** in the Pass 3 local Kubernetes Conduit suite:

1. **Smoke 1** — distributed tracing/logging instrumentation is alive for all nodes.
2. Smoke 2 — every potential creator completes the section 3.3 creator bootup flow
   and stores Publisher-signed DHT / discovery entries locally.
3. Smoke 3 — selected creator constructs a route from its local DHT, exchanges an
   encryption-enveloped dummy frame with the Publisher, and demonstrates failover.

If Smoke 1 fails, Smoke 2 and Smoke 3 should not be trusted; their failure analysis
depends on the same `chain_id` propagation, Loki searchability, and Tempo span export
validated here. Smoke 1 is the first gate.

The Pass 2 file `GBN-PROTO-011-Local-Kubernetes-Tracing-Smoke-Test-Plan.md` is left
unchanged. This Pass 3 successor document defines the assertions that the Pass 3
implementation must satisfy.

---

## 1. Goal

Prove that:

1. All 20 pods deployed in Phase 0 emit logs and spans tagged with the same
   `chain_id` for any test probe.
2. Loki indexes those `chain_id`s within the test timeout (default 15 s).
3. Tempo indexes those spans within the test timeout (default 15 s).
4. Every `service.name` Tempo expects (`publisher-authority`, `publisher-receiver`,
   `exit-bridge-0` … `exit-bridge-9`, `creator-host`, `creator-new`) is exporting
   spans.
5. Prometheus has scraped recent samples from every `/metrics` endpoint exposed by
   Phase 0's expanded topology.

---

## 2. Pre-Conditions

- WSL2 Ubuntu host (Master plan §2.8 guard at top of script).
- WSL2 host allocation per Phase 0: `memory=10GB processors=6 swap=4GB`.
- `bash prototype/gbn-bridge-proto/infra/scripts/k8s-up.sh` completed; all 20 pods
  Ready.
- Observability stack Ready (Prometheus, Grafana, Loki, Promtail, Tempo).

---

## 3. Implemented Command

```bash
bash prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-tracing-v3.sh \
  --require-observability \
  --timeout 15
```

Flags:

- `--require-observability` (default on): refuse to run if any of Prometheus, Loki,
  or Tempo is not Ready. Without it the script still runs but skips the obs assertions
  and reports "no obs evidence" instead of pass.
- `--timeout N`: per-query timeout in seconds.
- `--chain-id-prefix smoke-1-`: prefix used so artifacts are easy to grep.

---

## 4. Probe Sequence

1. Generate `chain_id = smoke-1-<UUID>`.
2. POST `/v1/admin/echo-chain-id` (a no-op admin endpoint that exists solely to emit
   a `chain_id`-tagged log+span pair from each node) on every pod:
   - 2 Publisher surfaces;
   - 10 ExitBridges;
   - 2 creators.
3. Assert every response echoes the exact generated `chain_id`, `actor_id`, and
   `role`; any mismatch fails before observability queries run.
4. Wait `--timeout` seconds.
5. Run assertions.

The `/v1/admin/echo-chain-id` endpoint is added in Phase 0 alongside `node-metadata`.
It accepts `{ "chain_id": "smoke-1-..." }`, emits one log line and one span per call
with that chain_id and the local actor_id, and returns
`{ "chain_id": "...", "actor_id": "...", "role": "..." }`. Adding this endpoint to
the `creator-runner` binary is part of Phase 0's cluster bring-up scope.

---

## 5. Assertions

### 5.1 Loki

For each of the 14 pod actors:

- `LogQL: {namespace="veritas",actor_id="<id>"} |= "<chain_id>"` returns ≥ 1 entry.
- The matched entry contains `actor_id`, `role`, and `chain_id` keys.
- The indexed `chain_id` equals the response `chain_id` for that actor.

### 5.2 Tempo

For each of the 14 pod actors:

- `traceql: { service.name="<actor_id>" && chain_id="<chain_id>" }` returns ≥ 1 span.
- Span attributes include `chain_id`, `actor_id`, and `role`.
- The span `chain_id` equals the response `chain_id` for that actor.

### 5.3 Prometheus

- All `up` series for the Conduit `ServiceMonitor`s are 1.
- The following counters have at least one fresh sample within the last 60 s
  (proves scrape works):
  - `conduit_authority_*`
  - `conduit_receiver_*`
  - `conduit_bridge_*` (10 series, one per bridge)
  - `conduit_creator_*` (2 series, one per creator)

---

## 6. Artifacts

Written to `/tmp/conduit-smoke-1-${chain_id}/`:

- `pods.json`
- `chain-id.txt`
- `loki-hits-by-actor.json`
- `tempo-spans-by-actor.json`
- `prometheus-up.json`
- `prometheus-counter-samples.json`
- `summary.md` (table: actor_id, role, loki_hits, tempo_spans, prom_up)

The script prints the artifact directory path on success and on failure.

---

## 7. Success Criteria

- Script exits 0.
- Every actor reports ≥ 1 Loki entry, ≥ 1 Tempo span for the test `chain_id`.
- Every Prometheus `up` series is 1.
- Every pod has at least one fresh counter sample within the scrape window.

## 8. Failure Modes And Triage

| Failure | Likely Cause |
|---|---|
| Loki has no hits for actor X | Promtail not picking up X's logs; check namespace label, container path |
| Tempo has spans for X but no `chain_id` attribute | OTLP exporter misconfigured on X; missing span processor |
| Prometheus `up=0` for X | ServiceMonitor selector or scrape port mismatch |
| All actors fail | Observability stack itself is degraded; check `kubectl get pods -n observability` |

---

## 9. Out Of Scope

- Bootstrap and route assertions (Smoke 2, Smoke 3).
- AWS X-Ray; AWS uses CloudWatch Logs + X-Ray with a parallel script invoked from
  Phase 6's AWS acceptance flow.
- Performance, scale, or latency assertions.

---

## 10. Implementation Tasks

1. Add `/v1/admin/echo-chain-id` endpoint to all four binaries
   (`gbn-bridge-publisher` for both surfaces, `gbn-bridge-cli` exit-bridge runner,
   and `creator-runner`). Single-purpose: emit log + span with the supplied
   `chain_id`.
2. Add `infra/scripts/k8s-smoke-tracing-v3.sh` (new file). Sources
   `k8s-smoke-common.sh` for shared Loki/Tempo/Prometheus helpers.
3. Extend `k8s-smoke-common.sh` with helpers for:
   - `pod_list_by_role(role)` returning JSON;
   - `loki_query_chain_id(chain_id, actor_id, timeout)`;
   - `tempo_query_chain_id(chain_id, service_name, timeout)`.
4. WSL2 guard at top of script per Master plan §2.8.

---

## 11. Acceptance

- `bash -n prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-tracing-v3.sh` passes.
- Against a fresh `k8s-up.sh` cluster, the script exits 0.
- The script produces a `summary.md` showing pass for all 14 actors.
- `git diff --stat -- docs/prototyping/Conduit/Full-Implementation-Plan-Pass2/`
  is empty (Pass 2 is not modified).
- V1 (`prototype/gbn-proto/**`) is unchanged.
