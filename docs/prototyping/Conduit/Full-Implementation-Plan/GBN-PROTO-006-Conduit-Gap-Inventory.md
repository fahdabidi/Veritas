# GBN-PROTO-006 - Conduit Gap Inventory

**Document ID:** GBN-PROTO-006-GAP-INVENTORY  
**Status:** Current-state production-gap inventory for the full-implementation track  
**Last Updated:** 2026-04-23  
**Execution Plan:** [GBN-PROTO-006 Conduit Full Implementation - Execution Plan](GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md)  
**Detailed Phase 0 Plan:** [GBN-PROTO-006 Execution Phase 0 - Inventory The Current Conduit Simulation Baseline](GBN-PROTO-006-Execution-Phase0-Conduit-Simulation-Baseline-Inventory.md)  
**Protected V1 Baseline:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)

---

## 1. Purpose

This document records the exact production gaps between the currently committed Conduit prototype and the architecture required for a fully distributed implementation.

It is organized by production boundary:

- not by crate count
- not by file count
- not by local harness coverage

Each section states:

- what the architecture requires
- what the current implementation actually does
- why that is insufficient
- which `GBN-PROTO-006` phase should resolve it

---

## 2. Architectural Requirement Summary

The current Conduit architecture requires a system that can:

- expose a real publisher authority service
- persist authoritative state durably
- maintain real bridge control sessions
- use real creator, host-creator, and bridge network clients
- distribute bootstrap seed assignments and bridge-set payloads over the network
- receive forwarded bridge payloads and emit correlated ACKs over a real service boundary
- preserve one distributed `chain_id` end-to-end
- deploy as a real multi-service topology
- prove behavior with distributed harnesses and live AWS/mobile validation

The committed Conduit baseline does not yet satisfy that requirement set.

---

## 3. Publisher Authority API Gap

### Architecture Requires

A real Publisher authority service that receives creator, host-creator, and bridge requests over a network boundary.

### Current Implementation Does

- wraps `PublisherAuthority` in a thin local `AuthorityServer`
- deploys a placeholder `bridge-publisher` process
- uses `InProcessPublisherClient` from runtime code

Key evidence:

