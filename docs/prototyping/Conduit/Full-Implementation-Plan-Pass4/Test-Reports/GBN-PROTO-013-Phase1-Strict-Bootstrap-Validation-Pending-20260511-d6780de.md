# GBN-PROTO-013 Phase 1 Strict Bootstrap Validation Report

## Run Metadata

- Date: `2026-05-11`
- Workspace: `prototype/gbn-bridge-proto`
- Commit under test: `d6780de feat: harden pass4 bootstrap validation`
- Script: `infra/scripts/k8s-smoke-bootstrap-strict-v4.sh`
- Required environment: WSL2 Ubuntu with local k8s Conduit stack
- Result: `PENDING K8S EXECUTION`

## Scope

This report is the Pass 4 equivalent of the Pass 3 Smoke 2 Discovery detailed report.
It is ready to be replaced by the generated run report after the strict bootstrap smoke is
executed from WSL2 Ubuntu.

The strict bootstrap smoke must prove:

1. NewCreator starts with only HostCreator DHT reachability.
2. Publisher receives NewCreator DHT and NewCreator encryption public key through HostCreator.
3. Publisher returns an encrypted CreatorBootstrap payload through HostCreator.
4. Initial relay response does not carry a plaintext full bridge set.
5. SeedBridgeCatalog is handed back as encrypted payload metadata.
6. Seed bridge and NewCreator report seed tunnel progress.
7. Every selected ExitBridge reports per-bridge fanout progress before completion.
8. NewCreator local DHT agrees with Publisher bootstrap session state.
9. ChainID evidence is archived across Publisher, HostCreator, NewCreator, seed bridge, and remaining bridges.

## Command

Run from WSL2 Ubuntu:

```bash
cd prototype/gbn-bridge-proto
infra/scripts/k8s-up.sh
infra/scripts/k8s-observability-up.sh
infra/scripts/k8s-smoke-bootstrap-strict-v4.sh \
  --require-observability \
  --require-encrypted-bootstrap-payload \
  --require-seed-bridge-catalog-handoff \
  --require-real-fanout-progress
```

## Expected Artifact Directory

The script writes under:

```text
prototype/gbn-bridge-proto/target/k8s-smoke-artifacts/smoke-2-bootstrap-strict-v4/<run-id>
```

It also archives the tracked report under:

```text
docs/prototyping/Conduit/Full-Implementation-Plan-Pass4/Test-Reports/
```

with prefix:

```text
GBN-PROTO-013-Phase1-Strict-Bootstrap-<run-id>.md
```

## Required Evidence Artifacts

| Evidence | Artifact |
|---|---|
| HostCreator seed result | `seed-host-creator-result.json` |
| NewCreator seed/bootstrap response | `seed-new-creator-result.json` |
| Strict encrypted payload summary | `strict-bootstrap-summary.json` |
| Publisher bootstrap session | `bootstrap-session.json` |
| NewCreator local DHT terminal state | `local-dht-final.json` |
| NewCreator DHT progression | `local-dht-progression.jsonl` |
| Publisher DHT dump | `publisher-dht/publisher-dht.json` |
| Bootstrap DHT agreement | `bootstrap-assertion-summary.json` |
| README 15-step flow ledger | `strict-bootstrap-flow-steps.json` |
| ChainID pod-log evidence | `pod-log-events.json`, `pod-logs/*.log` |
| Strict detailed report | `strict-report.md` |

## API Gate Ledger

| README Step | Gate | Status Before Run | Required Pass Condition |
|---:|---|---:|---|
| 1 | NewCreator pairs with HostCreator | pending | `seed-new-creator-payload.json` and result identify `new-creator` and `host-creator` |
| 2 | NewCreator sends DHT entry and public key to HostCreator | pending | strict evidence has `new_creator_dht_entry_id=new-creator` and NewCreator encryption key |
| 3 | HostCreator relays entry request through existing bridge path | pending | pod logs include `host_creator_join_relayed_via_bridge` and `publisher_join_received` |
| 4 | Publisher creates signed bootstrap payload with NewCreator, Publisher, and Seed ExitBridgeB DHT | pending | strict evidence has Publisher entry in payload and Seed ExitBridgeB id |
| 5 | Publisher encrypts bootstrap payload to NewCreator public key | pending | `encrypted_bootstrap_payload.payload_kind=creator_bootstrap` and ciphertext bytes exist |
| 6 | Publisher seeds ExitBridgeB with remaining bridge DHT set | pending | bootstrap session has seed payload progress and expected bridge catalog count |
| 7 | Encrypted bootstrap payload returns through existing path | pending | pod logs include Publisher-to-Host and Host-to-NewCreator relay events |
| 8 | NewCreator decrypts payload and stores Publisher + Seed ExitBridgeB DHT state | pending | final local DHT has Publisher entry and active seed bridge state |
| 9 | NewCreator and ExitBridgeB establish seed tunnel and report progress | pending | progress events include SeedTunnelEstablished from Seed ExitBridgeB and NewCreator |
| 10 | NewCreator requests bridge catalog from ExitBridgeB | pending | pod logs include `new_creator_bridge_set_requested` |
| 11 | ExitBridgeB returns signed remaining bridge catalog | pending | pod logs include `seed_bridge_bridge_set_returned`; catalog has expected bridge count |
| 12 | Publisher fans out NewCreator DHT to remaining ExitBridges | pending | pod logs include `publisher_remaining_bridges_triggered` |
| 13 | Remaining ExitBridges establish tunnels and report progress | pending | every session bridge id reports `bridge_tunnel_established` |
| 14 | NewCreator marks each bridge active only after corresponding progress | pending | active local DHT bridge ids equal progress reporter ids |
| 15 | Every step preserves the same ChainID | pending | seed result, Publisher session, progress events, and pod logs use one ChainID |

## Script Readiness Evidence

The script was syntax-checked in the implementation environment:

```bash
bash -n prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-bootstrap-strict-v4.sh
```

Observed result: `PASS`.

## Result

The strict bootstrap report is pending live WSL2/k8s execution. No k8s PASS result is
claimed by this placeholder report.
