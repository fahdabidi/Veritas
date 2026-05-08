# GBN-PROTO-012 - Conduit Architecture-Correct Bootstrap Execution Plan (Pass 3)

**Document ID:** GBN-PROTO-012
**Status:** Pending
**Last Updated:** 2026-05-08
**Related Docs:**
[GBN-ARCH-001-V2 Media Creation Network](../../../architecture/GBN-ARCH-001-Media-Creation-Network-V2.md),
[GBN-PROTO-007 Pass 2 V2-V1 Parity](../Full-Implementation-Plan-Pass2/GBN-PROTO-007-Conduit-V2-V1-Parity-Execution-Plan.md),
[GBN-PROTO-008 Local Kubernetes Test Infrastructure](../Full-Implementation-Plan-Pass2/GBN-PROTO-008-Local-Kubernetes-Test-Infrastructure-Execution-Plan.md),
[GBN-PROTO-009 Discovery Smoke Plan](../Full-Implementation-Plan-Pass2/GBN-PROTO-009-Local-Kubernetes-Discovery-Smoke-Test-Plan.md),
[GBN-PROTO-010 Route Smoke Plan](../Full-Implementation-Plan-Pass2/GBN-PROTO-010-Local-Kubernetes-Creator-Publisher-Route-Smoke-Test-Plan.md)

Pass 3 corrects the remaining Conduit V2 architectural gap: Pass 2 added operator admin
surfaces and a synthetic creator library, but first-time creator onboarding is still not
implemented as documented in `GBN-ARCH-001-V2` section 3.3.

Current state:

- `relay-control-interactive-v2.sh` has `SendDummy`, `TriggerCommand`, and
  `BootstrapSmoke`, but no command that seeds a HostCreator or NewCreator.
- `POST /v1/admin/send-dummy` and `POST /v1/admin/discovery-probe` call the Publisher
  authority directly using preconfigured `GBN_BRIDGE_AUTHORITY_URL`.
- The synthetic join request currently sets `host_creator_id` and `relay_bridge_id` from
  the same selected actor. It does not model a distinct `HostCreator`, `NewCreator`, or
  `ExitBridgeA`.
- There is no per-node local DHT / discovery table that records Publisher-signed creator
  and bridge entries after onboarding.
- `SendDummy` does not require the selected node to have completed NewCreator onboarding.

Pass 3 replaces the synthetic shortcut with the documented flow:

0. Phase 0 deploys two dedicated creator pods (`creator-host`, `creator-new`) and
   scales ExitBridge replicas from 3 to 10 so the Publisher can seed a full 10-entry
   signed ExitBridge DHT set during bootstrap. Without real creator pods, "select a
   HostCreator node" and "select a NewCreator node" have no targets.
1. Operator runs `SeedHostCreator`, selects the `creator-host` pod, and provides one
   ExitBridge plus Publisher DHT metadata to make that node ready to act as
   `HostCreator`.
2. Operator runs `SeedNewCreator`, selects the `creator-new` pod, and provides the
   HostCreator DHT metadata from step 1.
3. The NewCreator starts section 3.3 first-time bootup through HostCreator and
   ExitBridgeA.
4. The Publisher issues signed bootstrap entries and selects ExitBridgeB.
5. The Publisher returns the bootstrap response back through the same path
   (`Publisher → ExitBridgeA → HostCreator → NewCreator`) so the NewCreator learns
   ExitBridgeB's identity and starts the receive-side of the punch (per §3.3 step 6).
6. ExitBridgeB returns the seeded bridge set to NewCreator.
7. NewCreator stores signed entries in its local DHT / discovery table and marks
   entries active after reachability ACKs.
8. `SendDummy` can only run on onboarded NewCreators, must construct routes from that
   local DHT / discovery table, and must envelope-encrypt the dummy frame for the
   Publisher so bridges only see opaque ciphertext (§3.5 boundary).

## Status Trackers

- `[ ]` Pending
- `[/]` In Progress
- `[x]` Completed

