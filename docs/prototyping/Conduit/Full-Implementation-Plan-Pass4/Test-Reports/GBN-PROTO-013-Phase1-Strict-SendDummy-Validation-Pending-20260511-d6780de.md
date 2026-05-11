# GBN-PROTO-013 Phase 1 Strict SendDummy Validation Report

## Run Metadata

- Date: `2026-05-11`
- Workspace: `prototype/gbn-bridge-proto`
- Commit under test: `d6780de feat: harden pass4 bootstrap validation`
- Script: `infra/scripts/k8s-smoke-senddummy-strict-v4.sh`
- Required environment: WSL2 Ubuntu with local k8s Conduit stack
- Result: `PENDING K8S EXECUTION`

## Scope

This report is the Pass 4 equivalent of the Pass 3 Smoke 3 Route detailed report.
It is ready to be replaced by the generated run report after strict SendDummy is executed
from WSL2 Ubuntu.

The strict SendDummy smoke must prove:

1. NewCreator is onboarded by the strict bootstrap path.
2. SendDummy selects routes from local DHT, not Publisher-side route synthesis.
3. Dummy payload is encrypted before crossing an ExitBridge.
4. Selected ExitBridge logs do not contain the plaintext marker.
5. Publisher receiver persists the frame.
6. Publisher decrypts and hash-validates the payload.
7. Forced bridge-failure variant reroutes through another active local DHT entry.
8. ChainID evidence is archived for normal and failover sends.

## Command

Run from WSL2 Ubuntu:

```bash
cd prototype/gbn-bridge-proto
infra/scripts/k8s-smoke-senddummy-strict-v4.sh \
  --require-observability \
  --require-onboarded-from-strict-bootstrap \
  --require-route-source local_dht \
  --require-ciphertext-only-at-bridge
```

## Expected Artifact Directory

The script writes under:

```text
prototype/gbn-bridge-proto/target/k8s-smoke-artifacts/smoke-3-senddummy-strict-v4/<run-id>
```

The strict wrapper nests the bootstrap prerequisite under:

```text
bootstrap/
```

and the route smoke under:

```text
route/
```

It also archives the tracked report under:

```text
docs/prototyping/Conduit/Full-Implementation-Plan-Pass4/Test-Reports/
```

with prefix:

```text
GBN-PROTO-013-Phase1-Strict-SendDummy-<run-id>.md
```

## Required Evidence Artifacts

| Evidence | Artifact |
|---|---|
| Strict bootstrap prerequisite | `bootstrap/strict-bootstrap-summary.json` |
| Creator local DHT readiness | `route/creator-local-dht-ready-summary.json` |
| Pre-send DHT evidence | `route/dht-evidence/pre-send/dht-summary.json` |
| Normal SendDummy result | `route/send-dummy-normal-result.json` |
| Normal receiver frames | `route/frames-normal.json` |
| Normal Publisher decrypt/hash validation | `route/received-dummy-normal.json` |
| Forced-failover SendDummy result | `route/send-dummy-failover-result.json` |
| Forced-failover receiver frames | `route/frames-failover.json` |
| Forced-failover Publisher decrypt/hash validation | `route/received-dummy-failover.json` |
| Bridge ciphertext-only check | `route/bridge-plaintext-grep.txt` |
| ChainID evidence | `route/chainid-evidence/`, `route/bridge-logs-by-chain-id/` |
| Strict summary | `strict-senddummy-summary.json` |
| Strict detailed report | `strict-report.md` |

## API Gate Ledger

| Gate | Status Before Run | Required Pass Condition |
|---|---:|---|
| Strict bootstrap prerequisite | pending | Bootstrap wrapper produces matching session id |
| NewCreator local DHT readiness | pending | `state=onboarded` with expected active bridge count |
| Route source | pending | Normal and failover results show `route_source=local_dht` |
| Normal SendDummy API completion | pending | Result contains ChainID and assigned bridge id |
| Normal receiver persistence | pending | At least one frame persisted for normal ChainID |
| Normal Publisher decrypt/hash validation | pending | `payload_hash_match=true` |
| Failover SendDummy API completion | pending | Forced-failover result contains ChainID and assigned bridge id |
| Failover receiver persistence | pending | At least one frame persisted for failover ChainID |
| Failover Publisher decrypt/hash validation | pending | `payload_hash_match=true` |
| Ciphertext-only bridge boundary | pending | `bridge-plaintext-grep.txt` is empty |
| ChainID evidence | pending | Creator, Publisher, receiver, and selected bridge logs contain ChainID |

## Script Readiness Evidence

The script was syntax-checked in the implementation environment:

```bash
bash -n prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-senddummy-strict-v4.sh
```

Observed result: `PASS`.

## Result

The strict SendDummy report is pending live WSL2/k8s execution. No k8s PASS result is
claimed by this placeholder report.
