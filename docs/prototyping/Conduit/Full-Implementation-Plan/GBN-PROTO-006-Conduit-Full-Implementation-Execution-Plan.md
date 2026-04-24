# GBN-PROTO-006 - Conduit Full Implementation - Execution Plan

**Document ID:** GBN-PROTO-006  
**Status:** Draft - Phase 0 complete, Phase 1 complete, Phase 2 complete, Phase 3 complete, Phase 4 complete, Phase 5 complete, Phase 6 complete, Phase 7 complete, Phase 8 implementation complete with external deployment revalidation deferred to Phase 10, Phase 9 complete, Phase 10 implemented locally with live AWS/mobile evidence pending
**Last Updated:** 2026-04-24
**Related Docs:** [GBN-PROTO-005 Execution Plan](GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign-Execution-Plan.md), [GBN-ARCH-000-V2](../architecture/GBN-ARCH-000-System-Architecture-V2.md), [GBN-ARCH-001-V2](../architecture/GBN-ARCH-001-Media-Creation-Network-V2.md), [GBN-ARCH-002-V2](../architecture/GBN-ARCH-002-Bridge-Protocol-V2.md)

This document defines the follow-on execution plan that upgrades the current Conduit implementation from a local simulation and harness-oriented prototype into a fully fledged distributed implementation.

The starting point for this plan is the currently committed Conduit state from `GBN-PROTO-005`, where:

- the wire model exists
- the authority logic exists
- the creator, bridge, and publisher flows exist in prototype form
- AWS/mobile tooling exists in partial form
- but critical production boundaries remain simulated or stubbed

In particular, this plan replaces:

- in-process publisher coupling
- placeholder deployment entrypoints and images
- in-memory authority state as the production default
- simulated bootstrap distribution
- simulated bridge-to-publisher forwarding

It also adds the V1-style distributed trace `chain_id` across all production code paths, nodes, workers, logs, and validation artifacts.

## Status Trackers

- `[ ]` Pending
- `[/]` In Progress
- `[x]` Completed

---

## 1. Execution Rules

### 1.1 Phase Sequencing Rule

Every phase must finish with:

- all phase-specific validation tests passing
- the required V1 regression suite passing
- a clean review of forbidden V1 file paths showing no changes

No later phase may begin until the current phase has been explicitly approved.

### 1.2 V1 Preservation Rule

This full-implementation track must preserve the published V1 Lattice baseline exactly unless a separate explicit V1 request is approved.

The V1 no-touch paths from `GBN-PROTO-005` remain in effect:

- `prototype/gbn-proto/Cargo.toml`
- `prototype/gbn-proto/Cargo.lock`
- `prototype/gbn-proto/crates/gbn-protocol/**`
- `prototype/gbn-proto/crates/mcn-crypto/**`
- `prototype/gbn-proto/crates/mcn-router-sim/**`
- `prototype/gbn-proto/crates/mpub-receiver/**`
- `prototype/gbn-proto/crates/proto-cli/**`
- `prototype/gbn-proto/Dockerfile.relay`
- `prototype/gbn-proto/Dockerfile.publisher`
- `prototype/gbn-proto/docker-compose.scale-test.yml`
- `prototype/gbn-proto/infra/cloudformation/**`
- `prototype/gbn-proto/infra/scripts/**`
- `prototype/gbn-proto/tests/integration/**`
- `docs/prototyping/GBN-PROTO-004-*`
- `docs/architecture/GBN-ARCH-000-System-Architecture.md`
- `docs/architecture/GBN-ARCH-001-Media-Creation-Network.md`

### 1.3 Current Conduit Baseline Rule

The committed `GBN-PROTO-005` Conduit state is the starting baseline for this plan.

That baseline may be refactored inside `prototype/gbn-bridge-proto/`, but it must not be silently discarded. Any simulated boundary being replaced must be:

- identified explicitly
- replaced with a real distributed boundary
- covered by new tests
- documented as retired from production paths

### 1.4 Real Service Boundary Rule

No phase may claim completion for the full implementation if the production path still depends on:

- `InProcessPublisherClient`
- placeholder deployment binaries
- placeholder Docker images
- placeholder docker-compose smoke services
- in-memory-only authority storage for the production mode

Simulation paths may remain only if they are clearly restricted to local tests, local harnesses, or explicit dev-only modes.

### 1.5 Publisher Responsibility Rule

The Publisher must be implemented as a real distributed control-plane service that is capable of:

