# Veritas Conduit Test Vectors

This directory documents the deterministic assumptions used by the Phase 9 local harness.

## Scope

The Phase 9 harness is a Conduit-local integration layer. It exercises committed Phase 3 through Phase 8 behavior across:

- bridge registration
- catalog refresh
- first-time creator bootstrap
- UDP punch ACK correlation
- batch onboarding
- creator failover
- insufficient-fanout reuse
- payload confidentiality boundaries
- reachability filtering

It does not replace:

- focused per-crate tests
- AWS deployment validation
- mobile-network validation

## Deterministic Inputs

The harness uses fixed deterministic values so failures are reproducible:

- signing-key seeds are fixed one-byte arrays expanded to 32 bytes
- bridge IDs use readable stable strings such as `bridge-a`, `bridge-seed`, and `bridge-brokered`
- creator IDs use readable stable strings such as `creator-refresh` and `creator-bootstrap`
- punch ports default to `443` unless a scenario explicitly validates a signed port transition
- timestamps are monotonic synthetic integers such as `1_000`, `2_000`, and `5_000`

## Confidentiality Boundary

The current Conduit prototype treats `BridgeData.ciphertext` as opaque bridge payload.

Phase 9 confidentiality assertions prove only that:

- creator-generated framed payload is forwarded unchanged by bridges
- publisher ingest receives the same opaque framed payload
- bridges do not need clear-payload awareness to relay frames

Phase 9 does not claim stronger cryptographic properties than the current implementation provides.

## Smoke Harness

The local smoke entrypoint is:

```bash
bash prototype/gbn-bridge-proto/infra/scripts/run-local-bridge-tests.sh
```

The docker-compose file is a V2-local smoke scaffold:

```bash
docker compose -f prototype/gbn-bridge-proto/docker-compose.bridge-smoke.yml config
```

It exists to keep the topology shape visible and separate from V1 assets. It is not the Phase 10 AWS deployment artifact.
