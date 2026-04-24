# GBN-PROTO-005 - Phase 2 Distributed Peer-to-Peer Onion Redesign - Execution Plan

**Document ID:** GBN-PROTO-005  
**Status:** Active - Phase 0 complete, Phase 1 complete, Phase 2 complete, Phase 3 complete, Phase 4 complete, Phase 5 complete, Phase 6 complete, Phase 7 complete, Phase 8 complete, Phase 9 complete, Phase 10 implemented locally with AWS deployment validation pending, Phase 11 implemented locally with live mobile validation pending, Phase 12 implemented and decision recorded: Conduit remains experimental
**Last Updated:** 2026-04-22
**Related Docs:** [GBN-PROTO-005 Plan](GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign.md), [GBN-ARCH-000-V2](../architecture/GBN-ARCH-000-System-Architecture-V2.md), [GBN-ARCH-001-V2](../architecture/GBN-ARCH-001-Media-Creation-Network-V2.md), [GBN-ARCH-002-V2](../architecture/GBN-ARCH-002-Bridge-Protocol-V2.md)

This document expands the GBN-PROTO-005 execution roadmap into concrete implementation phases. It is intentionally additive to the existing `prototype/gbn-proto` workspace and must preserve the V1 onion implementation unchanged.

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

### 1.2 Workspace Isolation Rule

The V2 redesign must be built in a separate sibling workspace:

```text
prototype/
|- gbn-proto/          # V1 onion implementation, preserved
`- gbn-bridge-proto/   # V2 bridge-mode implementation, new
```

The V2 work must not require the V1 workspace to be edited in order to compile, run, or deploy.

### 1.3 Allowed Reuse Rule

Read-only reuse of V1 code patterns is allowed. Editing V1 code is not.

Preferred order of reuse:

1. Read V1 implementation for reference.
2. Reuse concepts and schemas in V2-local code.
3. If safe, consume V1 crates as path dependencies without editing them.
4. If adapting a V1 crate would require API or behavior changes, copy the needed logic into V2-local crates instead of mutating V1.

### 1.4 V1 No-Touch Paths

Unless the user explicitly approves otherwise, do not modify these files or directories during GBN-PROTO-005 implementation:

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

### 1.5 V1 Onion Logic That Must Remain Untouched

The following V1 modules encode current onion-mode behavior and must be preserved exactly unless a separate explicit V1 change request is approved:

- `prototype/gbn-proto/crates/mcn-router-sim/src/swarm.rs`
- `prototype/gbn-proto/crates/mcn-router-sim/src/gossip.rs`
- `prototype/gbn-proto/crates/mcn-router-sim/src/control.rs`
- `prototype/gbn-proto/crates/mcn-router-sim/src/circuit_manager.rs`
- `prototype/gbn-proto/crates/mcn-router-sim/src/relay_engine.rs`
- `prototype/gbn-proto/crates/proto-cli/src/main.rs`
- `prototype/gbn-proto/crates/gbn-protocol/src/onion.rs`
- `prototype/gbn-proto/crates/gbn-protocol/src/dht.rs`
- `prototype/gbn-proto/crates/gbn-protocol/src/chunk.rs`

### 1.6 Prompt Design Rule

Every phase prompt in this document must be usable by a fresh agent with little or no prior context.

That means each prompt must be self-contained about:

- the repo split between V1 and V2
- the V1 onion-mode baseline that must be preserved
- the V2 bridge-mode concept being built
- the exact phase scope
- the allowed and forbidden file paths
- the required validation commands
- the rule that the agent must stop after the current phase and wait for approval

---

## 2. Validation Baseline

Each phase below lists its own validation tests. The following baseline suites are reused across phases.

### 2.1 V1 File Integrity Check

Run a path-scoped diff before and after each phase to confirm no forbidden V1 paths changed:

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

These tests confirm the V1 workspace still compiles and the core V1 router simulation tests still pass.

### 2.3 Extended V1 Local Regression Suite

Run when a phase introduces new local dev tooling, Docker assets, or shared documentation references:

```bash
bash validate-scale-test.sh
```

This is a partial local regression pass for the V1 scaled topology workflow.

### 2.4 Extended V1 AWS Regression Suite

Run when a phase introduces new deployment tooling or any repo-level changes that could accidentally affect shared infrastructure conventions:

```bash
bash infra/scripts/run-tests.sh <v1-stack-name> <region>
```

This is the most expensive V1 regression and should be used as a gate before any V2 deployment phase is declared complete.

### 2.5 V2 Workspace Sanity Suite

Once the V2 workspace exists, every phase that changes code there should run:

```bash
cd prototype/gbn-bridge-proto
cargo fmt --check
cargo check --workspace
cargo test --workspace
```

If the V2 workspace does not yet contain all crates or tests, run the subset that is valid for that phase and document the gap explicitly.

---

## 3. Phase 0 - Freeze The V1 Baseline

### 3.1 Objective

Record the current V1 onion implementation as the baseline that V2 must preserve.

Phase 0 is complete. The frozen Lattice baseline is published as [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline) from commit `c5dc415124f101e5de3dd20e2eeed608bd6948df`.

### 3.2 Files To Create Or Modify

Create:

- `docs/prototyping/GBN-PROTO-005-V1-Baseline-Freeze.md`
- `docs/prototyping/GBN-PROTO-005-V1-Regression-Suite.md`

May modify:

- `docs/prototyping/GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign.md`
- this execution plan document

Must not modify:

- any path under `prototype/gbn-proto/`

### 3.3 Deliverables

- a recorded V1 baseline manifest containing:
  - current branch or tag
  - commit hash
  - V1 test suites that must remain green
  - no-touch file path list
- a documented definition of what counts as V1 regression
- a GitHub release package definition containing:
  - release tag name
  - release title
  - release note template
  - release target commit requirements

### 3.4 Validation Tests

- V1 file integrity check passes
- minimum V1 code regression suite passes
- baseline manifest includes a concrete commit hash and test command list
- Phase 0 release tag and published GitHub release exist

### 3.5 V1 Preservation Instructions

- Do not edit `prototype/gbn-proto/Cargo.toml`.
- Do not edit any V1 crate source.
- Do not rename or move V1 scripts.
- Do not rewrite any GBN-PROTO-004 or V1 architecture docs.

### 3.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 0: Freeze The V1 Baseline. Do not proceed to the next phase 1 without getting explicit approval.

Create documentation-only artifacts that freeze the current V1 onion implementation as the protected baseline for GBN-PROTO-005. Record the exact V1 commit or branch reference, enumerate the V1 no-touch paths, and define the V1 regression suites that must remain green for every later phase.

Do not modify any file under prototype/gbn-proto/. Do not change any V1 architecture or prototype document except to add new V2-only documentation in new files. The output of this phase is a clear baseline manifest and regression checklist, not new code.

Do not modify the main repo README.md during this phase. Keep README.md pinned to the published Lattice release-facing content, and defer any V2 README updates until all V2 code changes are complete and explicitly approved as a separate documentation pass.
```