- receiving creator, host-creator, and bridge requests over a network boundary
- persisting authoritative bridge, lease, bootstrap, catalog, and progress state
- actively distributing signed bootstrap and fanout instructions to bridges
- coordinating first-contact bootstrap and ongoing refresh state
- receiving bridge-forwarded payloads and issuing ACKs over real service paths

A library-only or in-process publisher implementation does not satisfy this rule.

### 1.6 ChainID Trace Continuity Rule

The V1 distributed trace concept must be carried into the Conduit full implementation using the same field name: `chain_id`.

Rules:

- do not replace `chain_id` with a differently named root correlation field
- every creator-originated upload or bootstrap flow must originate or carry a root `chain_id`
- `chain_id` must be propagated across:
  - `HostCreator`
  - `ExitBridgeA`
  - `PublisherAuthority`
  - `ExitBridgeB`
  - remaining bootstrap bridges
  - `PublisherReceiver`
  - ACK and progress-report flows
- `chain_id` must appear in:
  - protocol messages where correlation is required
  - service logs
  - metrics labels or structured fields where practical
  - persistent bootstrap / session records
  - local and AWS/mobile validation artifacts

### 1.7 README Freeze Rule

Do not modify the main repo `README.md` during this implementation track unless there is a separate explicit documentation approval after the code work is complete.

Keep `README.md` pinned to the current release-facing Lattice/Conduit coexistence messaging until the full implementation and validation work is complete.

### 1.8 Prompt Design Rule

Every phase prompt in this document must be usable by a fresh agent with little or no prior context.

That means each prompt must be self-contained about:

- the repo split between V1 and V2
- the published V1 Lattice baseline that must be preserved
- the current Conduit simulation baseline that is being upgraded
- the exact phase scope
- the allowed and forbidden file paths
- the required validation commands
- the `chain_id` propagation rule
- the rule that the agent must stop after the current phase and wait for approval

---

## 2. Validation Baseline

Each phase below lists its own validation tests. The following baseline suites are reused across phases.

### 2.1 V1 File Integrity Check

Run a path-scoped diff before and after each phase:

```bash
git diff --name-only -- \
  prototype/gbn-proto \
  docs/prototyping/GBN-PROTO-004-Phase2-Serverless-Scale-Onion-Plan.md \
  docs/prototyping/GBN-PROTO-004-Phase2-Serverless-Scale-Test.md \
  docs/architecture/GBN-ARCH-000-System-Architecture.md \
  docs/architecture/GBN-ARCH-001-Media-Creation-Network.md
```

Expected result:

- no output for forbidden V1 paths

### 2.2 Minimum V1 Code Regression Suite

Run from `prototype/gbn-proto/`:

```bash
cargo check --workspace
cargo test -p mcn-router-sim
```

### 2.3 Extended V1 Local Regression Suite

Run when a phase introduces new local dev tooling, Docker assets, or shared infrastructure conventions:

```bash
bash validate-scale-test.sh
```

### 2.4 Extended V1 AWS Regression Suite

Run when a phase introduces or materially changes AWS deployment assets:

```bash
bash infra/scripts/run-tests.sh <v1-stack-name> <region>
```

### 2.5 V2 Workspace Sanity Suite

Run from `prototype/gbn-bridge-proto/`:

```bash
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
```

If OneDrive-backed local writes still fail for `target/`, use the documented temp-target fallback and record the exact target directory used.

### 2.6 Simulation Retirement Check

For any phase that claims a production boundary has been replaced, verify:

- the production path no longer calls `InProcessPublisherClient`
- the relevant CLI entrypoint is no longer a placeholder
- the relevant deployment image no longer runs a placeholder process
- the relevant test suite exercises the real networked boundary

### 2.7 Distributed Trace Verification Check

For any phase that introduces or replaces a request path, verify:

- `chain_id` is present in the initiating request or is generated there
- `chain_id` is preserved through every hop in that phase's scope
- logs and test assertions can correlate events using the same `chain_id`

---

## 3. Phase 0 - Inventory The Current Conduit Simulation Baseline

### 3.1 Objective

Record and assess the currently committed Conduit simulation as the explicit starting point that this full-implementation track will replace.

### 3.2 Files To Create Or Modify

Create:

- `docs/prototyping/GBN-PROTO-006-Conduit-Simulation-Baseline.md`
- `docs/prototyping/GBN-PROTO-006-Conduit-Gap-Inventory.md`

May modify:

- this execution plan document
- V2-only architecture / prototyping docs

Must not modify:

- any path under `prototype/gbn-proto/`

