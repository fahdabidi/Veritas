# GBN-PROTO-013 - Execution Phase 1 - Bootstrap Hardening And Validation

**Status:** Complete
**Last Updated:** 2026-05-11
**Parent Plan:** [GBN-PROTO-013](GBN-PROTO-013-Conduit-Mobile-Creator-Public-Internet-Validation-Execution-Plan.md)
**Depends On:** Pass 3 local k8s bootstrap/upload baseline

## Objective

Harden the existing Pass 3 bootstrap path so it matches the README first-time creator
bootstrap target before any mobile app work enters the live network path.

This phase closes the prototype shortcuts that are acceptable for in-cluster validation
but unsafe to carry into mobile validation:

- no separate Publisher ingest into the NewCreator before first-time bootstrap;
- no cleartext Publisher bootstrap payload readable by HostCreator or relay bridges;
- no direct marking of all bridge entries as active without real bridge progress;
- no full bridge set delivered directly in the initial NewCreator bootstrap response.

At completion:

- NewCreator starts with only HostCreator DHT seed material.
- NewCreator sends its DHT entry and public key to HostCreator.
- HostCreator relays that entry request to Publisher over the existing bridge path.
- Publisher encrypts the bootstrap payload to the NewCreator public key.
- HostCreator and relay bridges forward opaque bootstrap bytes only.
- NewCreator first receives Publisher public key/DHT plus Seed ExitBridgeB DHT.
- Seed ExitBridgeB returns the signed remaining bridge catalog.
- Remaining ExitBridges receive NewCreator DHT and establish tunnels before they are
  marked active.
- Bootstrap validation and SendDummy validation both pass against local k8s.

Update the parent plan status tracker when this phase is complete.

---

## Required Flow

The hardened flow must follow the README target:

1. `NewCreator` pairs with `HostCreator`.
2. `NewCreator` sends its DHT entry and public key to `HostCreator`.
3. `HostCreator` relays the entry request to Publisher through its existing bridge path.
4. Publisher creates a signed bootstrap payload containing:
   - NewCreator entry;
   - Publisher public key;
   - Publisher DHT entry;
   - Seed ExitBridgeB DHT entry.
5. Publisher encrypts that bootstrap payload to the NewCreator public key.
6. Publisher seeds ExitBridgeB with the remaining bridge DHT set.
7. The encrypted bootstrap payload returns through the existing path:
   `Publisher -> ExitBridgeA -> HostCreator -> NewCreator`.
8. NewCreator decrypts the payload and stores Publisher + Seed ExitBridgeB DHT state.
9. NewCreator and ExitBridgeB establish the seed tunnel and report progress.
10. NewCreator requests the bridge catalog from ExitBridgeB.
11. ExitBridgeB returns the signed remaining bridge catalog.
12. Publisher fans out NewCreator DHT to the remaining ExitBridges.
13. Remaining ExitBridges establish tunnels with NewCreator and report progress.
14. NewCreator marks each bridge active only after corresponding progress is observed.
15. Every step preserves the same `chain_id`.

---

## Implementation Requirements

### Encrypted Bootstrap Payload

Add a protocol shape for the Publisher bootstrap payload that can be encrypted to the
NewCreator public key. The payload must include:

- `chain_id`;
- `bootstrap_session_id`;
- NewCreator entry;
- Publisher public key;
- Publisher DHT entry;
- Seed ExitBridgeB DHT entry;
- payload expiry;
- Publisher signature over the plaintext before encryption, or equivalent authenticated
  envelope metadata.

HostCreator and ExitBridgeA must only forward opaque bootstrap bytes. They may route,
store evidence hashes, and preserve ChainID metadata, but they must not deserialize the
Publisher payload contents.

### Seed ExitBridgeB Catalog Handoff

The initial payload must not deliver all bridge DHT entries directly to NewCreator.
Publisher seeds ExitBridgeB with the remaining bridge DHT set, and NewCreator receives
that set only after the Seed ExitBridgeB tunnel is established.

Required evidence:

- Publisher queued seed assignment for ExitBridgeB.
- ExitBridgeB ACKed the seed assignment.
- NewCreator established a seed tunnel with ExitBridgeB.
- ExitBridgeB returned the signed bridge catalog to NewCreator.
- NewCreator local DHT contains the Seed ExitBridgeB entry before the remaining bridge
  entries.

### Remaining Bridge Fanout

The existing `BridgeBatchAssign` concept is retained, but active state must be driven by
real per-bridge progress:

- Publisher sends NewCreator DHT to each remaining ExitBridge.
- Each remaining ExitBridge starts tunnel establishment with NewCreator.
- NewCreator marks a bridge active only after the bridge-specific progress/ACK path is
  observed.
- Partial fanout may still be allowed, but it must be explicit: `fanout_partial` with
  active count and missing bridge ids.

### Compatibility

Existing Pass 3 HTTP/admin surfaces remain available:

- `/v1/admin/seed-host-creator`
- `/v1/admin/seed-new-creator`
- `/v1/admin/start-bootstrap`
- `/v1/admin/local-dht`
- `/v1/admin/send-dummy`

Response shapes may add fields for strict bootstrap evidence, but existing fields used by
Pass 3 operator scripts must remain compatible.

---

## Validation

Run from WSL2 Ubuntu.

### Bootstrap Validation

Add or update a strict bootstrap smoke that proves the hardened flow:

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

The strict bootstrap validation must archive:

- HostCreator seed result.
- NewCreator join request evidence, including NewCreator public key.
- Publisher encrypted bootstrap payload metadata, hash, and ChainID.
- Evidence that HostCreator/ExitBridgeA forwarded opaque bytes only.
- Seed ExitBridgeB assignment and ACK.
- Seed tunnel progress from NewCreator and ExitBridgeB.
- Seed ExitBridgeB bridge-catalog response.
- Remaining bridge fanout assignments and per-bridge progress.
- NewCreator local DHT progression.
- Final DHT agreement between Publisher session and NewCreator local DHT.
- ChainID logs across NewCreator, HostCreator, Publisher, Seed ExitBridgeB, and remaining
  ExitBridges.

### SendDummy Validation

After strict bootstrap succeeds, run SendDummy against the same onboarded NewCreator:

```bash
cd prototype/gbn-bridge-proto
infra/scripts/k8s-smoke-senddummy-strict-v4.sh \
  --require-observability \
  --require-onboarded-from-strict-bootstrap \
  --require-route-source local_dht \
  --require-ciphertext-only-at-bridge
```

The SendDummy validation must prove:

- NewCreator is onboarded from the hardened bootstrap run.
- Route selection uses local DHT entries learned through the hardened bootstrap path.
- Dummy frame is encrypted for Publisher before crossing any bridge.
- Selected ExitBridge sees ciphertext only.
- Publisher receiver accepts and validates the frame.
- Result includes `route_source=local_dht`, assigned bridge id, and ChainID.
- Forced bridge-failure variant still reroutes using active local DHT entries.

---

## Tests

Add focused Rust tests for:

- Publisher encrypts bootstrap payload to NewCreator public key.
- HostCreator relay rejects attempts to inspect or mutate encrypted payload contents.
- NewCreator rejects bootstrap payloads that are not encrypted to its public key.
- NewCreator stores only Publisher + Seed ExitBridgeB DHT before seed-catalog handoff.
- Seed ExitBridgeB serves the signed remaining bridge catalog after seed tunnel progress.
- Remaining ExitBridge fanout does not mark bridges active until per-bridge progress is
  observed.
- Existing Pass 3 admin endpoints remain backward compatible.

Run:

```bash
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
```

---

## Acceptance Criteria

- Hardened bootstrap uses no separate Publisher ingest into NewCreator.
- Publisher bootstrap payload is encrypted to NewCreator public key.
- HostCreator and ExitBridgeA forward opaque bootstrap bytes only.
- Initial bootstrap payload contains Publisher public key/DHT and Seed ExitBridgeB DHT,
  not the full bridge set.
- Seed ExitBridgeB returns the signed bridge catalog after tunnel establishment.
- Remaining ExitBridges receive NewCreator DHT and report progress before NewCreator
  marks them active.
- Strict local k8s bootstrap validation passes and archives evidence.
- Strict local k8s SendDummy validation passes and archives evidence.
- Pass 3 Smoke 1 through Smoke 4 continue to pass or are superseded by documented strict
  equivalents with the same evidence standard.
- V1 preservation checks return no files.

---

## Completion Evidence

When this phase is implemented, archive:

- Rust test output.
- Strict bootstrap validation report.
- Strict SendDummy validation report.
- NewCreator local DHT progression.
- Publisher bootstrap-session dump.
- Seed ExitBridgeB command/evidence dump.
- Remaining ExitBridge fanout progress dump.
- ChainID trace bundle.
- Pass 3 compatibility output.
- V1 preservation command output.