- [`prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/server.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/server.rs)
- [`prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/lib.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/lib.rs)
- [`prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/publisher_client.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/publisher_client.rs)

### Why It Is Insufficient

There is no real service boundary, authentication envelope, or request/response path.

### Resolving Phase

- `Phase 1`

---

## 4. Durable Storage And Restart Recovery Gap

### Architecture Requires

Durable publisher storage for bridge state, leases, bootstrap sessions, upload sessions, progress, and recovery after restart.

### Current Implementation Does

- uses in-memory authority storage for production-path logic
- does not define a production database, migration, or restart recovery model

### Why It Is Insufficient

State is not durable and cannot safely survive process restart or deployed-service recovery.

### Resolving Phase

- `Phase 2`

---

## 5. Bridge Control-Session Gap

### Architecture Requires

A real publisher-to-bridge command delivery path for:

- bootstrap seed assignment
- fanout activation
- refresh / revoke style control messages

### Current Implementation Does

- computes authoritative commands
- but does not maintain real authenticated long-lived bridge control sessions

### Why It Is Insufficient

The Publisher cannot actively distribute instructions to bridges over real service boundaries.

### Resolving Phase

- `Phase 3`

---

## 6. Creator / Host-Creator / Bridge Network Client Gap

### Architecture Requires

Real network clients for creator, host-creator, and bridge interactions with the Publisher.

### Current Implementation Does

- uses `InProcessPublisherClient`
- keeps first-contact and refresh behavior bound to local object interaction

### Why It Is Insufficient

The runtime still bypasses the distributed service boundary it is supposed to exercise.

### Resolving Phase

- `Phase 4`

---

## 7. Real Bootstrap Distribution And Fanout Gap

### Architecture Requires

A publisher-owned bootstrap session that:

- selects `ExitBridgeB`
- distributes seed assignment
- returns bootstrap response through the relay path
- distributes the bridge set
- activates remaining bridges for fanout
- tracks timeouts, retries, and reassignment

### Current Implementation Does

- returns `AuthorityBootstrapPlan`
- injects assignment state into the seed bridge locally
- tracks fanout in local runtime state

### Why It Is Insufficient

Bootstrap is still simulation-shaped rather than publisher-distributed.

### Resolving Phase

- `Phase 5`

---

## 8. Real Receiver / ACK Path Gap

### Architecture Requires

A real publisher receiver service that:

- accepts bridge-forwarded payloads
- tracks session open/data/close
- emits correlated ACKs

### Current Implementation Does

- forwards payload frames through `InProcessPublisherClient`
- exposes ingest and ACK logic as local library functions

### Why It Is Insufficient

The bridge-to-publisher data path still does not cross a real service boundary.

### Resolving Phase

- `Phase 6`

---

## 9. Distributed `chain_id` Propagation Gap

### Architecture Requires

One root `chain_id` that survives:

- creator
- host creator
- bridges
- authority
- receiver
- ACK path
- bootstrap fanout
- persistence
- logs and validation artifacts

### Current Implementation Does

- has no effective V2 `chain_id` propagation today
- still relies on V1 for the actual field and tooling precedent

Key V1 references:

- [`prototype/gbn-proto/crates/mcn-router-sim/src/control.rs`](../../../prototype/gbn-proto/crates/mcn-router-sim/src/control.rs)
- [`prototype/gbn-proto/crates/mcn-router-sim/src/swarm.rs`](../../../prototype/gbn-proto/crates/mcn-router-sim/src/swarm.rs)

### Why It Is Insufficient

Conduit currently lacks the distributed trace field required for production debugging, validation, and observability.

### Resolving Phase

- `Phase 7`

---

## 10. Deployment Image And AWS Topology Gap

### Architecture Requires

A real deployable topology with:

- authority service
- receiver service
- bridge services
- durable store
- secrets/config
- service discovery or equivalent stable internal routing

### Current Implementation Does

- uses one monolithic `bridge-publisher` image
- uses a prototype `phase2-bridge-stack.yaml`
- uses placeholder local smoke compose services

### Why It Is Insufficient

The deployment topology still reflects the prototype implementation rather than the full distributed design.

### Resolving Phase

- `Phase 8`

---

## 11. Distributed Test-Harness Gap

### Architecture Requires

A real distributed e2e harness with:

- first-contact bootstrap
- refresh
- data path
- failover
- restart recovery
- fault injection
- `chain_id` continuity assertions

### Current Implementation Does

- has local integration tests
- has smoke scripts
- does not yet have a dedicated distributed full-system harness

### Why It Is Insufficient

The current validation surface is still too local and too prototype-shaped to prove the final distributed implementation.

### Resolving Phase

- `Phase 9`

---

## 12. Live Validation Gap

### Architecture Requires

Live AWS and mobile-network evidence for:

- bootstrap
- punch success
- forwarding and ACK behavior
- churn / failover
- batch behavior
- `chain_id` continuity

### Current Implementation Does

- provides prototype AWS/mobile tooling and docs
- explicitly documents the current deployed binaries as prototype-only

### Why It Is Insufficient

The current repo does not yet have full implementation evidence under live conditions.

### Resolving Phase

- `Phase 10`

---

## 13. Recommended Remediation Order

The correct replacement order remains:

1. Phase 0: inventory the simulation baseline
2. Phase 1: real publisher authority API
3. Phase 2: durable storage and recovery
4. Phase 3: bridge control sessions
5. Phase 4: runtime network clients
6. Phase 5: real bootstrap distribution and fanout
7. Phase 6: real receiver and ACK path
8. Phase 7: full `chain_id` propagation
9. Phase 8: real deployment images and AWS control plane
10. Phase 9: distributed e2e harness and fault injection
11. Phase 10: live AWS and mobile validation
12. Phase 11: decision gate

This order is correct because it:

- establishes real service boundaries first
- makes bootstrap and data flow real before deployment claims
- adds full trace propagation before live evidence gathering
- postpones the final decision until evidence exists

---

## 14. Exit Criteria For Phase 0

Phase 0 is complete only when:

- the current committed Conduit baseline is described factually
- the current production gaps are explicitly listed
- the current absence of V2 `chain_id` propagation is recorded
- the remediation order is explicitly stated
- no Conduit simulation freeze or release artifact is created

At that point, Phase 1 can begin against a documented starting state instead of assumptions.
