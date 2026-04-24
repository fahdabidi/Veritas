# GBN-PROTO-006 - Conduit Simulation Baseline

**Document ID:** GBN-PROTO-006-SIM-BASELINE  
**Status:** Current-state inventory for the full-implementation track  
**Last Updated:** 2026-04-23  
**Execution Plan:** [GBN-PROTO-006 Conduit Full Implementation - Execution Plan](GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md)  
**Detailed Phase 0 Plan:** [GBN-PROTO-006 Execution Phase 0 - Inventory The Current Conduit Simulation Baseline](GBN-PROTO-006-Execution-Phase0-Conduit-Simulation-Baseline-Inventory.md)  
**Protected V1 Baseline:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)

---

## 1. Purpose

This document records the currently committed Conduit state that `GBN-PROTO-006` starts from.

It is an inventory only:

- it does not freeze the current Conduit simulation
- it does not create a tag or release
- it does not claim the current Conduit implementation is production-capable

Its purpose is to give the full-implementation track a factual baseline for replacing the remaining simulated boundaries.

---

## 2. Starting Point

| Item | Value |
|---|---|
| Starting branch | `main` |
| Starting commit | `2b6d5c5d24e269e96e3fdc820f3f90669607414a` |
| Protected V1 release | `veritas-lattice-0.1.0-baseline` |
| Repo state note | documentation is being reorganized under `docs/prototyping/Conduit/` and `docs/prototyping/Lattice/`; this is visible in the worktree and must not be confused with V1 code drift |

Phase 0 treats the current commit as the starting inventory point for the full-implementation effort.

---

## 3. Current Committed Conduit Modules

The committed Conduit workspace under `prototype/gbn-bridge-proto/` already contains substantial prototype logic:

| Surface | Current State |
|---|---|
| `gbn-bridge-protocol` | wire model exists for bridge descriptors, bootstrap payloads, catalogs, leases, punch flow, and session messages |
| `gbn-bridge-publisher` | authority logic exists for registration, lease/liveness, catalog issuance, bootstrap planning, batching, policy, reachability, ingest, and ACK logic |
| `gbn-bridge-runtime` | creator, host-creator, bridge, bootstrap, fanout, forwarding, session, and reachability logic exist in prototype form |
| `gbn-bridge-cli` | deployment and role binaries exist, but they still include placeholder deployment entrypoint behavior |
| `infra/` | V2-only AWS prototype assets and local scripts exist, but they still describe a prototype deployment shape rather than a full production topology |
| `tests/` | local harness and integration tests exist, but they are still rooted in the prototype implementation rather than a fully distributed deployed system |

This means Conduit is not an empty track. The full-implementation effort starts from an implemented simulation and prototype harness, not from scratch.

---

## 4. Current Runtime And Publisher Boundaries

The key current service-boundary reality is:

### 4.1 Publisher Deployment Boundary

The deployed `bridge-publisher` entrypoint is still a placeholder.

Evidence:

- [`prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/lib.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/lib.rs)
- [`prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/bin/bridge-publisher.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/bin/bridge-publisher.rs)

Current behavior:

- reads environment
- prints configuration summary
- sleeps in a loop under `--serve`
- explicitly says the network protocol service remains placeholder

### 4.2 Publisher Service Boundary

The publisher "server" is still a wrapper around in-process authority state.

Evidence:

- [`prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/server.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/server.rs)

Current behavior:

- owns `PublisherAuthority`
- exposes getters and `into_inner`
- does not expose a real network listener or HTTP/WebSocket service

### 4.3 Runtime Publisher Coupling

The runtime still depends on in-process publisher coupling.

Evidence:

- [`prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/publisher_client.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/publisher_client.rs)

Current behavior:

- `InProcessPublisherClient` stores `PublisherAuthority` in `Rc<RefCell<_>>`
- creator refresh, bootstrap, registration, ingest, close, and progress calls still mutate publisher state directly

### 4.4 Bootstrap Distribution Boundary

The bootstrap path is still driven by local `AuthorityBootstrapPlan` handoff instead of real distributed publisher-owned orchestration.

Evidence:

- [`prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/bootstrap.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/bootstrap.rs)
- [`prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/bootstrap.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/bootstrap.rs)
- [`prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/bootstrap_bridge.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/bootstrap_bridge.rs)
- [`prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/punch_fanout.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/punch_fanout.rs)