| Phase | Title | Status |
|---|---|---|
| 0 | Creator Pod Deployment And Cluster Topology | `[x]` |
| 1 | Creator Local State And DHT Metadata Model | `[x]` |
| 2 | SeedHostCreator Admin API And Operator Command | `[x]` |
| 3 | SeedNewCreator API And First-Contact Join Path | `[x]` |
| 4 | Bootstrap Payload Delivery, Local DHT Population, And Punch Fanout | `[x]` |
| 5 | Onboarded-Creator SendDummy And Local-DHT Single-Lane Envelope Demo | `[x]` |
| 6 | Operator Scripts And Acceptance Gate | `[ ]` |
| 7 | Smoke 1 — Tracing Suite Implementation | `[ ]` |
| 8 | Smoke 2 — Discovery / Bootup Suite Implementation | `[ ]` |
| 9 | Smoke 3 — Route And Encryption Boundary Suite Implementation | `[ ]` |
| 10 | Upload Session Build And Per-Chunk Encryption Pipeline (§3.4 + §3.5) | `[ ]` |
| 11 | Multi-Lane Progressive Fanout (§3.6 + §3.7) | `[ ]` |
| 12 | Smoke 4 — Full Upload Pipeline Suite Implementation | `[ ]` |

Each phase must update this status tracker when completed.

---

## 1. Gap Inventory

