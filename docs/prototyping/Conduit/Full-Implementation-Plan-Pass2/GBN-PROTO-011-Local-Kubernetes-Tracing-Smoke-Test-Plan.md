# GBN-PROTO-011 - Local Kubernetes Tracing Smoke Test Plan

**Document ID:** GBN-PROTO-011
**Status:** Implemented - `k8s-smoke-tracing.sh`
**Last Updated:** 2026-05-08
**Related Docs:**
[GBN-PROTO-008 Local Kubernetes Test Infrastructure](GBN-PROTO-008-Local-Kubernetes-Test-Infrastructure-Execution-Plan.md),
[GBN-PROTO-009 Local Kubernetes Discovery Smoke Test Plan](GBN-PROTO-009-Local-Kubernetes-Discovery-Smoke-Test-Plan.md),
[GBN-PROTO-010 Local Kubernetes Creator To Publisher Route Smoke Test Plan](GBN-PROTO-010-Local-Kubernetes-Creator-Publisher-Route-Smoke-Test-Plan.md)

This plan defines the tracing-first smoke test for the local Conduit Kubernetes stack. It
proves that the instrumentation itself is alive before deeper discovery or route assertions
start depending on it for debugging.

---

## 1. Goal

Launch a local smoke test and confirm that distributed tracing/log evidence appears for all
Conduit node roles:

1. authority
2. receiver
3. every bridge pod

The test generates fresh `chain_id` values, verifies those IDs appear in local pod logs,
verifies Loki can query those IDs, verifies Tempo can query those IDs, and verifies Tempo
exposes the expected `chain_id` and `service.name` tags.

---

## 2. Implemented Command

```bash
bash prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-tracing.sh \
  --namespace veritas \
  --expected-bridges 3 \
  --message-size 128
```

The script writes artifacts under:

```text
prototype/gbn-bridge-proto/target/k8s-smoke-artifacts/tracing/<run-id>/
```

---

## 3. Assertions

- Conduit local Kubernetes rollouts are Ready.
- Authority registry contains the expected bridge count.
- Admin metrics endpoints respond on every node.
- Prometheus, Loki, and Tempo readiness checks pass through local port-forwards.
- The script triggers `POST /v1/admin/send-dummy` from authority, receiver, and every
  bridge pod to generate role-specific trace evidence.
- For every returned `chain_id`:
  - local pod logs contain the creator node's own `chain_id`;
  - Loki returns at least one hit;
  - Tempo returns at least one trace hit.
- Tempo tag discovery includes `chain_id` and `service.name`.
- Prometheus `up{namespace="veritas"}` returns the expected Conduit scrape targets.

---

## 4. Debug Artifacts

- `send-dummy-results.jsonl`
- `prometheus-up.json`
- `tempo-tags.json`
- `loki/<chain_id>.json`
- `tempo/<chain_id>.json`
- `kubectl-logs/<pod>.log`
- `trace-summary.md`

These artifacts are deliberately shared with the discovery and route smoke test style so a
failed run can be compared across all three layers.