### 4.5 Receiver Boundary

The data receiver and ACK flow still exists as publisher library logic, not as a real receiver service boundary.

Evidence:

- [`prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/forwarder.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/forwarder.rs)
- [`prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/ingest.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/ingest.rs)
- [`prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/ack.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/ack.rs)

The committed Conduit state therefore still uses local simulation boundaries for the most critical distributed control and data paths.

---

## 5. Current Deployment Boundaries

The current deployment surface is still prototype-shaped.

### 5.1 Local Compose

Evidence:

- [`prototype/gbn-bridge-proto/docker-compose.bridge-smoke.yml`](../../../prototype/gbn-bridge-proto/docker-compose.bridge-smoke.yml)

Current behavior:

- `busybox` placeholder services for publisher, bridges, host creator, and creator
- manual profile only
- no real network services

### 5.2 AWS Prototype Stack

Evidence:

- [`prototype/gbn-bridge-proto/infra/cloudformation/phase2-bridge-stack.yaml`](../../../prototype/gbn-bridge-proto/infra/cloudformation/phase2-bridge-stack.yaml)
- [`prototype/gbn-bridge-proto/infra/README-infra.md`](../../../prototype/gbn-bridge-proto/infra/README-infra.md)

Current behavior:

- one publisher ECS task family
- one bridge ECS task family
- no separate authority and receiver service split
- no production storage wiring
- no full-stack service discovery model
- README explicitly describes the current binaries as prototype entrypoints rather than production listeners

This means the current Conduit deployment surface is a scaffolding layer, not a production-capable topology.

---

## 6. Current Trace And Observability Boundaries

The current V2 trace / observability state is incomplete.

### 6.1 V2 `chain_id` State

Search result:

- no `chain_id` hits under `prototype/gbn-bridge-proto/`

### 6.2 V1 `chain_id` Reference Points

The V1 baseline already has a distributed trace concept in:

- [`prototype/gbn-proto/crates/mcn-router-sim/src/control.rs`](../../../prototype/gbn-proto/crates/mcn-router-sim/src/control.rs)
- [`prototype/gbn-proto/crates/mcn-router-sim/src/swarm.rs`](../../../prototype/gbn-proto/crates/mcn-router-sim/src/swarm.rs)
- [`prototype/gbn-proto/infra/scripts/relay-control-interactive.sh`](../../../prototype/gbn-proto/infra/scripts/relay-control-interactive.sh)

So the Conduit full-implementation track is not inventing a new trace idea. It is carrying forward an existing V1 concept that is currently absent from V2.

### 6.3 Current Validation Artifact State

Current V2 scripts and docs such as:

- [`prototype/gbn-bridge-proto/infra/scripts/mobile-validation.sh`](../../../prototype/gbn-bridge-proto/infra/scripts/mobile-validation.sh)
- [`prototype/gbn-bridge-proto/infra/scripts/collect-bridge-metrics.sh`](../../../prototype/gbn-bridge-proto/infra/scripts/collect-bridge-metrics.sh)
- [`prototype/gbn-bridge-proto/docs/mobile-test-matrix.md`](../../../prototype/gbn-bridge-proto/docs/mobile-test-matrix.md)

do not yet provide full end-to-end `chain_id` correlation artifacts for Conduit.

---

## 7. Current Validation State

The current Conduit baseline already has:

- a local Rust workspace
- local tests
- integration tests
- AWS/mobile prototype validation scripts

But it does not yet have:

- a real networked publisher authority API
- a real bridge control session service
- a real bootstrap distribution boundary
- a real receiver service boundary
- a real full-stack deployment topology
- full V2 `chain_id` propagation

So the current validation state is best described as:

**implemented and locally exercisable prototype, not full distributed implementation.**

---

## 8. Summary Of What Conduit Is And Is Not Yet

### Conduit Already Is

- a real V2 code workspace
- a structured protocol and runtime prototype
- a meaningful architecture and implementation starting point
- a better-than-empty simulation with real code paths and tests

### Conduit Is Not Yet

- a fully distributed publisher control plane
- a fully networked creator/bridge/publisher system
- a production deployment topology
- a fully traceable `chain_id`-aware implementation
- a promoted replacement for the protected V1 Lattice baseline

This is the baseline state that `GBN-PROTO-006` is intended to replace.