| Gap | Current Behavior | Required Pass 3 Behavior | Phase |
|---|---|---|---|
| No dedicated creator pods | Synthetic creator runs in-process inside Publisher binaries; no `creator-host` / `creator-new` exist | Two dedicated creator deployments (`creator-host`, `creator-new`) running `creator-runner` binary with admin listener | 0 |
| Only 3 ExitBridge replicas | Local k3d and CloudFormation deploy 3 bridges | Scale to 10 (1 ExitBridgeA relay + 9 in bootstrap set per §3.3 step 3) | 0 |
| WSL2 host allocation undocumented | `~/.wslconfig` may have insufficient memory/CPU for 20-pod cluster | Documented `memory=10GB processors=6 swap=4GB` baseline with verification | 0 |
| Bridge descriptor fields incomplete | Only `node_id`, `ip_addr`, `pub_key`, `udp_punch_port`, `entry_expiry_ms`, `publisher_sig`, `active` | Full §4.1 set: `bridge_id`, `identity_pub`, `ingress_endpoints[]`, `udp_punch_port`, `reachability_class`, `lease_expiry_ms`, `entry_expiry_ms`, `capabilities[]`, `publisher_sig`, `active` | 1 |
| No local creator DHT state | Creator calls return transient catalog/bootstrap data only | Every potential creator has container-local persisted DHT/discovery state with Publisher-signed entries (per Pass 3 D1: survives container restart, not cluster destroy) | 1 |
| Local DHT concurrency model under-specified | "thread-safe" only | Single-writer task on dedicated thread; reads via RwLock snapshot; mutations enqueued through command channel | 1 |
| No node metadata dump suitable for seed commands | Operator infers task/pod IPs and authority registry data | Every node exposes role-tagged self metadata needed for DHT entries and seed payloads | 1 |
| Trust root semantics ambiguous | Phase 1 says "publisher_sig OR configured trust-root marker" | Publisher entries are validated against the locally configured Publisher pubkey trust root (no `publisher_sig` field). Bridge and creator entries carry `publisher_sig` validated against that trust root | 1 |
| `bootstrap_session_id` referenced but not modeled | Phase 4 logs use it but Phase 1 schema lacks it | `LocalDiscoveryTable.current_bootstrap_session: Option<BootstrapSession>` with `session_id`, `started_at_ms`, `last_event_ms`, `last_state` | 1 |
| No reset / rollback path | A `failed` onboarding state is a dead-end | `POST /v1/admin/reset-creator-state` admin endpoint and `ResetCreatorState` operator command | 1 |
| `local-dht` semantics on non-creator pods unclear | Phase 1 says it works on authority/receiver/bridge | Creator pods return full table; Publisher and bridge pods return role-tagged `state=not_applicable` payload | 1 |
| No `SeedHostCreator` command | Relay-control script cannot prepare a HostCreator | Operator can select an onboarded creator node and inject Publisher + ExitBridgeA metadata | 2 |
| `SeedHostCreator` allows non-onboarded host | API stores seed without requiring host pod to be onboarded itself | Default: reject unless target's `self_onboarding_state=onboarded`. Test-only `bootstrap_genesis=true` flag installs pre-onboarded state for the very first HostCreator | 2 |
| Seed APIs not idempotent | Repeated calls undefined | Byte-identical retry returns same `chain_id`; differing payload returns 409 unless `force=true` | 2, 3 |
| No `SeedNewCreator` command | Relay-control script cannot supply HostCreator metadata to a NewCreator | Operator can select a NewCreator and trigger bootup through a seeded HostCreator | 3 |
| Join request bypasses HostCreator and ExitBridgeA | Synthetic creator calls Publisher directly | NewCreator requests join through HostCreator; HostCreator relays via ExitBridgeA to Publisher | 3 |
| Publisher-selected ExitBridgeB is not used as seed bridge handoff | Bootstrap result returns directly to synthetic caller | Publisher sends bootstrap payload to ExitBridgeB; ExitBridgeB ACKs and starts punch toward NewCreator | 4 |
| Bootstrap response return path missing | Phase 4 jumps from "ExitBridgeB ACKs" straight to "NewCreator receives seed bridge" | Publisher returns bootstrap response back through `Publisher → ExitBridgeA → HostCreator → NewCreator` so NewCreator learns ExitBridgeB identity (per §3.3 step 6) | 4 |
| Bidirectional punch progress reporting missing | Only NewCreator-side ACK event | Both seed bridge and NewCreator emit `bootstrap_progress` events to Publisher (per §3.3 step 7) | 4 |
| Protocol-message reuse not specified | "Add or complete runtime surfaces for ..." without naming wire messages | Each sub-step explicitly maps to a §5.2 / §5.3 message name; no overloading of existing messages | 4 |
| Failure recovery missing | `failed` is a dead-end state; no timeouts; no suspect-bridge marking | Seed-tunnel and fanout timeouts; `seed_tunnel_failed`, `fanout_partial`, `fanout_failed` transitions; `mark_bridge_suspect` action with TTL | 4 |
| Local DHT entries are not persisted or activated | Scripts compare authority registry and pod names | NewCreator stores signed entries to container-local disk and marks entries active after tunnel / ACK | 4 |
| `SendDummy` can run from any admin listener | Selected actor need not be onboarded | `SendDummy` errors unless selected node has NewCreator onboarding state | 5 |
| `SendDummy` route source is not visible | Assigned bridge comes from direct authority bootstrap | Response and traces show route source is local DHT / discovery state | 5 |
| `relay_only` bridges not filtered from creator ingress | Phase 5 selects from active bridges without checking reachability class | Reject `reachability_class=relay_only` from creator ingress selection (per §4.2) | 5 |
| Encryption boundary not exercised | `send-dummy` sends raw plaintext | Dummy frame encrypted for Publisher (X25519 + HKDF + AEAD) before crossing bridge; bridge sees only ciphertext (§3.5 / §6 / §9.2 trust boundary) | 5 |
| Failover not tested | Single-bridge happy path only | `force_bridge_failure: bool` debug flag triggers mid-flight bridge-suspect marking and second-bridge selection (§7.1, §9.1) | 5 |
| Smoke 2/3 plans describe desired behavior but scripts do not enforce it yet | `discovery-probe` and `send-dummy` are baseline shortcuts | New Pass 3 smoke plans (`GBN-PROTO-012-Smoke-1/2/3`) supersede Pass 2 `GBN-PROTO-009/010/011` and validate real bootup, local-DHT routing, and bridge-cannot-decrypt | 6 |
| Smoke 1 not updated for new tracing events | Pass 2 Smoke 1 does not assert the 16 new bootup events | Smoke 1 successor asserts all 16 events appear in Tempo for at least one bootstrap session | 6 |
| `send-dummy` breaking change has no migration | Pass 2 callers may target publisher/bridge pods | Enumerate all call sites; redirect to `creator-host` / `creator-new`; remove non-creator call sites with no shim | 6 |
| Operator scripts diverge with no harmonization | `relay-control-interactive-v2.sh` (AWS) and `k8s-control-interactive.sh` (k8s) duplicate menu logic | Extract shared menu actions into `_seed_actions.sh`; top-level scripts only differ in transport | 6 |
| §3.4 Pre-processing pipeline missing | No sanitizer, chunker, manifest builder, or session builder exists | Sanitizer strips EXIF / container metadata / encoder ids / normalizes timestamps; chunker emits fixed-size chunks with per-chunk plaintext_hash; manifest builder computes content_hash; session builder issues session_id + loads trust root + loads local DHT | 10 |
| §3.5 Per-chunk encryption missing | Phase 5 only encrypts a single dummy frame | Per-chunk X25519 + HKDF + AES-256-GCM with AAD = `session_id || chunk_index || total_chunks || plaintext_hash`; bridge cannot decrypt any chunk | 10 |
| §3.6 Multi-lane upload missing | Phase 5 SendDummy uses one bridge | Creator selects N upload lanes from local DHT (target N=10), opens `BridgeOpen` per lane, disperses chunks across lanes, tracks per-chunk per-lane ACK | 11 |
| §3.7 Progressive fanout missing | No lane-becomes-active staged dispatch; no reuse when fewer than 10 lanes | Chunks start flowing as each lane becomes active; reuse already-active lanes if fewer than 10 active before timeout; failover reroutes pending chunks to remaining active lanes | 11 |
| Full upload pipeline not validated | No smoke covers sanitize → chunk → encrypt → multi-lane → receiver reconstruct | New `k8s-smoke-upload-v3.sh` (Smoke 4) drives `BuildUploadSession` + `SendUpload`; asserts content_hash match, ciphertext-only at every bridge, ≥ 2 distinct lanes used, progressive timeline | 12 |

