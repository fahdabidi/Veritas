# Veritas Conduit Mobile Validation Matrix

This matrix defines the Conduit mobile and AWS validation scenarios, the primary
evidence source for each one, and the current execution state.

## Scope

The legacy `mobile-validation.sh` script remains available for the earlier
prototype stack. The full implementation validation surface introduced by
GBN-PROTO-006 Phase 10 uses:

- `infra/scripts/mobile-validation-full.sh`
- `infra/scripts/collect-conduit-traces.sh`
- `docs/prototyping/Conduit/Full-Implementation-Plan/GBN-PROTO-006-Conduit-Full-Implementation-Test-Report.md`

## Scenario Matrix

| Scenario | Goal | Primary Command / Evidence | Acceptance Target | Current State |
|---|---|---|---|---|
| App restart with cached catalog | creator reconnects after restart using signed cached state | `mobile-validation-full.sh --mode local` plus distributed e2e trace artifacts | reconnect and refresh without trust-root drift | local full implementation evidence available; live mobile run pending |
| Stale bridge recovery | creator skips stale or downgraded bridges and still refreshes catalog | `mobile-validation-full.sh --mode local` plus reachability/e2e tests | stale entry does not block refresh success | local full implementation evidence available; live mobile run pending |
| First-time bootstrap | new creator reaches Publisher through HostCreator path and establishes seed tunnel | `mobile-validation-full.sh --mode local` plus `tests/e2e/bootstrap.rs` | bootstrap completes and seed bridge becomes active | local full implementation evidence available; live AWS/mobile run pending |
| UDP punch ACK on default port | creator and seed bridge complete bidirectional punch ACK on `443` unless overridden | `mobile-validation-full.sh --mode local` plus e2e bootstrap/data path tests | ACK success on signed port with no class mismatch | local full implementation evidence available; live AWS/mobile run pending |
| Network switch / IP churn | creator survives network identity change without losing all upload paths | live AWS/mobile run plus `collect-conduit-traces.sh` | catalog refresh and continued fanout within one recovery cycle | pending live AWS/mobile run |
| Bridge failover latency | upload continues after one bridge failure | `mobile-validation-full.sh --mode local` plus `tests/e2e/failover.rs` | failover remains within one reassignment cycle | local full implementation evidence available; live AWS/mobile run pending |
| Fanout reuse after churn | creator reuses already-live bridges when full 10-bridge set is unavailable | `tests/e2e/data_path.rs` and live AWS/mobile traces | session completes without full 10-bridge availability | local full implementation evidence available; live mobile run pending |
| Batched onboarding latency | first 10 join requests stay in one batch; 11th rolls cleanly into next | full workspace tests plus AWS trace snapshot | 10-request window stays in one batch, 11th is isolated to next rollover | local evidence available; live AWS timing still pending |
| End-to-end chain trace | one root trace is visible across authority, bridge, receiver, and validation artifacts | `collect-conduit-traces.sh --chain-id <id> --require-chain-id` | matching events appear in all three service log groups | pending live AWS/mobile run |

## Provisional Thresholds

These thresholds are the Phase 11 targets to measure against during live runs.

| Metric | Target |
|---|---|
| first-time bootstrap to seed-tunnel ACK | <= 30s |
| returning creator refresh after restart | <= 10s |
| bridge failover reassignment | <= 5s in local harness, <= 15s in AWS/mobile run |
| stale bridge recovery | <= 2 refresh attempts |
| batch rollover penalty for 11th join | <= one additional batch window plus 2s control latency |

## Tooling

- legacy prototype local proxy workflow: `prototype/gbn-bridge-proto/infra/scripts/mobile-validation.sh --mode local`
- full implementation local workflow: `prototype/gbn-bridge-proto/infra/scripts/mobile-validation-full.sh --mode local`
- full implementation AWS/mobile workflow: `prototype/gbn-bridge-proto/infra/scripts/mobile-validation-full.sh --mode aws`
- full implementation trace collection: `prototype/gbn-bridge-proto/infra/scripts/collect-conduit-traces.sh`

## Current Limitation

The full implementation now has deployment binaries for authority, receiver,
and exit bridge services. Live mobile-carrier behavior is still pending until a
deployed `gbn-conduit-full-*` stack is exercised from a real mobile network path
and its `chain_id` traces are collected.
