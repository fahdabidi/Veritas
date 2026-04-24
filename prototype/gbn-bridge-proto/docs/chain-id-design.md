# Conduit Chain ID Design

## Purpose

Conduit uses one canonical distributed trace field:

- `chain_id`

This preserves the V1 field name and gives one root correlation key across creator, host creator, exit bridge, and publisher paths.

## Canonical Rules

1. `chain_id` is the only root distributed trace field.
2. Creator-originated flows preserve an incoming trusted `chain_id` or mint one once at the runtime edge.
3. Host creator, bridges, and publisher services forward the existing `chain_id`. They do not replace it with a competing root.
4. `request_id`, `session_id`, `bootstrap_session_id`, and `frame_id` remain useful local identifiers, but they are not the distributed trace root.

## Generation And Import

The runtime canonical helper lives in:

- [`crates/gbn-bridge-runtime/src/trace.rs`](../crates/gbn-bridge-runtime/src/trace.rs)

Rules:

- bootstrap flows use `default_chain_id("bootstrap", host_creator_id, request_id)`
- upload flows use `default_chain_id("upload", creator_id, session_id)`
- service-local import paths must call `import_chain_id(...)` before reusing external input

## Protocol Coverage

The protocol canonical helper lives in:

- [`crates/gbn-bridge-protocol/src/trace.rs`](../crates/gbn-bridge-protocol/src/trace.rs)

Phase 7 requires explicit `chain_id` coverage for:

- `CreatorJoinRequest`
- `CreatorBootstrapResponse`
- `BridgeSetRequest`
- `BridgeSetResponse`
- `BootstrapJoinReply`
- `BridgeSeedAssign`
- `BridgePunchStart`
- `BridgePunchProbe`
- `BridgePunchAck`
- `BootstrapProgress`
- `BridgeBatchAssign`
- `BridgeOpen`
- `BridgeData`
- `BridgeAck`
- `BridgeClose`
- control-plane hello / command / ack envelopes

## Persistence Coverage

The publisher must persist `chain_id` in durable records required for flow reconstruction. Current required records include:

- bootstrap sessions
- bridge command records
- catalog issuance records where present
- upload sessions
- ingested frame records

This keeps `chain_id` available after restart and during postmortem analysis.

## Service Boundary Rules

- authority API requests and responses carry `chain_id`
- bridge control commands and acks carry `chain_id`
- runtime progress reporting carries `chain_id`
- receiver ingress and ACK generation carry `chain_id`

Any mismatch across a signed or durable boundary is a protocol or authority error, not a warning.

## Validation Surface

Phase 7 validation evidence lives in:

- [`tests/integration/test_chain_id.rs`](../tests/integration/test_chain_id.rs)
- `cargo test --workspace`
- `infra/scripts/mobile-validation.sh`
- `infra/scripts/collect-bridge-metrics.sh`

The local validation script prints the deterministic chain IDs used by the dedicated Phase 7 integration tests:

- `bootstrap-host-creator-01-join-chain-e2e`
- `upload-creator-chain-01-upload-000001`

For AWS/mobile runs, operators should pass the live trace root with `--chain-id` so collected log summaries stay correlated.