### 3.7 Detailed Execution Reference

Use [GBN-PROTO-005-Execution-Phase0-V1-Baseline-Freeze](GBN-PROTO-005-Execution-Phase0-V1-Baseline-Freeze.md) as the implementation checklist for this phase. It expands the Phase 0 scope into preflight checks, required evidence capture, release packaging, validation gates, and sign-off criteria. This phase is now complete; use the published Lattice release as the V1 preservation reference point for all later phases.

---

## 4. Phase 1 - Create The V2 Workspace Boundary

### 4.1 Objective

Create a separate V2 workspace that can evolve without changing V1.

Phase 1 is complete. The isolated V2 workspace exists under `prototype/gbn-bridge-proto/`, and the Phase 0 gate remains satisfied by the published baseline release [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline).

### 4.2 Files To Create Or Modify

Create:

- `prototype/gbn-bridge-proto/Cargo.toml`
- `prototype/gbn-bridge-proto/.gitignore`
- `prototype/gbn-bridge-proto/README.md`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/Cargo.toml`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/lib.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/Cargo.toml`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/lib.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/Cargo.toml`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/lib.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-cli/Cargo.toml`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/main.rs`
- `prototype/gbn-bridge-proto/tests/.gitkeep`
- `prototype/gbn-bridge-proto/infra/README-infra.md`
- `prototype/gbn-bridge-proto/infra/scripts/.gitkeep`
- `prototype/gbn-bridge-proto/infra/cloudformation/.gitkeep`

May modify:

- V2 docs under `docs/prototyping/GBN-PROTO-005*`
- V2 docs under `docs/architecture/*V2.md`

Must not modify:

- `prototype/gbn-proto/Cargo.toml`
- any V1 workspace member list

### 4.3 Deliverables

- independent V2 Cargo workspace
- initial crate boundaries for protocol, runtime, publisher, and CLI
- naming rules for V2 images, stacks, env vars, and metrics

### 4.4 Validation Tests

- `cd prototype/gbn-bridge-proto && cargo fmt --check`
- `cd prototype/gbn-bridge-proto && cargo check --workspace`
- `cd prototype/gbn-bridge-proto && cargo test --workspace`
- the published Phase 0 baseline release exists: [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)
- V1 file integrity check passes
- minimum V1 code regression suite passes

### 4.5 V1 Preservation Instructions

- Do not add V2 crates to `prototype/gbn-proto/Cargo.toml`.
- Do not place new V2 code under `prototype/gbn-proto/crates/`.
- Do not reuse V1 infra script names inside V1 directories.

### 4.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 1: Create The V2 Workspace Boundary. Do not proceed to the next phase 2 without getting explicit approval.

Create a new sibling Rust workspace at prototype/gbn-bridge-proto/ with isolated crates for protocol, runtime, publisher, and CLI. Add only the minimum scaffolding needed to compile the empty workspace. Establish V2 naming conventions for environment variables, image names, stack names, and metrics namespaces.

Do not modify prototype/gbn-proto/Cargo.toml or any V1 source file. The V2 workspace must compile independently.

Do not modify the main repo README.md during this phase. Keep README.md pinned to the published Lattice release-facing content, and defer any V2 README updates until all V2 code changes are complete and explicitly approved as a separate documentation pass.
```

### 4.7 Detailed Execution Reference

Use [GBN-PROTO-005-Execution-Phase1-V2-Workspace-Boundary](GBN-PROTO-005-Execution-Phase1-V2-Workspace-Boundary.md) as the implementation checklist and completion record for this phase. It expands the Phase 1 scope into current repo findings, preflight gates, evidence capture, workspace-boundary rules, file-by-file minimum content, naming decisions, validation gates, and sign-off criteria.

---

## 5. Phase 2 - Lock The V2 Wire Model

### 5.1 Objective

Define the transport, authority, bootstrap, and reachability-repair schemas before runtime logic is implemented.

Phase 2 must lock the canonical M1 wire model so later runtime and authority work do not churn message names or descriptor fields.

Phase 2 is complete. The canonical M1 wire model is committed in `gbn-bridge-protocol` and documented in `GBN-ARCH-002`.

### 5.2 Files To Create Or Modify

Create:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/descriptor.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/bootstrap.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/messages.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/catalog.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/lease.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/punch.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/session.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/signing.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/error.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/tests/protocol_roundtrip.rs`
- `docs/architecture/GBN-ARCH-002-Bridge-Protocol-V2.md`

May modify:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/lib.rs`
- V2 docs

Must not modify:

- `prototype/gbn-proto/crates/gbn-protocol/**`

### 5.3 Deliverables

- canonical `BridgeDescriptor` schema for M1 including:
  - `bridge_id`
  - `identity_pub`
  - `ingress_endpoints[]`
  - `udp_punch_port`
  - `reachability_class`
  - `lease_expiry_ms`
  - `capabilities[]`
  - `publisher_sig`
- explicit deferral note for `network_type`, `geo_tag`, and `observed_reliability_score` unless separately approved for a later phase
- publisher-seeded bootstrap entry schema for creators and bridges
- bridge registration, heartbeat, lease, catalog, and bootstrap response messages
- creator bootstrap / authority wire types:
  - `BridgeRefreshHint`
  - `CreatorJoinRequest`
  - `CreatorBootstrapResponse`
  - `BridgeSetRequest`
  - `BridgeSetResponse`
- reachability-repair wire types:
  - `BridgePunchStart`
  - `BridgePunchProbe`
  - `BridgePunchAck`
  - `BootstrapProgress`
  - `BridgeBatchAssign`
- session wire types:
  - `BridgeOpen`
  - `BridgeData`
  - `BridgeAck`
- signature, expiry, versioning, and replay-prevention semantics

### 5.4 Validation Tests

- serde round-trip tests for every protocol type
- signature verification tests for valid and invalid publisher signatures
- lease expiry tests
- bootstrap entry expiry and signature validation tests
- UDP punch message round-trip tests
- protocol version mismatch tests
- V2 workspace sanity suite
- V1 file integrity check passes
- minimum V1 code regression suite passes

### 5.5 V1 Preservation Instructions

- Do not add fields to V1 `RelayNode`, `DhtEntry`, or onion protocol messages.
- Do not change V1 chunk framing, DHT framing, or onion message serialization.
- If a V1 type is inspirational but not identical, create a new V2 type instead of editing the old one.

### 5.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 2: Lock The V2 Wire Model. Do not proceed to the next phase 3 without getting explicit approval.

Implement the V2 bridge protocol schemas in the gbn-bridge-protocol crate. Define BridgeDescriptor with udp_punch_port, publisher-seeded bootstrap entry types, registration, heartbeat, lease, catalog, creator-bootstrap, UDP punch, batching, and bridge-session message types together with signing, expiry, and version semantics. Add protocol round-trip and signature validation tests.

For Phase 2, treat the following as the canonical minimal BridgeDescriptor field set: bridge_id, identity_pub, ingress_endpoints[], udp_punch_port, reachability_class, lease_expiry_ms, capabilities[], and publisher_sig. Do not add network_type, geo_tag, or observed_reliability_score in this phase unless explicitly approved. Include BridgeRefreshHint in the creator discovery / refresh message surface so the protocol crate matches the current Conduit architecture docs.

Do not modify any file under prototype/gbn-proto/crates/gbn-protocol/. All V2 wire types must live under prototype/gbn-bridge-proto/.

Do not modify the main repo README.md during this phase. Keep README.md pinned to the published Lattice release-facing content, and defer any V2 README updates until all V2 code changes are complete and explicitly approved as a separate documentation pass.
```

### 5.7 Detailed Execution Reference

Use [GBN-PROTO-005-Execution-Phase2-V2-Wire-Model](GBN-PROTO-005-Execution-Phase2-V2-Wire-Model.md) as the implementation checklist and completion record for this phase. It expands the Phase 2 scope into current repo findings, canonical wire decisions, module-boundary rules, dependency policy, validation fallback strategy, risks, sign-off criteria, and the executed validation results.

---

## 6. Phase 3 - Publisher Authority Plane

### 6.1 Objective

Implement the publisher-side authority service that signs leases, serves bridge catalogs, and coordinates first-contact bootstrap plus batched punch fanout.

Phase 3 is complete. The publisher authority plane is committed from the Phase 2 protocol baseline and remains the authority surface for all later Conduit work.

### 6.2 Files To Create Or Modify

Create:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/authority.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/bootstrap.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/batching.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/registry.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/lease.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/catalog.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/punch.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/server.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/storage.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/metrics.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/tests/authority_flow.rs`

May modify:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/lib.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/main.rs`

Must not modify:

- `prototype/gbn-proto/crates/mpub-receiver/**`
- `prototype/gbn-proto/crates/proto-cli/src/main.rs`

### 6.3 Deliverables

- bridge registration validation
- signed lease issuance including preferred UDP punch port
- bridge heartbeat and liveness tracking
- bridge catalog generation and signing
- seed-bridge selection for first-time creators
- publisher-seeded bootstrap payload issuance for new creators and active bridges
- creator/bootstrap punch instruction generation
- short-window batching for new-creator onboarding fanout
- publisher-side policy hooks for direct vs non-direct bridges

### 6.4 Validation Tests

- bridge registration success and rejection cases
- heartbeat extends liveness and lease state correctly
- expired bridges disappear from catalogs
- tampered catalogs fail verification
- bootstrap request acceptance and rejection cases
- seed-bridge selection and bootstrap payload signing tests
- batch assignment tests for 10-request and 11th-request rollover cases
- V2 workspace sanity suite
- V1 file integrity check passes
- minimum V1 code regression suite passes

### 6.5 V1 Preservation Instructions

- Do not reuse `mpub-receiver` as the V2 authority service by editing it in place.
- Do not alter V1 publisher binaries, ports, or CLI flags.
- Keep V2 authority state and metrics names isolated from V1 names.

### 6.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 3: Publisher Authority Plane. Do not proceed to the next phase 4 without getting explicit approval.

Implement the V2 publisher authority service in the gbn-bridge-publisher crate. Add bridge registration handling, signed lease issuance with UDP punch port metadata, bridge liveness tracking through heartbeats, signed catalog generation, first-contact creator bootstrap orchestration, and batched punch fanout assignment. Add tests covering successful registration, rejection, lease expiry, catalog signing, bootstrap issuance, and batch rollover.

Do not modify mpub-receiver or any V1 publisher logic. V2 publisher authority must be implemented in V2-local files only.

Do not modify the main repo README.md during this phase. Keep README.md pinned to the published Lattice release-facing content, and defer any V2 README updates until all V2 code changes are complete and explicitly approved as a separate documentation pass.
```

### 6.7 Detailed Execution Reference

Use [GBN-PROTO-005-Execution-Phase3-V2-Publisher-Authority-Plane](GBN-PROTO-005-Execution-Phase3-V2-Publisher-Authority-Plane.md) as the implementation checklist and execution record for this phase. It expands the Phase 3 scope into current repo findings, authority-boundary rules, bootstrap and batching policy assumptions, dependency limits, validation fallback strategy, risks, sign-off criteria, and the executed validation results.

---

## 7. Phase 4 - ExitBridge Runtime

### 7.1 Objective

Implement the runtime for an authorized ExitBridge node, including Publisher-directed UDP hole punching and seed-bridge bootstrap duties.

Phase 4 is complete. The ExitBridge runtime is committed against the current Phase 3 authority surface and is the bridge baseline for creator bootstrap work.

### 7.2 Files To Create Or Modify

Create:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/bridge.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/bootstrap_bridge.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/publisher_client.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/heartbeat_loop.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/lease_state.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/creator_listener.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/forwarder.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/progress_reporter.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/punch.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/tests/bridge_runtime.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/bin/exit-bridge.rs`

May modify:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/lib.rs`

Must not modify:

- `prototype/gbn-proto/crates/mcn-router-sim/src/relay_engine.rs`
- `prototype/gbn-proto/crates/mcn-router-sim/src/control.rs`

### 7.3 Deliverables

- outbound registration to V2 publisher
- lease and heartbeat maintenance loop
- creator ingress listener for `direct` bridges
- Publisher-directed UDP punch initiation toward creators
- seed-bridge response path that can return publisher-seeded bridge entries to a new creator
- progress reporting back to Publisher as tunnels come online
- payload forwarder to publisher

### 7.4 Validation Tests

- bridge registers successfully on startup
- bridge starts UDP punching only after Publisher instruction or valid creator refresh state
- seed bridge establishes and ACKs a bootstrap tunnel to a new creator
- bridge drops creator ingress when lease becomes invalid
- bridge reconnects and re-registers after publisher restart
- bridge never exposes ingress when reachability class is not `direct`
- V2 workspace sanity suite
- V1 file integrity check passes
- minimum V1 code regression suite passes

### 7.5 V1 Preservation Instructions

- Do not refactor V1 relay runtime into a shared library during this phase.
- Do not alter V1 relay control sockets or DHT behavior.
- If V1 tracing helpers are useful, copy patterns or reuse read-only APIs; do not rewrite V1 tracing code.

### 7.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 4: ExitBridge Runtime. Do not proceed to the next phase 5 without getting explicit approval.

Implement the ExitBridge runtime in the V2 runtime crate. The bridge must register outbound to the V2 publisher, maintain lease and heartbeat state, expose creator ingress only when allowed by policy, execute Publisher-directed UDP punching, return publisher-seeded bridge entries when acting as the seed bridge, report tunnel progress, and forward opaque creator payloads upstream.

Do not modify any V1 relay or control-plane implementation. The ExitBridge is a new V2 runtime, not a refactor of the old onion relay.

Do not modify the main repo README.md during this phase. Keep README.md pinned to the published Lattice release-facing content, and defer any V2 README updates until all V2 code changes are complete and explicitly approved as a separate documentation pass.
```

---

## 8. Phase 5 - Creator Bootstrap Flow

### 8.1 Objective

Implement the creator-side bootstrap path for both returning creators and first-time creators, including HostCreator-assisted onboarding and immediate UDP punch fanout.

Phase 5 is complete. The creator bootstrap flow is committed from the Phase 3 and Phase 4 baseline and is now the creator/runtime starting point for the Phase 6 data path.

### 8.2 Files To Create Or Modify

Create:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/creator.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/catalog_cache.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/host_creator.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/local_dht.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/punch_fanout.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/selector.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/bootstrap.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/tests/creator_bootstrap.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/bin/creator-client.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/bin/host-creator.rs`
- `prototype/gbn-bridge-proto/configs/creator.example.toml`
- `prototype/gbn-bridge-proto/configs/host_creator.example.toml`

May modify:

- V2 protocol and runtime modules created in earlier phases

Must not modify:

- `prototype/gbn-proto/crates/mcn-router-sim/src/circuit_manager.rs`
- `prototype/gbn-proto/crates/proto-cli/src/main.rs`

### 8.3 Deliverables

- creator trust-root loading
- cached catalog load and validation
- bridge selection filtering for direct reachable bridges
- HostCreator-assisted first-time creator join flow
- seed-bridge establishment and ACK handling
- local DHT / discovery-table updates from publisher-seeded bootstrap entries
- immediate creator-to-bridge UDP punch fanout after catalog refresh or bootstrap
- fresh catalog request through a connected bridge
- catalog cache update flow

### 8.4 Validation Tests

- expired bridge descriptors are rejected
- invalid publisher signatures are rejected
- returning creator can connect using only cached descriptors
- first-time creator can reach the Publisher through HostCreator and seed bridge
- creator stores the 9 publisher-seeded bridge entries after bootstrap
- creator refreshes catalog through a reachable bridge
- creator and bridge ACK bidirectional tunnel establishment on default port `443` unless overridden
- creator retries next bridge when first bridge fails
- V2 workspace sanity suite
- V1 file integrity check passes
- minimum V1 code regression suite passes

### 8.5 V1 Preservation Instructions

- Do not edit V1 creator upload logic.
- Do not reuse V1 DHT bootstrap logic by mutating it in place.
- Keep V2 creator configuration files separate from V1 env vars and CLI flags.

### 8.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 5: Creator Bootstrap Flow. Do not proceed to the next phase 6 without getting explicit approval.

Implement the V2 creator bootstrap flow. The creator must support returning-creator refresh from cached signed bridge descriptors and first-time bootstrap through a HostCreator path, validate publisher-signed bootstrap entries, establish a seed bridge, update its local DHT / discovery table, and immediately start UDP punch fanout toward newly assigned bridges. Add tests for expiry filtering, invalid signatures, cached reconnect, first-time bootstrap, tunnel ACKs, and retry across bridges.

Do not modify the V1 creator circuit manager or V1 upload path. This is a new bootstrap flow for V2 bridge mode only.

Do not modify the main repo README.md during this phase. Keep README.md pinned to the published Lattice release-facing content, and defer any V2 README updates until all V2 code changes are complete and explicitly approved as a separate documentation pass.
```

### 8.7 Detailed Execution Reference

Use [GBN-PROTO-005-Execution-Phase5-V2-Creator-Bootstrap-Flow](GBN-PROTO-005-Execution-Phase5-V2-Creator-Bootstrap-Flow.md) as the implementation checklist and current execution record for this phase. It expands the Phase 5 scope into current repo findings, trust-root and cache rules, HostCreator trust-boundary assumptions, selector and fanout policy, validation fallback strategy, risks, sign-off criteria, and the executed validation results.

---

## 9. Phase 6 - Bridge-Mode Data Path

### 9.1 Objective

Implement encrypted creator upload through the bridge and publisher ACK return path, with progressive 10-bridge fanout and bridge reuse on timeout.

Phase 6 is complete. The bridge-mode data path is committed from the Phase 5 creator/bootstrap baseline and now serves as the runtime baseline for Phase 7 discovery work.

### 9.2 Files To Create Or Modify

Create:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/session.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/bridge_pool.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/fanout_scheduler.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/framing.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/ack_tracker.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/chunk_sender.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/ingest.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/ack.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/tests/data_path.rs`

May modify:

- earlier V2 protocol/runtime/publisher files

Must not modify:

- `prototype/gbn-proto/crates/gbn-protocol/src/chunk.rs`
- `prototype/gbn-proto/crates/gbn-protocol/src/onion.rs`
- `prototype/gbn-proto/crates/mpub-receiver/**`

### 9.3 Deliverables

- `BridgeOpen`, `BridgeData`, and `BridgeAck` runtime handling
- creator-side payload wrapping and chunk fanout scheduling across up to 10 active bridges
- bridge-side opaque forwarding
- publisher-side receive, validate, and ACK flow
- retry, failover, and reuse of already-live bridges when fewer than 10 are available before timeout

### 9.4 Validation Tests

- end-to-end upload from creator to publisher through one bridge
- ACK correlation to the correct creator session
- bridge failover during mid-session upload
- creator reuses active bridges when full 10-bridge fanout is not available before timeout
- confidentiality test proving the bridge cannot inspect publisher-encrypted payload
- replay or duplicate `BridgeData` handling tests
- V2 workspace sanity suite
- V1 file integrity check passes
- minimum V1 code regression suite passes

### 9.5 V1 Preservation Instructions

- Do not extend the V1 onion frame to carry bridge-mode messages.
- Do not add V2 bridge ACK logic into V1 publisher receive code.
- If V1 chunking helpers are useful, consume them read-only or reimplement equivalent logic in V2.

### 9.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 6: Bridge-Mode Data Path. Do not proceed to the next phase 7 without getting explicit approval.

Implement the V2 encrypted upload path from creator to publisher through ExitBridges. Add session open, bridge data transfer, ACK handling, retransmission, progressive fanout across active bridges, reuse of already-live bridges when fanout is incomplete, and failover behavior. Add end-to-end tests for upload success, ACK routing, bridge reuse, failover, and bridge payload opacity.

Do not modify V1 onion message framing, V1 chunk protocol types, or V1 publisher receive code.

Do not modify the main repo README.md during this phase. Keep README.md pinned to the published Lattice release-facing content, and defer any V2 README updates until all V2 code changes are complete and explicitly approved as a separate documentation pass.
```

### 9.7 Detailed Execution Reference

Use [GBN-PROTO-005-Execution-Phase6-V2-Bridge-Mode-Data-Path](GBN-PROTO-005-Execution-Phase6-V2-Bridge-Mode-Data-Path.md) as the implementation checklist and current execution record for this phase. It expands the Phase 6 scope into current repo findings, trust and confidentiality boundaries, session and ACK rules, fanout / reuse policy, validation fallback strategy, risks, sign-off criteria, and the executed validation results.

---

## 10. Phase 7 - Weak Discovery Integration

### 10.1 Objective

Add optional weak discovery as a non-authoritative hint layer that supplements, but never overrides, publisher-signed catalogs and publisher-seeded bootstrap entries.

Phase 7 is complete. Weak-discovery integration is committed from the Phase 6 bridge-mode data-path baseline and now serves as the runtime trust/discovery baseline for Phase 8 reachability policy work.

### 10.2 Files To Create Or Modify

Create:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/discovery.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/seed_catalog.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/hint_merge.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/tests/discovery.rs`
- `prototype/gbn-bridge-proto/docs/discovery-design.md`

May modify:

- creator bootstrap, local DHT, catalog cache, and selector modules in V2

Must not modify:

- `prototype/gbn-proto/crates/mcn-router-sim/src/swarm.rs`
- `prototype/gbn-proto/crates/mcn-router-sim/src/gossip.rs`
- `prototype/gbn-proto/crates/gbn-protocol/src/dht.rs`

### 10.3 Deliverables

- weak discovery hints from static seeds or a simplified discovery service
- candidate hint merge logic with explicit precedence rules:
  - active publisher-seeded bootstrap entries
  - freshest publisher-signed catalog descriptors
  - weak discovery hints
- strict enforcement that only publisher-signed descriptors are transport-eligible
- bootstrap protection rules ensuring weak discovery cannot replace the Publisher-selected seed bridge or initial bridge set for a new creator session

### 10.4 Validation Tests

- discovery candidate without valid publisher signature is ignored for transport
- weak discovery cannot override active bootstrap entries for a new creator session
- discovery candidates can seed a later successful publisher catalog refresh
- stale discovery data does not override fresher signed data
- creator still functions when discovery is disabled but cached catalog exists
- V2 workspace sanity suite
- V1 file integrity check passes
- minimum V1 code regression suite passes

### 10.5 V1 Preservation Instructions

- Do not add new V2 bridge semantics into V1 gossip or DHT paths.
- Do not repurpose V1 `NodeAnnounce`, `DirectNodeProbe`, or DHT validation loops for V2 by editing them.
- If a discovery concept from V1 is useful, re-express it in V2-local modules.

### 10.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 7: Weak Discovery Integration. Do not proceed to the next phase 8 without getting explicit approval.

Implement a V2 weak-discovery layer that can surface candidate bridge hints without granting trust. A creator may use these hints to find potential bridges, but only publisher-signed bridge descriptors may be used for actual transport. Add tests proving discovery cannot override publisher authorization.

Do not modify the V1 DHT, gossip, or direct-validation logic. V2 weak discovery must be implemented in V2-local code.

Do not modify the main repo README.md during this phase. Keep README.md pinned to the published Lattice release-facing content, and defer any V2 README updates until all V2 code changes are complete and explicitly approved as a separate documentation pass.
```

### 10.7 Detailed Execution Reference

Use [GBN-PROTO-005-Execution-Phase7-V2-Weak-Discovery-Integration](GBN-PROTO-005-Execution-Phase7-V2-Weak-Discovery-Integration.md) as the implementation checklist and completion record for this phase. It expands the Phase 7 scope into current repo findings, weak-hint trust boundaries, deterministic merge precedence, bootstrap-protection rules, validation fallback strategy, risks, sign-off criteria, and the executed validation results.

---

## 11. Phase 8 - Reachability Classification

### 11.1 Objective

Implement the publisher and creator policy for `direct`, `brokered`, and `relay_only` bridges, including seed-bridge eligibility and preferred UDP punch port handling.

Phase 8 is complete. Reachability classification is committed from the Phase 7 weak-discovery baseline and now serves as the transport-eligibility baseline for Phase 9 test-harness work.

### 11.2 Files To Create Or Modify

Create:

- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/reachability.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/policy.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/bridge_scoring.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/tests/reachability.rs`

May modify:

- `descriptor.rs`, bootstrap types, and catalog-related files in V2 protocol
- creator selector and publisher catalog code in V2

Must not modify:

- V1 subnet-tag or DHT state machine logic

### 11.3 Deliverables

- publisher-side policy and scoring for classifying bridges
- bridge eligibility rules for seed-bridge selection, initial bootstrap fanout, and ordinary catalog refresh
- creator-side filtering by reachability class and signed `udp_punch_port`
- downgrade and exclusion behavior when a direct bridge becomes brokered or relay_only
- safe cache update behavior when reachability class or preferred punch port changes

### 11.4 Validation Tests

- only `direct` bridges are returned to creators for first-contact bootstrap
- only `direct` bridges are included in the initial new-creator bridge set
- `brokered` and `relay_only` bridges are kept out of first-contact seed selection and immediate punch fanout
- class downgrade removes a bridge from new creator selections and active candidate refresh
- catalog and cache updates handle class and punch-port transitions safely
- V2 workspace sanity suite
- V1 file integrity check passes
- minimum V1 code regression suite passes

### 11.5 V1 Preservation Instructions

- Do not overload V1 role tags such as `FreeSubnet` or `HostileSubnet` to represent V2 reachability classes.
- Keep V2 reachability as a new concept in V2 protocol and policy files only.

### 11.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 8: Reachability Classification. Do not proceed to the next phase 9 without getting explicit approval.

Implement V2 reachability classification and policy. The publisher must classify bridges as direct, brokered, or relay_only, and creators must only use direct bridges for first-contact bootstrap. Add tests for class filtering, class downgrade, and catalog update behavior.

Do not change V1 subnet-tag semantics or V1 DHT validation state. Reachability classes are new V2-only transport metadata.

Do not modify the main repo README.md during this phase. Keep README.md pinned to the published Lattice release-facing content, and defer any V2 README updates until all V2 code changes are complete and explicitly approved as a separate documentation pass.
```

### 11.7 Detailed Execution Reference

Use [GBN-PROTO-005-Execution-Phase8-V2-Reachability-Classification](GBN-PROTO-005-Execution-Phase8-V2-Reachability-Classification.md) as the implementation checklist for this phase. It expands the Phase 8 scope into current repo findings, class semantics, eligibility rules, signed UDP port transition handling, validation fallback strategy, risks, and sign-off criteria.

---

## 12. Phase 9 - Test Harness

### 12.1 Objective

Create a dedicated V2 integration harness for local and CI-style bridge-mode testing across returning-creator refresh, first-time bootstrap, UDP punching, batching, and progressive upload.

Phase 9 is complete and committed from the Phase 8 reachability-classification baseline. The V2 harness, V2 workspace sanity suite, protected V1 diff check, and minimum V1 regression suite passed.

### 12.2 Files To Create Or Modify

Create:

- `prototype/gbn-bridge-proto/tests/common/mod.rs`
- `prototype/gbn-bridge-proto/tests/integration/test_bridge_registration.rs`
- `prototype/gbn-bridge-proto/tests/integration/test_catalog_refresh.rs`
- `prototype/gbn-bridge-proto/tests/integration/test_first_creator_bootstrap.rs`
- `prototype/gbn-bridge-proto/tests/integration/test_udp_punch_ack.rs`
- `prototype/gbn-bridge-proto/tests/integration/test_creator_failover.rs`
- `prototype/gbn-bridge-proto/tests/integration/test_batch_bootstrap.rs`
- `prototype/gbn-bridge-proto/tests/integration/test_bridge_reuse_timeout.rs`
- `prototype/gbn-bridge-proto/tests/integration/test_payload_confidentiality.rs`
- `prototype/gbn-bridge-proto/tests/integration/test_reachability_filtering.rs`
- `prototype/gbn-bridge-proto/docker-compose.bridge-smoke.yml`
- `prototype/gbn-bridge-proto/infra/scripts/run-local-bridge-tests.sh`
- `prototype/gbn-bridge-proto/test-vectors/README.md`

May modify:

- V2 crate tests and local configs
- `prototype/gbn-bridge-proto/Cargo.toml`
- minimal V2-local root package files needed to make root integration tests executable under Cargo

Must not modify:

- `prototype/gbn-proto/tests/integration/**`
- `prototype/gbn-proto/validate-scale-test.sh`

### 12.3 Deliverables

- reproducible local harness for host creator, creator, multiple bridges, and publisher authority
- automated integration tests for bridge mode including first-time bootstrap, tunnel ACKs, batch fanout, failover, confidentiality, and bridge reuse
- separate local scripts for V2 smoke testing

### 12.4 Validation Tests

- full V2 workspace sanity suite
- all V2 integration tests pass
- local smoke harness covers both returning-creator refresh and first-time bootstrap
- batch bootstrap and insufficient-fanout reuse paths pass under automation
- extended V1 local regression suite passes
- V1 file integrity check passes
- minimum V1 code regression suite passes

### 12.5 V1 Preservation Instructions

- Do not append V2 cases into existing V1 integration test files.
- Do not edit V1 docker-compose topology for V2 tests.
- Keep V2 smoke harness fully separate under `prototype/gbn-bridge-proto/`.

### 12.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 9: Test Harness. Do not proceed to the next phase 10 without getting explicit approval.

Create a V2-local integration harness and test suite for bridge mode. Add isolated integration tests for bridge registration, catalog refresh, first-time creator bootstrap, UDP punch ACKs, batch onboarding, creator failover, bridge reuse, payload confidentiality, and reachability filtering. Add a V2-local docker-compose smoke environment and a V2-local test runner script.

Do not modify any V1 test file, V1 docker-compose file, or V1 validation script. V2 tests must live entirely under prototype/gbn-bridge-proto/.

Do not modify the main repo README.md during this phase. Keep README.md pinned to the published Lattice release-facing content, and defer any V2 README updates until all V2 code changes are complete and explicitly approved as a separate documentation pass.

If root-level V2 integration tests are used, make the minimal V2-local Cargo changes required to ensure those tests actually run under `cargo test --workspace`. Do not leave the Phase 9 harness as inert files.
```

### 12.7 Detailed Execution Reference

Use [GBN-PROTO-005-Execution-Phase9-V2-Test-Harness](GBN-PROTO-005-Execution-Phase9-V2-Test-Harness.md) as the implementation checklist and current execution record for this phase. It expands the Phase 9 scope into current repo findings, Cargo harness execution constraints, test-boundary decisions, smoke-topology assumptions, validation fallback strategy, executed validation results, risks, and sign-off criteria.

---

## 13. Phase 10 - AWS Prototype Deployment

### 13.1 Objective

Deploy the V2 bridge-mode system to AWS, including HostCreator-assisted bootstrap and UDP punch validation, without affecting V1 infrastructure.

Phase 10 is implemented locally from the committed Phase 9 test-harness baseline. The V2-only Dockerfiles, CloudFormation template, deployment scripts, and deployment entrypoints now exist. Live AWS deployment validation and the extended V1 AWS regression suite remain pending because they require Docker, AWS credentials, and target VPC/subnet inputs.

### 13.2 Files To Create Or Modify

Create:

- `prototype/gbn-bridge-proto/Dockerfile.bridge`
- `prototype/gbn-bridge-proto/Dockerfile.bridge-publisher`
- `prototype/gbn-bridge-proto/infra/cloudformation/phase2-bridge-stack.yaml`
- `prototype/gbn-bridge-proto/infra/cloudformation/parameters.json`
- `prototype/gbn-bridge-proto/infra/scripts/build-and-push.sh`
- `prototype/gbn-bridge-proto/infra/scripts/bootstrap-smoke.sh`
- `prototype/gbn-bridge-proto/infra/scripts/deploy-bridge-test.sh`
- `prototype/gbn-bridge-proto/infra/scripts/status-snapshot.sh`
- `prototype/gbn-bridge-proto/infra/scripts/teardown-bridge-test.sh`
- `prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh`
- `prototype/gbn-bridge-proto/infra/README-infra.md`

May modify:

- V2 runtime and publisher configs needed for deployment

Must not modify:

- `prototype/gbn-proto/infra/cloudformation/**`
- `prototype/gbn-proto/infra/scripts/**`
- V1 Dockerfiles

### 13.3 Deliverables

- separate V2 CloudFormation stack
- separate V2 container images
- separate V2 deploy, bootstrap-smoke, status, and teardown scripts
- deployment wiring for HostCreator-assisted bootstrap, default UDP punch port, and publisher batch-window configuration
- no naming collision with V1 stacks, repos, or services

### 13.4 Validation Tests

- V2 deployment succeeds using V2-only stack and image names
- bridge registration succeeds in AWS
- first-time creator can reach the Publisher through HostCreator and establish a seed bridge in AWS
- creator receives bootstrap bridge entries and begins punch fanout in AWS
- returning creator can attach and refresh catalog in AWS
- upload path succeeds through a bridge in AWS
- extended V1 AWS regression suite passes before phase sign-off
- V1 file integrity check passes
- minimum V1 code regression suite passes

### 13.5 V1 Preservation Instructions

- Never modify `prototype/gbn-proto/infra/scripts/deploy-scale-test.sh`.
- Never modify `prototype/gbn-proto/infra/scripts/build-and-push.sh`.
- Never reuse a V1 stack name, ECS service name, or ECR repo name for V2.
- Keep all V2 env vars under the `GBN_BRIDGE_` prefix.

### 13.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 10: AWS Prototype Deployment. Do not proceed to the next phase 11 without getting explicit approval.

Create V2-only deployment assets for bridge mode. Add new Dockerfiles, CloudFormation templates, and deploy/bootstrap-smoke/status/teardown scripts under prototype/gbn-bridge-proto/. Deploy the V2 system with unique stack names, image names, and environment variable prefixes. Validate bridge registration, first-time bootstrap through HostCreator, catalog refresh, and upload flow in AWS.

Do not modify any V1 infra script, CloudFormation template, or Dockerfile. V2 deployment assets must be fully isolated from V1.

Do not modify the main repo README.md during this phase. Keep README.md pinned to the published Lattice release-facing content, and defer any V2 README updates until all V2 code changes are complete and explicitly approved as a separate documentation pass.
```

### 13.7 Detailed Execution Reference

Use [GBN-PROTO-005-Execution-Phase10-V2-AWS-Prototype-Deployment](GBN-PROTO-005-Execution-Phase10-V2-AWS-Prototype-Deployment.md) as the implementation checklist and current execution record for this phase. It expands the Phase 10 scope into current repo findings, V2-only naming decisions, AWS deployment boundaries, validation fallback strategy, live-AWS sign-off requirements, and known blockers.

---

## 14. Phase 11 - Mobile-Network Validation

### 14.1 Objective

Validate whether bridge mode actually survives realistic mobile-network conditions, including first-time bootstrap and progressive bridge fanout.

Phase 11 is implemented locally from the committed Phase 10 AWS-prototype baseline. V2-only mobile validation scripts, a mobile scenario matrix, and a current test-results document now exist. The remaining gap is live AWS/mobile measurement of bootstrap, punch, failover, batching, and churn behavior.

### 14.2 Files To Create Or Modify

Create:

- `docs/prototyping/GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign-Test.md`
- `prototype/gbn-bridge-proto/infra/scripts/mobile-validation.sh`
- `prototype/gbn-bridge-proto/infra/scripts/collect-bridge-metrics.sh`
- `prototype/gbn-bridge-proto/docs/mobile-test-matrix.md`

May modify:

- V2 infra README
- V2 metrics configuration files

Must not modify:

- V1 mobile, scale-test, or AWS scripts

### 14.3 Deliverables

- mobile or mobile-like validation matrix
- measured behavior for:
  - app restart
  - IP churn
  - network switch
  - stale catalog recovery
  - bridge failure recovery
  - first-time bootstrap success rate and latency
  - coordinated UDP punch success on default port `443` and any signed overrides
  - batched onboarding latency for 10-request and 11th-request rollover cases
- documented results and unresolved gaps

### 14.4 Validation Tests

- creator reconnects using cached catalog after app restart
- creator recovers from at least one stale bridge entry
- first-time creator reaches the Publisher through a HostCreator path under mobile conditions
- creator and seed bridge complete bidirectional tunnel ACK on default port `443` unless overridden
- creator refreshes catalog successfully through any reachable bridge
- creator reuses already-live bridges when full fanout is unavailable after churn
- batch onboarding stays within the accepted latency threshold for 10-request windows and 11th-request rollover
- bridge failover latency stays within the accepted threshold documented for this phase
- V2 workspace sanity suite
- V1 file integrity check passes
- minimum V1 code regression suite passes

### 14.5 V1 Preservation Instructions

- Keep all mobile validation artifacts under V2 docs and V2 scripts.
- Do not retrofit V1 deployment scripts to run V2 mobile experiments.

### 14.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 11: Mobile-Network Validation. Do not proceed to the next phase 12 without getting explicit approval.

Add V2-only mobile-network validation tooling and documentation. Measure app restart recovery, first-time bootstrap, UDP punch success, stale bridge handling, network change behavior, batched onboarding, and bridge failover using the V2 AWS deployment or equivalent realistic test environment. Record results in a dedicated V2 test-results document.

Do not change V1 deployment or validation tooling. This phase is about measuring V2 behavior, not rewriting V1 scripts.

Do not modify the main repo README.md during this phase. Keep README.md pinned to the published Lattice release-facing content, and defer any V2 README updates until all V2 code changes are complete and explicitly approved as a separate documentation pass.
```

### 14.7 Detailed Execution Reference

Use [GBN-PROTO-005-Execution-Phase11-V2-Mobile-Network-Validation](GBN-PROTO-005-Execution-Phase11-V2-Mobile-Network-Validation.md) as the implementation checklist and current execution record for this phase. It expands the Phase 11 scope into current repo findings, local-vs-live evidence boundaries, validation commands, executed results, and remaining blockers.

---

## 15. Phase 12 - Decision Gate

### 15.1 Objective

Conclude whether V2 bridge mode should remain experimental, coexist long-term with V1 onion mode, or become the preferred mobile transport path, based on measured bootstrap viability, coordinated UDP punching, progressive upload, and onboarding scalability.

Phase 12 is implemented. The current decision is that Conduit remains experimental and Lattice remains the baseline and release-facing transport mode. This decision is based on strong local implementation and harness evidence together with the still-open live AWS/mobile validation gaps from Phases 10 and 11.

### 15.2 Files To Create Or Modify

Create:

- `docs/architecture/GBN-ARCH-007-Transport-Mode-Coexistence.md`
- `docs/prototyping/GBN-PROTO-005-Decision-Record.md`

May modify:

- `docs/architecture/GBN-ARCH-000-System-Architecture-V2.md`
- `docs/architecture/GBN-ARCH-001-Media-Creation-Network-V2.md`
- `docs/prototyping/GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign.md`
- this execution plan

Must not modify:

- V1 architecture docs to overwrite historical V1 behavior
- V1 implementation code

### 15.3 Deliverables

- explicit coexistence decision:
  - V2 remains experimental
  - V2 becomes alternate transport mode
  - V2 becomes default mobile transport mode
- explicit decision against the major prototype assumptions and exit criteria from the redesign document
- migration rules and ownership boundaries
- updated architecture docs reflecting the final decision
- documented unresolved risks around first-contact bootstrap, punch success rate, batching, and weak-discovery trust boundaries

### 15.4 Validation Tests

- all prior phase validations are documented as complete
- V2 test results are attached or linked
- decision record explicitly addresses:
  - first-time bootstrap viability
  - direct tunnel establishment success rates
  - insufficient-fanout bridge reuse behavior
  - publisher batch onboarding scalability
  - weak discovery trust boundaries
- minimum V1 code regression suite passes one final time
- extended V1 AWS regression suite passes if V2 infra work was merged
- V1 file integrity check passes

### 15.5 V1 Preservation Instructions

- Do not rewrite V1 documents to pretend V2 replaced them historically.
- V1 remains the baseline implementation unless and until a separate approved migration plan is executed.

### 15.6 Phase Prompt

```text
Ensure the previous phase was completed fully before proceeding and all its validation tests have passed. Only implement the changes in this phase 12: Decision Gate. Do not proceed to any follow-on migration or refactor work without getting explicit approval.

Produce the final GBN-PROTO-005 decision record and coexistence architecture notes. Summarize what was built, what tests passed, how first-time bootstrap and coordinated UDP punching performed, what risks remain, and whether bridge mode should remain experimental, coexist with V1 onion mode, or become the default mobile transport path. Update only V2 documents needed to record that decision.

Do not modify V1 code. Do not overwrite V1 architecture history. This phase is documentation and decision-making only.

Do not modify the main repo README.md during this phase. Keep README.md pinned to the published Lattice release-facing content, and defer any V2 README updates until all V2 code changes are complete and explicitly approved as a separate documentation pass.
```

### 15.7 Detailed Execution Reference

Use [GBN-PROTO-005-Execution-Phase12-V2-Decision-Gate](GBN-PROTO-005-Execution-Phase12-V2-Decision-Gate.md) as the implementation checklist and current execution record for this phase. It expands the Phase 12 scope into the current evidence boundary, recorded decision, validation results, and the remaining blockers that keep Conduit in experimental status.

---

## 16. Quick Reference - Phase Completion Checklist

Use this checklist at the end of every phase:

1. Confirm the current phase scope is complete.
2. Run all phase-specific validation tests.
3. Run the minimum V1 code regression suite.
4. Run any extended V1 regression suite required by that phase.
5. Confirm the V1 file integrity check is clean.
6. Record results in the phase notes or commit message.
7. Stop and obtain explicit approval before moving to the next phase.

## 17. Recommended Order Of Actual Implementation

If the phases need to be grouped into practical work batches, use:

1. Documentation and isolation:
   - Phase 0
   - Phase 1
   - Phase 2
2. Core runtime:
   - Phase 3
   - Phase 4
   - Phase 5
   - Phase 6
3. Discovery and policy:
   - Phase 8
   - Phase 7
4. Verification and deployment:
   - Phase 9
   - Phase 10
   - Phase 11
5. Final architecture decision:
   - Phase 12

This ordering preserves the V1 implementation while allowing the V2 bridge-mode design to be built, validated, and reviewed incrementally.