---

## 2. Execution Rules

### 2.1 Architecture-First Rule

Pass 3 implements `GBN-ARCH-001-V2` section 3.3, 3.6, and 3.7 as written. If an
implementation shortcut is needed for local k8s or AWS ECS, the shortcut must be explicit
in the phase doc and must preserve the observable contract:

- NewCreator receives external HostCreator metadata.
- HostCreator has external Publisher + ExitBridgeA metadata.
- Publisher signs bootstrap entries.
- NewCreator stores those entries locally.
- Route construction uses NewCreator local state, not shell-side inference.

### 2.2 Admin Isolation Rule

All operator-triggered seed APIs remain on the localhost-only admin listener:
`127.0.0.1:9090` in ECS and `0.0.0.0:9090` only inside local k8s pods. Access stays via
ECS Exec or k8s pod-network transport. No public admin ingress is introduced.

### 2.3 Local DHT Truth Rule

`GET /v1/admin/local-dht` must return the selected node's actual local creator discovery
state. It must not synthesize records from Kubernetes pod names, ECS task lists, or a live
authority registry query at request time.

### 2.4 SendDummy Gating Rule

`POST /v1/admin/send-dummy` must fail with a specific error if the selected node is not
onboarded as a NewCreator:

```json
{
  "error": {
    "code": "creator_not_onboarded",
    "message": "selected node has not completed NewCreator onboarding"
  }
}
```

### 2.5 Traceability Rule

Every major bootup transition emits `chain_id`-tagged logs and spans. The full set is
16 events, covering forward path, return path, both-side punch progress, and dummy
delivery:

Forward path (NewCreator → HostCreator → ExitBridgeA → Publisher):

- host seed stored
- new creator seed stored
- join requested
- host relayed join via ExitBridgeA
- Publisher issued bootstrap payload

Return path (Publisher → ExitBridgeA → HostCreator → NewCreator), per `GBN-ARCH-001-V2`
section 3.3 step 6:

- publisher response to host via bridge
- host response received from bridge
- host relayed response to new creator
- new creator bootstrap response received

