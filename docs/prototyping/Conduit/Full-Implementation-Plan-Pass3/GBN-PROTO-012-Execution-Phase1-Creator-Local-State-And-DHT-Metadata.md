# GBN-PROTO-012 - Execution Phase 1 - Creator Local State And DHT Metadata Model

**Status:** Completed
**Last Updated:** 2026-05-08
**Parent Plan:** [GBN-PROTO-012](GBN-PROTO-012-Conduit-Architecture-Correct-Bootstrap-Execution-Plan.md)
**Depends On:** Phase 0 (creator pods deployed; `creator-runner` binary present)

## Objective

Create the local state and metadata surfaces required for architecture-correct creator
bootstrapping. This phase does not execute the full bootup flow yet; it establishes the
state model that later phases must use.

At completion, each `creator-runner` pod can:

- expose its own node metadata for operator seeding;
- store HostCreator, NewCreator, creator-entry, and bridge-entry state locally;
- persist that state to its container-local PVC (per Pass 3 D1: state survives container
  restart, not cluster destroy);
- dump that state through `GET /v1/admin/local-dht`;
- distinguish "not onboarded", "host seeded", "new creator seeded", and "new creator
  onboarded" states, while keeping host-role membership orthogonal to self-onboarding;
- accept `POST /v1/admin/reset-creator-state` to clear back to `state=none`.

Publisher (authority surface), Publisher (receiver surface), and ExitBridge pods also
expose `GET /v1/admin/node-metadata` (for operator seed payload assembly) and
`GET /v1/admin/local-dht` (returning a deliberately-empty role-tagged response —
non-creator pods do not have local-DHT state).

Update the parent plan status tracker when this phase is complete.

---

## Required Data Model

Add shared Rust types under
`prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/dht.rs`:

- `BridgeIngressEndpoint`
- `PublisherDhtEntry`
- `BridgeDhtEntry`
- `CreatorDhtEntry`
- `HostCreatorSeedState`
- `NewCreatorSeedState`
- `BootstrapSession`
- `SelfOnboardingState`
- `HostRoleState`
- `LocalDiscoveryTable`

Field requirements:

| Type | Required Fields |
|---|---|
| `BridgeIngressEndpoint` | `kind` (enum: `direct`, `brokered`, `relay_only`), `ip_addr`, `port`, optional `ttl_ms` |
| `PublisherDhtEntry` | `node_id`, `authority_url`, `receiver_url`, `pub_key`, `entry_expiry_ms`. **No** `publisher_sig` field — Publisher does not sign its own entry; see Trust Root Rule below |
| `BridgeDhtEntry` | `bridge_id` (renamed from `node_id`), `identity_pub` (renamed from `pub_key`), `ingress_endpoints: Vec<BridgeIngressEndpoint>` (replaces single `ip_addr`), `udp_punch_port`, `reachability_class` (enum: `direct`, `brokered`, `relay_only`), `lease_expiry_ms`, `entry_expiry_ms`, `capabilities: Vec<String>`, `publisher_sig`, `active`, `suspect_until_ms: Option<u64>` |
| `CreatorDhtEntry` | `node_id`, `ip_addr`, `pub_key`, `udp_punch_port`, `entry_expiry_ms`, `publisher_sig`, `active` |
| `HostCreatorSeedState` | HostCreator actor id, Publisher entry, ExitBridgeA entry, seeded timestamp |
| `NewCreatorSeedState` | NewCreator actor id, HostCreator entry, seeded timestamp |
| `BootstrapSession` | `session_id`, `started_at_ms`, `last_event_ms`, `last_state` (string mirroring the §State Transitions list in Phase 4) |
| `SelfOnboardingState` | enum: `none`, `new_creator_seeded`, `bootstrapping`, `seed_bridge_assigned`, `seed_tunnel_active`, `bridge_set_received`, `fanout_in_progress`, `fanout_partial`, `onboarded`, `seed_tunnel_failed`, `fanout_failed` |
| `HostRoleState` | enum: `not_host`, `host_seeded` |
| `LocalDiscoveryTable` | `actor_id`, `self_onboarding_state`, `host_role_state`, `publisher_entry: Option<PublisherDhtEntry>`, `host_creator_entry: Option<CreatorDhtEntry>` (used only when this node is acting as NewCreator), `creator_entry: Option<CreatorDhtEntry>` (this node's own Publisher-signed creator entry), `bridge_entries: Vec<BridgeDhtEntry>`, `active_tunnels: Vec<TunnelState>`, `current_bootstrap_session: Option<BootstrapSession>`, `last_update_ms`, `last_error: Option<String>` |

`BridgeDhtEntry` carries the full `GBN-ARCH-001-V2` §4.1 DHT field set while the
existing lease-oriented `BridgeDescriptor` remains unchanged so Pass 1 lease handling
continues to work.

Use existing protocol signature helpers where possible.

---

### Trust Root Rule (T1.2)

`GBN-ARCH-001-V2` §6.1: "Creator trusts Publisher key material out-of-band." The
Publisher does not sign its own entry — that would be a self-signed root and would
violate the trust model. Validation rules for entries stored in the local table:

- `PublisherDhtEntry` is valid iff its `pub_key` matches the locally configured
  Publisher trust root pubkey (loaded from `GBN_PUBLISHER_PUB_KEY_PATH` in Phase 0's
  `creator-runner`). The entry has no `publisher_sig` field.
- `BridgeDhtEntry` is valid iff `publisher_sig` verifies against that trust root
  pubkey AND `lease_expiry_ms` AND `entry_expiry_ms` are both in the future.
- `CreatorDhtEntry` is valid iff `publisher_sig` verifies against that trust root
  pubkey AND `entry_expiry_ms` is in the future.

A bridge marked `suspect_until_ms` is valid for storage but route selection in Phase 5
must skip it until that timestamp passes.

---

### State Orthogonality Rule (T1.1)

The original plan's single `CreatorOnboardingState` enum forced "host_seeded" and
"onboarded" to be mutually exclusive. They are not. A creator that has already onboarded
itself can later be seeded as a HostCreator for a new creator; both facts are true.

`LocalDiscoveryTable` exposes them as two orthogonal fields:

- `self_onboarding_state`: tracks this node's own NewCreator onboarding progress;
- `host_role_state`: tracks whether this node has been seeded as a HostCreator for
  someone else.

Phase 2 adds the precondition that `SeedHostCreator` requires
`self_onboarding_state = onboarded` before transitioning `host_role_state` to
`host_seeded` (with a test-only `bootstrap_genesis` escape hatch — see Phase 2).

---

## Concurrency Model (T1.4)

The local-DHT state machine is driven by three asynchronous sources:

1. Admin endpoints (`/seed-host-creator`, `/seed-new-creator`, `/send-dummy`,
   `/reset-creator-state`) handled on the admin listener's per-connection threads.
2. Background workers running the bootstrap workflow (Phase 4): seed-bridge punch,
   bridge-set request, fanout punch.
3. Background workers handling reachability-ACK collection (Phase 4) and timeout
   sweeping (Phase 4 §Failure Recovery).

To avoid lock contention and races between SendDummy reads and concurrent fanout
writes, use a single-writer model:

- A dedicated tokio task (or `std::thread`) owns the `LocalDiscoveryTable` mutator.
- All other components send mutation commands over an `mpsc` channel
  (`LocalDhtCommand` enum: `SeedHost`, `SeedNew`, `StartBootstrap`, `ApplyBootstrapResponse`,
  `MarkBridgeActive`, `MarkBridgeSuspect`, `Reset`, etc.).
- Reads use an `Arc<RwLock<LocalDiscoveryTable>>` snapshot the writer maintains; readers
  acquire a read lock that is released before any I/O.
- The writer task persists every applied mutation (debounced) per the Persistence
  subsection.

Document this contract in
`prototype/gbn-bridge-proto/crates/gbn-bridge-creator/src/local_dht.rs` rustdoc.

---

## Persistence (T1.5, Pass 3 D1)

State must survive a single container restart but not a cluster destroy. Match V1's
local-disk pattern from `prototype/gbn-proto/crates/mcn-router-sim/`.

- The writer task snapshots `LocalDiscoveryTable` to
  `${GBN_BRIDGE_STATE_DIR:-/var/lib/gbn-conduit}/local_dht.json` on every applied
  mutation (debounced to at most one fsync per 250 ms).
- Writes are atomic via `write-then-rename` (write to `local_dht.json.tmp`, fsync,
  rename over `local_dht.json`).
- On startup, `creator-runner` reads the file if it exists, validates entries against
  the configured Publisher trust root, drops invalid/expired entries, and resumes from
  the last persisted state.
- If the file is missing or unparseable, start with an empty table at `state=none` and
  log a warning (do not crash — operator may have just provisioned the pod).
- The k8s manifest (Phase 0) mounts a `ReadWriteOnce` PVC at `/var/lib/gbn-conduit`;
  k3d's `local-path-provisioner` keeps the PVC bound to the pod's host directory across
  restarts. CloudFormation tasks mount EFS at the same path.

---

## Admin Surface

Add three endpoints. `/v1/admin/node-metadata` and `/v1/admin/local-dht` are both `GET`;
`/v1/admin/reset-creator-state` is `POST` with empty body.

```http
GET  /v1/admin/node-metadata
GET  /v1/admin/local-dht
POST /v1/admin/reset-creator-state
```

### `GET /v1/admin/node-metadata`

Returns the operator-seed metadata for the selected node. Shape varies by role:

```json
// On creator-host or creator-new
{
  "node_id": "creator-host",
  "role": "creator",
  "ip_addr": "10.x.y.z",
  "admin_addr": "0.0.0.0:9090",
  "creator_udp_punch_port": 443,
  "public_key": "base64...",
  "publisher_public_key": "base64..."
}

// On Publisher (authority surface)
{
  "node_id": "publisher-authority",
  "role": "publisher",
  "publisher_surface": "authority",
  "authority_url": "http://publisher-authority:8080",
  "admin_addr": "127.0.0.1:9090",
  "public_key": "base64..."
}

// On Publisher (receiver surface) — note `role: "publisher"`, single role per §3.5
{
  "node_id": "publisher-receiver",
  "role": "publisher",
  "publisher_surface": "receiver",
  "receiver_url": "http://publisher-receiver:8081",
  "admin_addr": "127.0.0.1:9090",
  "public_key": "base64..."
}

// On exit-bridge-N
{
  "node_id": "exit-bridge-3",
  "role": "exit_bridge",
  "ingress_endpoints": [{ "kind": "direct", "ip_addr": "10.x.y.z", "port": 443 }],
  "admin_addr": "0.0.0.0:9090",
  "udp_punch_port": 443,
  "reachability_class": "direct",
  "capabilities": ["bridge-data-v1"],
  "lease_expiry_ms": 1780000000000,
  "public_key": "base64...",
  "publisher_signature": "base64..."
}
```

### `GET /v1/admin/local-dht` (T1.16 role-aware)

On creator pods, returns the full `LocalDiscoveryTable` (per §3.5, both Publisher
surfaces and bridges have no creator local-DHT state of their own):

```json
{
  "actor_id": "creator-host",
  "role": "creator",
  "self_onboarding_state": "none",
  "host_role_state": "not_host",
  "publisher_entry": null,
  "host_creator_entry": null,
  "creator_entry": null,
  "bridge_entries": [],
  "active_tunnels": [],
  "current_bootstrap_session": null,
  "last_update_ms": 1780000000000,
  "last_error": null
}
```

On Publisher (authority surface), Publisher (receiver surface), and ExitBridge pods,
returns a deliberately-empty role-tagged payload so the operator script can detect
the difference and print a role-appropriate message rather than crashing on a missing
field:

```json
{
  "role": "publisher",
  "publisher_surface": "authority",
  "state": "not_applicable",
  "reason": "publisher does not maintain creator local-DHT state"
}

{
  "role": "exit_bridge",
  "state": "not_applicable",
  "reason": "exit_bridge does not maintain creator local-DHT state"
}
```

### `POST /v1/admin/reset-creator-state` (T2.3)

Clears `LocalDiscoveryTable` back to `state=none, host_role_state=not_host`. Overwrites
the persisted JSON snapshot with the empty table (does not delete the file). Returns
the cleared `chain_id` of the most recent bootstrap session and the prior state for
audit:

Request body (empty `{}`):

```json
{}
```

Response:

```json
{
  "actor_id": "creator-new",
  "chain_id": "reset-creator-state-...",
  "prior_self_onboarding_state": "fanout_partial",
  "prior_host_role_state": "not_host",
  "prior_bootstrap_session_id": "boot-..."
}
```

Endpoint returns `405 Method Not Allowed` on Publisher and ExitBridge pods (the role
field on those pods has nothing to reset).

---

## Implementation Notes

- Add the local-DHT writer task to the `creator-runner` binary's startup sequence
  (Phase 0). The writer task lives for the process lifetime.
- Hold the `Arc<RwLock<LocalDiscoveryTable>>` snapshot inside `AdminState` so admin
  handlers can read without going through the channel.
- `creator-runner` startup: load persisted JSON; validate entries; spawn writer task;
  bind admin listener; ready.
- Publisher and ExitBridge binaries get a thin "not-applicable" handler for
  `/v1/admin/local-dht` rather than the full table — keep the implementation in a
  separate module so creator and non-creator processes share zero state code.
- Do not populate local state from shell-side pod/task discovery. Operator scripts may
  use shell discovery only to choose which node to seed.

---

## Tests

Add focused unit tests in
`prototype/gbn-bridge-proto/crates/gbn-bridge-creator/tests/local_dht.rs`:

- empty local-DHT dump before seeding;
- node-metadata shape by role (creator vs publisher-authority vs publisher-receiver
  vs exit_bridge);
- local-DHT serialization round-trip (write → reload → equal);
- Publisher trust-root validation: rejects entry whose `pub_key` does not match
  configured trust root;
- Bridge entry validation: rejects expired `lease_expiry_ms`, rejects expired
  `entry_expiry_ms`, rejects bad signature, accepts `suspect_until_ms` as marker but
  treats it as filter rather than rejection;
- single-writer model: 1000 concurrent admin reads + 1000 mutations through the
  command channel produce a coherent final state with no panics or deadlocks;
- persistence: write a state file, restart the table, verify reload reproduces the
  state byte-for-byte (modulo `last_update_ms`);
- reset endpoint: clears state and overwrites file; returns prior state.

Also add an integration test in
`prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/tests/admin_local_dht.rs`:

- Publisher authority surface returns `state: "not_applicable"`;
- Publisher receiver surface returns `state: "not_applicable"`;
- ExitBridge returns `state: "not_applicable"`.

Run inside WSL2 Ubuntu (per Master plan §2.8):

```bash
uname -a | grep -i microsoft >/dev/null || { echo "Pass 3 tooling requires WSL2 Ubuntu" >&2; exit 1; }
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check -p gbn-bridge-protocol -p gbn-bridge-publisher -p gbn-bridge-cli -p gbn-bridge-creator
cargo test -p gbn-bridge-protocol --test dht_types
cargo test -p gbn-bridge-creator --test local_dht
cargo test -p gbn-bridge-publisher --test admin_local_dht
```

---

## Completion Evidence

Completed on 2026-05-08.

Implementation landed:

- shared DHT/local discovery protocol types in `gbn-bridge-protocol/src/dht.rs`;
- creator-local single-writer state owner, atomic JSON persistence, reset, and trust-root
  pruning in `gbn-bridge-creator/src/local_dht.rs`;
- `creator-runner` startup loading/persisting local DHT state plus stable creator
  identity material;
- role-aware admin `GET /v1/admin/node-metadata`, `GET /v1/admin/local-dht`, and
  `POST /v1/admin/reset-creator-state`;
- k8s creator pod metadata now includes pod IP for creator transport metadata;
- k8s smoke script now asserts the real `LocalDiscoveryTable` shape, non-creator
  `state=not_applicable` behavior, role-specific node metadata, and creator restart
  persistence when `--check-creator-restart-persistence` is enabled.

Validation completed in WSL2 Ubuntu:

```bash
cargo fmt --all --check
cargo check -p gbn-bridge-protocol -p gbn-bridge-publisher -p gbn-bridge-cli -p gbn-bridge-creator
cargo test -p gbn-bridge-protocol --test dht_types
cargo test -p gbn-bridge-creator --test local_dht
cargo test -p gbn-bridge-publisher --test admin_local_dht
cargo test -p gbn-bridge-cli --bin creator_runner
bash -n infra/scripts/k8s-smoke.sh
kubectl kustomize infra/k8s/conduit/base
infra/scripts/k8s-smoke.sh
infra/scripts/k8s-smoke.sh --check-creator-restart-persistence
```

Additional in-cluster reset probe:

- `creator-host` `POST /v1/admin/reset-creator-state` returned `200`;
- `publisher-authority` `POST /v1/admin/reset-creator-state` returned `405`.

Additional Phase 1 closure validation completed after hardening the local-k8s bring-up:

- `infra/scripts/k8s-up.sh` completed with the rebuilt versioned image set;
- all 14 Conduit workload pods reached `Ready` and stayed stable through post-smoke
  and final settle windows;
- `creator-new` pod deletion and Deployment recreation preserved the persisted
  `LocalDiscoveryTable` from the PVC-backed `local_dht.json`;
- Postgres-backed authority persistence validation passed via
  `infra/scripts/k8s-test-publisher-postgres.sh`.

---

## Acceptance Criteria

- `GET /v1/admin/node-metadata` returns role-appropriate payloads on `creator-host`,
  `creator-new`, both Publisher surfaces, and every `exit-bridge` pod.
- `GET /v1/admin/local-dht` returns a full empty table on creator pods (with both
  `self_onboarding_state` and `host_role_state` present) and a `state=not_applicable`
  payload on non-creator pods.
- `POST /v1/admin/reset-creator-state` succeeds on creator pods and returns 405 on
  non-creator pods.
- Restarting a creator pod (`kubectl delete pod creator-new-...`) preserves the
  persisted state file; after the new pod boots, `GET /v1/admin/local-dht` returns
  the same `LocalDiscoveryTable` minus invalidated entries.
- `BridgeDhtEntry` carries the full `GBN-ARCH-001-V2` §4.1 field set including
  `ingress_endpoints[]`, `reachability_class`, `lease_expiry_ms`, and `capabilities[]`.
- `PublisherDhtEntry` does not carry a `publisher_sig` field; the validation helper
  proves this constructively.
- No route construction or `SendDummy` behavior changes in this phase.
- V1 (`prototype/gbn-proto/**`) is unchanged.
- Parent plan status tracker is updated.