### 3.3 Deliverables

- a current-state inventory for the committed Conduit simulation
- an explicit inventory of remaining simulated boundaries, including:
  - publisher authority API
  - bridge control dispatch
  - bootstrap distribution
  - bridge-to-publisher forwarding
  - deployment entrypoints and images
- a starting inventory of where `chain_id` does and does not exist in V2
- a recommended remediation order for replacing the simulated production boundaries

### 3.4 Validation Tests

- V1 file integrity check passes
- minimum V1 code regression suite passes
- V2 workspace sanity suite passes
- current-state inventory explicitly names every known simulated production boundary
- no release, tag, or publication artifact is required for the simulation baseline

### 3.5 V1 Preservation Instructions

- do not modify any V1 source file
- do not alter the published Lattice release reference
- do not create a GitHub release, tag, or freeze artifact for the current Conduit simulation

### 3.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 0: Inventory The Current Conduit Simulation Baseline. Do not proceed to the next phase 1 without getting explicit approval.

Create documentation-only artifacts that record the current committed Conduit implementation as the starting simulation state and inventory the exact production gaps that remain. Explicitly identify in-process publisher coupling, placeholder deployment entrypoints, placeholder images, in-memory-only production state, and missing distributed chain_id propagation. Do not create a release, tag, or freeze artifact for the Conduit simulation baseline in this phase.

Do not modify any file under prototype/gbn-proto/. Do not change the main repo README.md. Keep README.md pinned to the current release-facing content and defer any README updates until all code changes in this full-implementation track are complete and explicitly approved.
```

### 3.7 Detailed Execution Reference

Use [GBN-PROTO-006-Execution-Phase0-Conduit-Simulation-Baseline-Inventory](GBN-PROTO-006-Execution-Phase0-Conduit-Simulation-Baseline-Inventory.md) as the implementation checklist for this phase. It expands the Phase 0 scope into current repo findings, required evidence capture, inventory axes, validation gates, acceptance criteria, and sign-off rules.

---

## 4. Phase 1 - Real Publisher Authority API Service

### 4.1 Objective

Replace the library-only publisher boundary with a real networked authority API service.

### 4.2 Files To Create Or Modify

Create:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/api.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/config.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/auth.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/http.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/service.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/tests/api_flow.rs`