Seed bridge punch and bidirectional progress (per section 3.3 steps 5 and 7, "both
sides notify the Publisher"):

- ExitBridgeB accepted seed payload
- seed bridge punch progress to publisher
- new creator punch progress to publisher
- new creator received seed bridge
- bridge set returned
- local DHT updated
- reachability ACK recorded

Onboarded-creator single-lane envelope demo (Phase 5, sections 3.5 / 3.6):

- route selected from local DHT
- dummy frame delivered

Upload pipeline (Phase 10 / Phase 11, sections 3.4, 3.5, 3.6, 3.7):

- creator upload session built (sanitization, chunking, manifest hash done)
- creator upload lanes selected (multi-lane plan from local DHT)
- creator upload lane open (one event per lane × N)
- creator upload chunk encrypted (one event per chunk)
- creator upload chunk dispatched (one event per chunk × bridge)
- creator upload lane reused (when fewer than 10 active lanes — §3.7 reuse)
- creator upload lane failover (when a lane fails mid-session — §7.1)
- bridge upload chunk forwarded (per chunk × bridge)
- receiver upload chunk ingested (per chunk × bridge)
- receiver upload manifest received (once per session)
- publisher upload chunk ack returned (per chunk)
- creator upload session complete (once per session, when all chunks ACKed)

This brings the §2.5 event total to 36 (16 bootup + 8 single-lane envelope + 12 upload
pipeline), all `chain_id` and `bootstrap_session_id`/`upload_session_id` correlated.

### 2.6 V1 Preservation Rule

Pass 3 does not modify `prototype/gbn-proto/**`. V1 remains a reference only.

### 2.7 Phase Completion Rule

Each phase must finish with:

- targeted unit tests for new Rust behavior
- relevant shell syntax checks
- local-k8s smoke coverage when the phase touches runtime behavior
- status tracker update in this document

### 2.8 WSL2 Ubuntu Baseline Rule

All `cargo`, `bash`, `kubectl`, `k3d`, `docker`, and operator-script commands in Pass 3
phase docs are intended to be run inside WSL2 Ubuntu 22.04 or newer. Running these
commands from Windows-native PowerShell is not supported because shell scripts depend
on POSIX line endings, path separators, and tooling. Each Pass 3 test script begins
with the one-line guard:

```bash
uname -a | grep -i microsoft >/dev/null || { echo "Pass 3 tooling requires WSL2 Ubuntu" >&2; exit 1; }
```

The minimum WSL2 host allocation for Pass 3 (10 ExitBridges + 2 creator pods +
Publisher surfaces + Postgres + observability) is documented in Phase 0:
`memory=10GB`, `processors=6`, `swap=4GB` in `~/.wslconfig` on the Windows host.

### 2.9 Pass-2 Smoke Plan Successor Rule

Pass 2 introduced three smoke-test plans (`GBN-PROTO-009`, `010`, `011` under
`Full-Implementation-Plan-Pass2/`). Pass 3 changes the assertions those smoke runs
must enforce (real architecture-correct bootstrap, local-DHT route construction,
expanded tracing). To keep Pass 2 docs frozen as historical record, Pass 3 supersedes
them with new Pass-3-folder documents (`GBN-PROTO-012-Smoke-1-Tracing.md`,
`GBN-PROTO-012-Smoke-2-Discovery.md`, `GBN-PROTO-012-Smoke-3-Route.md`). The Pass 2
files are not edited by Pass 3.

---

## 3. Locked Decisions

### 3.1 Operator Command Names

The operator-facing commands are:

- `SeedHostCreator`
- `SeedNewCreator`
- `DumpLocalDht`
- `SendDummy` — single-lane envelope demo (Phase 5)
- `BuildUploadSession` — sanitize + chunk + manifest + per-chunk encrypt (Phase 10)
- `SendUpload` — multi-lane progressive fanout for an existing session (Phase 11)
- `ResetCreatorState`
- `CollectTraces`

`SendDummy` remains the familiar single-frame demonstration but its behavior changes:
it now requires an onboarded NewCreator and routes through local DHT / discovery state.
`SendUpload` is the architecture-correct full-upload path that exercises §3.4–§3.7
end-to-end.

### 3.2 SeedHostCreator Input

`SeedHostCreator` injects exactly one Publisher entry and one ExitBridgeA entry into the
selected HostCreator. The operator script gathers these from live node metadata and
registered bridge metadata.

### 3.3 SeedNewCreator Input

`SeedNewCreator` injects the HostCreator entry into the selected NewCreator. It does not
inject the Publisher or bridge set directly into NewCreator. That information must arrive
through the bootup workflow.

### 3.4 Local Kubernetes And AWS Parity

The Rust APIs are identical across local k8s and AWS ECS. The scripts differ only in how
they reach the admin listener and discover live node metadata.

### 3.5 Publisher Deployment Note

`GBN-ARCH-001-V2` section 3.6 names a single protocol role: **Publisher**. The current
Conduit deployment runs that role as two cooperating processes named
`publisher-authority` (lease signing, bridge registry, catalog issuance) and
`publisher-receiver` (payload sink, ACK emission). This split is a deployment-time
horizontal optimization, not a separate architectural role.

For the rest of this plan, "Publisher" means the single §3.6 role. When a step needs
to disambiguate which surface, the wording is "Publisher (authority surface)" or
"Publisher (receiver surface)". Operator scripts list both processes but treat them as
one Publisher for selection purposes — the operator is never asked to "choose between
authority and receiver" because there is only one Publisher to choose. Merging the
two binaries into one process is out of scope for Pass 3 and tracked separately.

### 3.6 Cluster Topology After Pass 3

Phase 0 establishes the topology that the rest of the phases depend on. Final pod /
task count after Pass 3 (matches `GBN-ARCH-001-V2` section 3.3 exactly):

| Role | k8s replicas | ECS task count |
|---|---|---|
| Publisher (authority surface) | 1 | 1 |
| Publisher (receiver surface) | 1 | 1 |
| ExitBridge | 10 | 10 |
| Creator (HostCreator candidate) | 1 (`creator-host`) | 1 |
| Creator (NewCreator candidate) | 1 (`creator-new`) | 1 |

The 10 ExitBridges break down as: 1 ExitBridgeA acting as the HostCreator's relay path
to the Publisher, plus the full 10-entry Publisher-seeded bridge DHT set returned to
the NewCreator during bootstrap. One non-ExitBridgeA entry is still selected as
ExitBridgeB for the seed handoff.

---

## 4. Phase Summaries

### Phase 0 - Creator Pod Deployment And Cluster Topology

[GBN-PROTO-012-Execution-Phase0-Creator-Pod-Deployment.md](GBN-PROTO-012-Execution-Phase0-Creator-Pod-Deployment.md)

Add dedicated `creator-host` and `creator-new` deployments, scale ExitBridge replicas
to 10, set per-pod resource requests and limits, document WSL2 host allocation, and
introduce container-local persistence via PVCs (k8s) or EFS volumes (ECS) per Pass 3 D1.

### Phase 1 - Creator Local State And DHT Metadata Model

[GBN-PROTO-012-Execution-Phase1-Creator-Local-State-And-DHT-Metadata.md](GBN-PROTO-012-Execution-Phase1-Creator-Local-State-And-DHT-Metadata.md)

Add local creator discovery state, DHT entry types, node metadata diagnostics, and
`GET /v1/admin/local-dht`. This phase creates the state model but does not yet execute the
full bootup workflow.

### Phase 2 - SeedHostCreator Admin API And Operator Command

[GBN-PROTO-012-Execution-Phase2-SeedHostCreator-Admin-API-And-Operator-Command.md](GBN-PROTO-012-Execution-Phase2-SeedHostCreator-Admin-API-And-Operator-Command.md)

Add `POST /v1/admin/seed-host-creator`, persist HostCreator seed state, and add
`SeedHostCreator` to the AWS and local k8s control scripts.

### Phase 3 - SeedNewCreator API And First-Contact Join Path

[GBN-PROTO-012-Execution-Phase3-SeedNewCreator-API-And-First-Contact-Join.md](GBN-PROTO-012-Execution-Phase3-SeedNewCreator-API-And-First-Contact-Join.md)

Add `POST /v1/admin/seed-new-creator`, NewCreator seed state, and the NewCreator ->
HostCreator -> ExitBridgeA -> Publisher join path.

### Phase 4 - Bootstrap Payload Delivery, Local DHT Population, And Punch Fanout

[GBN-PROTO-012-Execution-Phase4-Bootstrap-Payload-Delivery-Local-DHT-And-Fanout.md](GBN-PROTO-012-Execution-Phase4-Bootstrap-Payload-Delivery-Local-DHT-And-Fanout.md)

Complete the Publisher -> ExitBridgeB -> NewCreator seed payload path, bridge set
request/response, local DHT population, and reachability ACK activation.

Completed 2026-05-08. The Publisher now maintains its own signed bridge DHT view for
all 10 active ExitBridges, exposes an `InitializePublisherDht` operator command to
seed/rebuild that view before `SeedNewCreator`, and uses that Publisher DHT view when
building bootstrap payloads. The bootstrap reply carries the signed V2 creator entry
and all 10 Publisher-seeded bridge DHT entries, selects ExitBridgeB distinct from
ExitBridgeA, records local bootstrap progress, and the NewCreator stores the bridge set
as active local discovery state after the simulated smoke-test ACK path.

### Phase 5 - Onboarded-Creator SendDummy And Local-DHT Route Construction

[GBN-PROTO-012-Execution-Phase5-Onboarded-Creator-SendDummy-Local-DHT-Routing.md](GBN-PROTO-012-Execution-Phase5-Onboarded-Creator-SendDummy-Local-DHT-Routing.md)

Update `SendDummy` so it only runs from an onboarded NewCreator and constructs the upload
route from local DHT / discovery state as described in architecture sections 3.6 and 3.7.

Completed 2026-05-08. `SendDummy` now consumes the Publisher-seeded bridge entries
stored in the creator's local DHT during Phase 4; the admin endpoint no longer performs
a direct Publisher catalog/bootstrap shortcut. It rejects non-creator and non-onboarded
nodes, filters local DHT bridge entries, supports forced bridge-suspect failover, and
wraps the dummy frame in the Publisher-targeted encryption envelope so bridges only see
ciphertext.

### Phase 6 - Operator Scripts And Acceptance Gate

[GBN-PROTO-012-Execution-Phase6-Operator-Scripts-Smoke-Tests-And-Acceptance.md](GBN-PROTO-012-Execution-Phase6-Operator-Scripts-Smoke-Tests-And-Acceptance.md)

Update `relay-control-interactive-v2.sh`, `k8s-control-interactive.sh`, and introduce
the shared `_seed_actions.sh` library. Add the `ResetCreatorState` admin endpoint to
the menu. Land the AWS ECS Exec verification for the new creator pods. Define the
final Pass-3 acceptance gate (local-k8s + AWS walkthroughs).

This phase does **not** implement the smoke test scripts themselves — those land in
Phases 7, 8, and 9.

### Phase 7 - Smoke 1 - Tracing Suite Implementation

[GBN-PROTO-012-Smoke-1-Tracing.md](GBN-PROTO-012-Smoke-1-Tracing.md)

Implement `infra/scripts/k8s-smoke-tracing-v3.sh` plus the `/v1/admin/echo-chain-id`
endpoint on all four binaries. Validates that all 14 actor pods (2 Publisher
surfaces + 10 ExitBridges + 2 creators) emit Loki logs and Tempo spans for any
chain_id, and that Prometheus has fresh scrape samples from each. First gate of the
suite.

### Phase 8 - Smoke 2 - Discovery / Bootup Suite Implementation

[GBN-PROTO-012-Smoke-2-Discovery.md](GBN-PROTO-012-Smoke-2-Discovery.md)

Implement `infra/scripts/k8s-smoke-discovery-v3.sh`. Drives `SeedHostCreator` and
`SeedNewCreator` end-to-end against the Phase 0 cluster, polls
`GET /v1/admin/local-dht` until terminal state, and asserts the §3.3 architecture
flow: full local DHT population, distinct actor chain (4 distinct ids), all 16 §2.5
events present in Tempo, no legacy `discovery-probe` shortcut.

### Phase 9 - Smoke 3 - Route And Encryption Boundary Suite Implementation

[GBN-PROTO-012-Smoke-3-Route.md](GBN-PROTO-012-Smoke-3-Route.md)

Implement `infra/scripts/k8s-smoke-route-v3.sh`. Drives two SendDummy invocations on
`creator-new` (normal + `force_bridge_failure=true`) and asserts:
`route_source=local_dht`, two distinct `assigned_bridge_id`s, bridge-side ciphertext
only (plaintext marker grep returns empty), receiver persistence, and 16 §2.5 events
for the SendDummy windows.

### Phase 10 - Upload Session Build And Per-Chunk Encryption Pipeline

[GBN-PROTO-012-Execution-Phase10-Upload-Session-And-Per-Chunk-Encryption.md](GBN-PROTO-012-Execution-Phase10-Upload-Session-And-Per-Chunk-Encryption.md)

Implement `GBN-ARCH-001-V2` §3.4 pre-processing pipeline (sanitizer, chunker, manifest
builder, session builder) and §3.5 per-chunk envelope encryption. New admin endpoint
`POST /v1/admin/build-upload-session` returns a session id and chunk count. Bridges
still see only ciphertext.

### Phase 11 - Multi-Lane Progressive Fanout

[GBN-PROTO-012-Execution-Phase11-Multi-Lane-Progressive-Fanout.md](GBN-PROTO-012-Execution-Phase11-Multi-Lane-Progressive-Fanout.md)

Implement `GBN-ARCH-001-V2` §3.6 multi-lane upload route construction and §3.7
progressive fanout: chunks dispatched across multiple active bridges, lanes started
as bridges become reachable, lane reuse when fewer than 10 lanes active before
timeout, lane failover on mid-session bridge loss. New admin endpoint
`POST /v1/admin/send-upload`.

### Phase 12 - Smoke 4 - Full Upload Pipeline Suite Implementation

[GBN-PROTO-012-Smoke-4-Full-Upload.md](GBN-PROTO-012-Smoke-4-Full-Upload.md)

Implement `infra/scripts/k8s-smoke-upload-v3.sh`. Drives `BuildUploadSession` then
`SendUpload` against `creator-new`. Asserts: full content reconstruction at receiver
(content_hash matches), bridges saw only ciphertext for every chunk, ≥ 2 distinct
lanes used, progressive fanout timeline (chunks delivered before all lanes active),
all 12 upload-pipeline §2.5 events present in Tempo.

---

## 5. Full Pass 3 Acceptance Criteria

Pass 3 is complete when:

1. `SeedHostCreator` can prepare a selected node with exactly one Publisher entry and one
   ExitBridgeA entry.
2. `SeedNewCreator` can prepare a selected node with HostCreator metadata and trigger
   section 3.3 bootup.
3. NewCreator join traffic reaches the Publisher through HostCreator and ExitBridgeA.
4. Publisher selects ExitBridgeB and produces signed bootstrap entries.
5. ExitBridgeB returns the seeded bridge set to NewCreator.
6. NewCreator local DHT / discovery state contains:
   - its own Publisher-signed creator entry;
   - Publisher-signed bridge entries;
   - non-expired entry windows;
   - active flags for successful tunnels.
7. `GET /v1/admin/local-dht` dumps that actual local state.
8. `SendDummy` fails on non-onboarded nodes.
9. `SendDummy` succeeds on onboarded NewCreators and reports `route_source=local_dht`.
10. Smoke 1 validates tracing for all nodes.
11. Smoke 2 validates creator bootup and local DHT population for every selected potential
    creator.
12. Smoke 3 validates local-DHT route construction and single-frame envelope dummy
    delivery.
13. `BuildUploadSession` runs the full §3.4 pipeline: sanitizer strips identifiable
    metadata; chunker emits fixed-size chunks with per-chunk `plaintext_hash`; manifest
    builder produces a content_hash; session builder issues a `session_id`.
14. `SendUpload` runs the full §3.5–§3.7 pipeline: per-chunk envelope encryption,
    multi-lane dispatch across at least 2 distinct active bridges, progressive
    fanout (chunks start flowing before all lanes are active), lane reuse when fewer
    than 10 lanes active, and lane failover on mid-session bridge loss.
15. Smoke 4 validates the full upload pipeline end-to-end: receiver reconstructs the
    plaintext content and the content_hash matches, every bridge sees only
    ciphertext, ≥ 2 distinct lanes used, all 12 upload-pipeline §2.5 events present.
16. Local k8s validation passes for all four smoke runs.
17. AWS ECS operator walkthrough passes when AWS validation is requested.

---

## 6. Out Of Scope

The following are explicitly out of scope for Pass 3. They are listed by architecture
section so a future pass can pick them up without re-deriving the boundary.

§3.4 (pre-processing pipeline), §3.5 (per-chunk encryption fanout), §3.6 (multi-lane
upload), and §3.7 (progressive fanout) are all in scope for Pass 3 — see Phase 10
(`Upload Session Build And Per-Chunk Encryption Pipeline`) and Phase 11
(`Multi-Lane Progressive Fanout`). They moved into scope to avoid downstream
architecture-correctness drift; deferring them risks discovering interface gaps in a
later pass that force rework of Phases 1–5.

Out of scope:

- `GBN-ARCH-001-V2` §10.2 Promotion gate measurements: live AWS bootstrap latency,
  mobile-network churn behavior, network-switch behavior, real-condition UDP punch
  success rates, batch onboarding latency, extended V1 AWS regression after V2 merge.
- Optional visual anonymization in §3.4 sanitizer (face blurring, OCR redaction).
  Pass 3 sanitizer strips EXIF, container metadata, encoder/device identifiers, and
  normalizes timestamps; visual anonymization is left for a later pass that can
  reuse the same pipeline interface.
- Public creator mobile UI.
- Real media files at production scale (multi-GB). Pass 3 upload pipeline tests use
  small synthetic test files (≤ 1 MiB chunked into 10 chunks of ≤ 100 KiB each) to
  fit the WSL2 cluster's resource envelope.
- Cross-region Publisher failover.
- Public admin API exposure.
- Merging `publisher-authority` and `publisher-receiver` into one binary (per §3.5
  this is recognized but deferred).
- V1 Lattice source changes (§2.6).