May modify:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/lib.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/server.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/bin/bridge-publisher.rs`

Must not modify:

- `prototype/gbn-proto/**`

### 4.3 Deliverables

- a real publisher authority listener for:
  - bridge registration
  - lease renewal / heartbeat
  - creator catalog refresh
  - host-creator bootstrap join requests
  - progress reporting
- authenticated request handling
- structured error responses
- `chain_id` support for every authority request and response path that participates in a creator flow

### 4.4 Validation Tests

- local network integration tests for registration, refresh, and join requests
- auth rejection tests
- malformed request tests
- `chain_id` presence and continuity tests on authority request/response logs
- V2 workspace sanity suite
- V1 file integrity check passes
- minimum V1 code regression suite passes

### 4.5 V1 Preservation Instructions

- do not modify `mpub-receiver`
- do not reuse V1 publisher APIs by editing them in place

### 4.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 1: Real Publisher Authority API Service. Do not proceed to the next phase 2 without getting explicit approval.

Implement a real networked publisher authority API in gbn-bridge-publisher. Replace the library-only boundary with a real authenticated service that can accept bridge registration, lease renewal, creator refresh, host-creator bootstrap join requests, and progress reports over a network boundary. Ensure every creator-initiated or bootstrap-related request path carries chain_id.

Do not modify any file under prototype/gbn-proto/. Do not modify the main repo README.md during this phase.
```

### 4.7 Detailed Execution Reference

Use [GBN-PROTO-006-Execution-Phase1-Real-Publisher-Authority-API-Service](GBN-PROTO-006-Execution-Phase1-Real-Publisher-Authority-API-Service.md) as the implementation checklist for this phase. It expands the Phase 1 scope into current repo findings, service-boundary and auth decisions, module ownership, dependency policy, validation gates, acceptance criteria, and sign-off rules.

---

## 5. Phase 2 - Durable Publisher Storage And Recovery

### 5.1 Objective

Replace in-memory production authority state with durable storage and restart recovery.

Aurora/Postgres is the default target for this plan because the publisher authority owns multi-entity bootstrap sessions, assignments, progress state, and audit history that benefit from transactional consistency.

### 5.2 Files To Create Or Modify

Create:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/storage/postgres.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/storage/schema.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/storage/recovery.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/signing/kms.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/tests/persistence_recovery.rs`

May modify:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/storage.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/authority.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/catalog.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/bootstrap.rs`

### 5.3 Deliverables

- durable storage for:
  - bridge registry
  - leases
  - signed descriptors and catalogs
  - bootstrap sessions
  - bridge assignments
  - progress events
  - upload session state
- restart recovery logic
- signing-key loading strategy for production mode
- `chain_id` persisted on bootstrap, progress, and session records

### 5.4 Validation Tests

- storage round-trip tests
- restart recovery tests
- bootstrap session recovery tests
- duplicate / replay protection tests
- `chain_id` persistence tests
- V2 workspace sanity suite
- V1 file integrity check passes
- minimum V1 code regression suite passes

### 5.5 V1 Preservation Instructions

- do not alter V1 storage or publisher key handling

### 5.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 2: Durable Publisher Storage And Recovery. Do not proceed to the next phase 3 without getting explicit approval.

Replace in-memory publisher authority state for the production path with durable storage and restart recovery. Persist bridge registry, leases, catalogs, bootstrap sessions, assignments, progress state, and upload session metadata. Persist chain_id on every creator/bootstrap/session record that needs distributed correlation.

Do not modify any file under prototype/gbn-proto/. Do not modify the main repo README.md during this phase.
```

### 5.7 Detailed Execution Reference

Use [GBN-PROTO-006-Execution-Phase2-Durable-Publisher-Storage-And-Recovery](GBN-PROTO-006-Execution-Phase2-Durable-Publisher-Storage-And-Recovery.md) as the implementation checklist for this phase. It expands the Phase 2 scope into current repo findings, storage and recovery decisions, schema and repository boundaries, `chain_id` persistence rules, validation gates, acceptance criteria, and sign-off rules.

---

## 6. Phase 3 - Bridge Control Sessions And Command Delivery

### 6.1 Objective

Implement real authenticated bridge-to-publisher control sessions so the Publisher can actively distribute bootstrap and fanout instructions.

### 6.2 Files To Create Or Modify

Create:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/control.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/dispatcher.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/assignment.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/control_client.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/tests/control_session.rs`

May modify:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/bridge.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/bootstrap.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/batching.rs`

### 6.3 Deliverables

- long-lived authenticated bridge control sessions
- publisher push delivery for:
  - seed assignment
  - bridge batch assignment
  - punch directives
  - revocations
  - descriptor refresh notifications
- reconnect and resume behavior
- `chain_id` propagation through all assignment and command paths

### 6.4 Validation Tests

- bridge connect / reconnect tests
- assignment delivery and ACK tests
- stale-session and revoked-bridge rejection tests
- `chain_id` continuity tests across command dispatch and progress ACKs
- V2 workspace sanity suite
- V1 file integrity check passes
- minimum V1 code regression suite passes

### 6.5 V1 Preservation Instructions

- do not modify V1 relay control or router behavior

### 6.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 3: Bridge Control Sessions And Command Delivery. Do not proceed to the next phase 4 without getting explicit approval.

Implement real authenticated bridge control sessions so the Publisher can push bootstrap seed assignments, fanout assignments, punch directives, and revocations to ExitBridges over a real networked boundary. Ensure chain_id is preserved across every command and progress-report path.

Do not modify any file under prototype/gbn-proto/. Do not modify the main repo README.md during this phase.
```

### 6.7 Detailed Execution Reference

Use [GBN-PROTO-006-Execution-Phase3-Bridge-Control-Sessions-And-Command-Delivery](GBN-PROTO-006-Execution-Phase3-Bridge-Control-Sessions-And-Command-Delivery.md) as the implementation checklist for this phase. It expands the Phase 3 scope into current repo findings, control-session and command-envelope decisions, module ownership, dependency policy, validation gates, acceptance criteria, and sign-off rules.

---

## 7. Phase 4 - Replace In-Process Clients With Network Clients

### 7.1 Objective

Remove in-process publisher coupling from creator, host-creator, and bridge production paths.

### 7.2 Files To Create Or Modify

Create:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/publisher_api_client.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/host_creator_client.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/network_transport.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/tests/network_client_flow.rs`

May modify:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/publisher_client.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/bootstrap.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/host_creator.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/creator.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/progress_reporter.rs`

### 7.3 Deliverables

- real network clients for authority API calls
- real host-creator relay path to Publisher
- production-path retirement of `InProcessPublisherClient`
- explicit dev-only gating for any remaining in-process simulation path
- `chain_id` generation or forwarding at creator / host-creator entry

### 7.4 Validation Tests

- network-client replacement tests
- explicit check that production entrypoints do not instantiate `InProcessPublisherClient`
- host-creator join relay tests
- `chain_id` continuity tests from creator through host creator to publisher
- simulation retirement check passes
- V2 workspace sanity suite
- V1 file integrity check passes
- minimum V1 code regression suite passes

### 7.5 V1 Preservation Instructions

- do not modify V1 CLI or V1 runtime code

### 7.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 4: Replace In-Process Clients With Network Clients. Do not proceed to the next phase 5 without getting explicit approval.

Replace in-process publisher coupling in the creator, host-creator, and bridge production paths with real network clients. If any in-process path is retained for local simulation or tests, isolate it as dev-only and ensure the production path no longer depends on it. Ensure the creator or host-creator establishes or forwards chain_id at the start of every bootstrap or upload flow.

Do not modify any file under prototype/gbn-proto/. Do not modify the main repo README.md during this phase.
```

### 7.7 Detailed Execution Reference

Use [GBN-PROTO-006-Execution-Phase4-Replace-In-Process-Clients-With-Network-Clients](GBN-PROTO-006-Execution-Phase4-Replace-In-Process-Clients-With-Network-Clients.md) as the implementation checklist for this phase. It expands the Phase 4 scope into current runtime findings, client-replacement decisions, module ownership, dependency policy, validation gates, acceptance criteria, and sign-off rules.

---

## 8. Phase 5 - Real Bootstrap Distribution And Fanout

### 8.1 Objective

Implement the full publisher-directed bootstrap distribution path required by the V2 architecture.

### 8.2 Files To Create Or Modify

Create:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/bootstrap/session.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/bootstrap/distribution.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/bootstrap/fanout.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/tests/bootstrap_distribution.rs`

May modify:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/bootstrap.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/dispatcher.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/bootstrap.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/bootstrap_bridge.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/punch_fanout.rs`

### 8.3 Deliverables

- publisher-created bootstrap session state machine
- real seed assignment to `ExitBridgeB`
- real bridge-set payload delivery for the 9 bridge entries
- real fanout activation to the remaining bridges
- timeout, retry, and reassignment rules
- `chain_id` continuity across:
  - host creator join
  - publisher selection
  - seed assignment
  - tunnel-up reporting
  - remaining bridge fanout

### 8.4 Validation Tests

- first-time bootstrap end-to-end tests over real service boundaries
- timeout and reassignment tests
- bridge-set delivery tests
- 10-request batch and 11th-request rollover tests
- `chain_id` continuity assertions across the whole bootstrap session
- V2 workspace sanity suite
- V1 file integrity check passes
- minimum V1 code regression suite passes

### 8.5 V1 Preservation Instructions

- do not alter V1 onion bootstrap or DHT logic

### 8.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 5: Real Bootstrap Distribution And Fanout. Do not proceed to the next phase 6 without getting explicit approval.

Implement the full publisher-directed bootstrap distribution flow required by the V2 architecture. The Publisher must create a bootstrap session, push a seed assignment to ExitBridgeB, return the bootstrap response through the existing path, activate remaining bridges for fanout, and track progress over real service boundaries. Preserve chain_id across the entire bootstrap session.

Do not modify any file under prototype/gbn-proto/. Do not modify the main repo README.md during this phase.
```

### 8.7 Detailed Execution Reference

Use [GBN-PROTO-006-Execution-Phase5-Real-Bootstrap-Distribution-And-Fanout](GBN-PROTO-006-Execution-Phase5-Real-Bootstrap-Distribution-And-Fanout.md) as the implementation checklist for this phase. It expands the Phase 5 scope into current bootstrap-simulation findings, publisher-owned session decisions, module ownership, timeout and reassignment rules, validation gates, acceptance criteria, and sign-off rules.

---

## 9. Phase 6 - Real Publisher Receiver And ACK Path

### 9.1 Objective

Replace simulated bridge-to-publisher forwarding with a real receiver path and ACK flow.

### 9.2 Files To Create Or Modify

Create:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/receiver.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/ack_service.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/forwarder_client.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/tests/receiver_flow.rs`

May modify:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/forwarder.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/session.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/chunk_sender.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/ingest.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/ack.rs`

### 9.3 Deliverables

- real bridge-to-publisher forwarding over a network boundary
- real publisher ACK emission
- session open / data / close path over real receiver service
- `chain_id` propagation on:
  - session open
  - data frames
  - ACK frames
  - close / error events

### 9.4 Validation Tests

- open/data/ack/close end-to-end tests over real service boundaries
- frame reordering and retry tests
- failed bridge forwarder tests
- `chain_id` continuity tests across forwarding and ACK paths
- V2 workspace sanity suite
- V1 file integrity check passes
- minimum V1 code regression suite passes

### 9.5 V1 Preservation Instructions

- do not modify the V1 payload receiver implementation

### 9.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 6: Real Publisher Receiver And ACK Path. Do not proceed to the next phase 7 without getting explicit approval.

Replace simulated bridge-to-publisher forwarding with a real receiver service and ACK path. Ensure session open, data, ACK, retry, and close behavior operate over real service boundaries and preserve chain_id in every correlated event.

Do not modify any file under prototype/gbn-proto/. Do not modify the main repo README.md during this phase.
```

### 9.7 Detailed Execution Reference

Use [GBN-PROTO-006-Execution-Phase6-Real-Publisher-Receiver-And-ACK-Path](GBN-PROTO-006-Execution-Phase6-Real-Publisher-Receiver-And-ACK-Path.md) as the implementation checklist for this phase. It expands the Phase 6 scope into current forwarding-simulation findings, receiver and ACK decisions, module ownership, validation gates, acceptance criteria, and sign-off rules.

---

## 10. Phase 7 - Distributed ChainID Trace Propagation

### 10.1 Objective

Complete and enforce distributed `chain_id` propagation across all Conduit control, bootstrap, data, ACK, storage, logging, and validation paths.

### 10.2 Files To Create Or Modify

Create:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/trace.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/trace.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/trace.rs`
- `prototype/gbn-bridge-proto/tests/integration/test_chain_id.rs`
- `prototype/gbn-bridge-proto/docs/chain-id-design.md`

May modify:

- protocol message types
- runtime and publisher logging / metrics code
- validation scripts and test harness assets

### 10.3 Deliverables

- one canonical `chain_id` propagation model for Conduit
- root `chain_id` generation or import policy
- propagation through:
  - creator
  - host creator
  - bridges
  - authority
  - receiver
  - ACK path
  - progress path
  - bootstrap fanout path
- `chain_id` presence in persistent authority records and test artifacts
- AWS/mobile validation scripts updated to emit and preserve `chain_id`

### 10.4 Validation Tests

- protocol-level trace field tests
- cross-service `chain_id` continuity tests
- structured-log correlation tests
- persistence correlation tests
- script-output correlation tests
- V2 workspace sanity suite
- V1 file integrity check passes
- minimum V1 code regression suite passes

### 10.5 V1 Preservation Instructions

- preserve the V1 field name `chain_id`
- do not alter V1 trace behavior

### 10.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 7: Distributed ChainID Trace Propagation. Do not proceed to the next phase 8 without getting explicit approval.

Implement full distributed chain_id propagation across Conduit. Preserve the V1 field name chain_id and carry it across creator, host-creator, bridge, authority, receiver, ACK, bootstrap, fanout, persistence, logs, metrics, scripts, and distributed tests. Do not introduce a second root tracing field that competes with chain_id.

Do not modify any file under prototype/gbn-proto/. Do not modify the main repo README.md during this phase.
```

### 10.7 Detailed Execution Reference

Use [GBN-PROTO-006-Execution-Phase7-Distributed-ChainID-Trace-Propagation](GBN-PROTO-006-Execution-Phase7-Distributed-ChainID-Trace-Propagation.md) as the implementation checklist for this phase. It expands the Phase 7 scope into current trace-propagation findings, canonical `chain_id` decisions, module ownership, validation gates, acceptance criteria, and sign-off rules.

---

## 11. Phase 8 - Real Deployment Images And AWS Control Plane

### 11.1 Objective

Replace placeholder deployment images and partial AWS scaffolding with a real deployable Conduit service topology.

### 11.2 Files To Create Or Modify

Create:

- `prototype/gbn-bridge-proto/Dockerfile.publisher-authority`
- `prototype/gbn-bridge-proto/Dockerfile.publisher-receiver`
- `prototype/gbn-bridge-proto/docker-compose.conduit-e2e.yml`
- `prototype/gbn-bridge-proto/infra/cloudformation/conduit-full-stack.yaml`
- `prototype/gbn-bridge-proto/infra/scripts/deploy-conduit-full.sh`
- `prototype/gbn-bridge-proto/infra/scripts/smoke-conduit-full.sh`
- `prototype/gbn-bridge-proto/infra/scripts/teardown-conduit-full.sh`

May modify:

- `prototype/gbn-bridge-proto/Dockerfile.bridge`
- `prototype/gbn-bridge-proto/Dockerfile.bridge-publisher`
- `prototype/gbn-bridge-proto/infra/cloudformation/phase2-bridge-stack.yaml`
- `prototype/gbn-bridge-proto/infra/README-infra.md`

### 11.3 Deliverables

- real publisher-authority image
- real publisher-receiver image
- real bridge image
- real creator / host-creator entrypoints where needed
- AWS stack with:
  - authority service
  - receiver service
  - bridge services
  - persistent database
  - secrets management
  - service discovery or stable internal endpoints
  - logging / metrics plumbing

### 11.4 Validation Tests

- image build tests
- local compose boot tests
- CloudFormation template validation
- deploy script dry-run or plan checks
- simulation retirement check passes for deployment entrypoints and images
- extended V1 AWS regression suite passes before this phase is declared complete

### 11.5 V1 Preservation Instructions

- do not modify V1 Dockerfiles or V1 AWS assets

### 11.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 8: Real Deployment Images And AWS Control Plane. Do not proceed to the next phase 9 without getting explicit approval.

Replace placeholder deployment images and partial AWS scaffolding with a real deployable Conduit topology. Deploy real publisher-authority, publisher-receiver, and bridge services together with persistent storage, secrets, and service wiring. Keep chain_id visible in service logs and validation artifacts.

Do not modify any file under prototype/gbn-proto/. Do not modify the main repo README.md during this phase.
```

### 11.7 Detailed Execution Reference

Use [GBN-PROTO-006-Execution-Phase8-Real-Deployment-Images-And-AWS-Control-Plane](GBN-PROTO-006-Execution-Phase8-Real-Deployment-Images-And-AWS-Control-Plane.md) as the implementation checklist for this phase. It expands the Phase 8 scope into current deployment-surface findings, image and stack decisions, module ownership, validation gates, acceptance criteria, and sign-off rules.

---

## 12. Phase 9 - Distributed End-To-End Harness And Fault Injection

### 12.1 Objective

Build a real distributed harness that tests Conduit across actual service boundaries rather than only in-process simulations.

### 12.2 Files To Create Or Modify

Create:

- `prototype/gbn-bridge-proto/tests/e2e/common/mod.rs`
- `prototype/gbn-bridge-proto/tests/e2e/bootstrap.rs`
- `prototype/gbn-bridge-proto/tests/e2e/refresh.rs`
- `prototype/gbn-bridge-proto/tests/e2e/data_path.rs`
- `prototype/gbn-bridge-proto/tests/e2e/failover.rs`
- `prototype/gbn-bridge-proto/tests/e2e/trace.rs`
- `prototype/gbn-bridge-proto/infra/scripts/run-conduit-e2e.sh`

May modify:

- root V2 test harness files
- V2 local smoke scripts

### 12.3 Deliverables

- distributed harness scenarios for:
  - returning creator refresh
  - first-contact bootstrap
  - seed bridge activation
  - 9-bridge fanout
  - bridge downgrade / failover
  - data path and ACKs
  - `chain_id` continuity
- deterministic fault-injection coverage for bridge failure, timeout, reassignment, and restart recovery

### 12.4 Validation Tests

- full local e2e harness pass
- fault injection pass
- trace continuity pass
- V2 workspace sanity suite
- V1 file integrity check passes
- minimum V1 code regression suite passes
- extended V1 local regression suite passes

### 12.5 V1 Preservation Instructions

- do not fold the V2 e2e harness into the V1 harness by editing V1 tests

### 12.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 9: Distributed End-To-End Harness And Fault Injection. Do not proceed to the next phase 10 without getting explicit approval.

Build a real distributed end-to-end harness that exercises the Conduit control and data paths across actual service boundaries. Cover returning refresh, first bootstrap, fanout, failover, data forwarding, ACKs, restart recovery, and chain_id continuity.

Do not modify any file under prototype/gbn-proto/. Do not modify the main repo README.md during this phase.
```

### 12.7 Detailed Execution Reference

Use [GBN-PROTO-006-Execution-Phase9-Distributed-End-To-End-Harness-And-Fault-Injection](GBN-PROTO-006-Execution-Phase9-Distributed-End-To-End-Harness-And-Fault-Injection.md) as the implementation checklist for this phase. It expands the Phase 9 scope into current harness findings, fault-injection decisions, module ownership, validation gates, acceptance criteria, and sign-off rules.

---

## 13. Phase 10 - Live AWS And Mobile Validation

### 13.1 Objective

Validate the full implementation on live AWS and mobile-network conditions.

### 13.2 Files To Create Or Modify

Create:

- `docs/prototyping/GBN-PROTO-006-Conduit-Full-Implementation-Test-Report.md`
- `prototype/gbn-bridge-proto/infra/scripts/mobile-validation-full.sh`
- `prototype/gbn-bridge-proto/infra/scripts/collect-conduit-traces.sh`

May modify:

- existing V2 AWS/mobile validation scripts
- V2 test-matrix docs

### 13.3 Deliverables

- live AWS bootstrap measurements
- live mobile-network bootstrap and punch success measurements
- live data-path and ACK measurements
- live batch-window behavior measurements
- live `chain_id` correlation evidence across authority, bridges, receiver, and validation scripts

### 13.4 Validation Tests

- deployed AWS smoke pass
- live bootstrap pass
- live upload / ACK pass
- mobile-network measurements captured
- `chain_id` evidence captured from end to end
- extended V1 AWS regression suite passes

### 13.5 V1 Preservation Instructions

- do not change the V1 production validation scripts

### 13.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 10: Live AWS And Mobile Validation. Do not proceed to the next phase 11 without getting explicit approval.

Run the fully implemented Conduit system on live AWS and mobile-network conditions. Capture bootstrap, fanout, forwarding, ACK, failover, and trace-correlation evidence. Include end-to-end chain_id evidence across all participating nodes and validation scripts.

Do not modify any file under prototype/gbn-proto/. Do not modify the main repo README.md during this phase.
```

### 13.7 Detailed Execution Reference

Use [GBN-PROTO-006-Execution-Phase10-Live-AWS-And-Mobile-Validation](GBN-PROTO-006-Execution-Phase10-Live-AWS-And-Mobile-Validation.md) as the implementation checklist for this phase. It expands the Phase 10 scope into current live-validation findings, evidence-capture decisions, validation gates, acceptance criteria, and sign-off rules.

---

## 14. Phase 11 - Decision Gate

### 14.1 Objective

Decide whether the full Conduit implementation is ready to move beyond experimental coexistence.

### 14.2 Files To Create Or Modify

Create:

- `docs/prototyping/GBN-PROTO-006-Decision-Record.md`
- `docs/architecture/GBN-ARCH-008-Conduit-Full-Implementation-Decision.md`

May modify:

- this execution plan document
- V2 architecture and prototype docs

### 14.3 Deliverables

- explicit decision on whether Conduit is:
  - still experimental
  - production-capable but opt-in
  - ready for promotion beyond coexistence
- gap list for any remaining blockers
- recorded decision on whether `chain_id` propagation is complete enough for production observability

### 14.4 Validation Tests

- all required Phase 10 evidence exists
- all required V1 regressions remain green
- all simulation retirement checks for claimed production paths are satisfied

### 14.5 V1 Preservation Instructions

- do not change V1 default behavior without a separate approved migration decision

### 14.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 11: Decision Gate. Do not proceed beyond this phase without explicit approval.

Evaluate the full Conduit implementation against the architecture, the distributed publisher requirements, the live validation evidence, and the chain_id observability requirements. Record a clear decision on whether Conduit remains experimental, becomes production-capable but opt-in, or is ready for broader promotion.

Do not modify any file under prototype/gbn-proto/. Do not modify the main repo README.md during this phase unless a separate explicit documentation approval is given after the decision record is complete.
```

### 14.7 Detailed Execution Reference

Use [GBN-PROTO-006-Execution-Phase11-Decision-Gate](GBN-PROTO-006-Execution-Phase11-Decision-Gate.md) as the implementation checklist for this phase. It expands the Phase 11 scope into current decision-surface findings, evidence sufficiency criteria, validation gates, acceptance criteria, and sign-off rules.

---

## 15. Recommended Execution Order

Implement this plan strictly in phase order.

Recommended milestone grouping:

1. Baseline and control plane
   - Phase 0
   - Phase 1
   - Phase 2
   - Phase 3
2. Runtime migration and bootstrap distribution
   - Phase 4
   - Phase 5
   - Phase 6
3. Trace, deployment, and distributed test hardening
   - Phase 7
   - Phase 8
   - Phase 9
4. Live validation and decision
   - Phase 10
   - Phase 11

Detailed phase-by-phase execution documents should be created only after this master plan is approved.
