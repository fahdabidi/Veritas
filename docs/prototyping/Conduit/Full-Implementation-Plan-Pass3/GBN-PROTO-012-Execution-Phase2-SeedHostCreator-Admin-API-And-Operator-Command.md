# GBN-PROTO-012 - Execution Phase 2 - SeedHostCreator Admin API And Operator Command

**Status:** Completed
**Last Updated:** 2026-05-08
**Parent Plan:** [GBN-PROTO-012](GBN-PROTO-012-Conduit-Architecture-Correct-Bootstrap-Execution-Plan.md)
**Depends On:** Phase 0 (creator pods deployed), Phase 1 (state model and admin surface)

## Objective

Implement the first external input in the documented section 3.3 flow. The operator must
be able to select a creator pod as `HostCreator` and seed it with:

- Publisher DHT metadata;
- a single already-working `ExitBridgeA` DHT metadata entry.

After this phase, the selected node is ready to relay a NewCreator join request through
ExitBridgeA to the Publisher, but NewCreator bootup itself is implemented in Phase 3.

Per Master plan §3.5, "Publisher" is one role with two deployment surfaces (authority,
receiver). The operator selects the Publisher once; the script reads metadata from both
surfaces.

Update the parent plan status tracker when this phase is complete.

---

## Admin API

Add:

```http
POST /v1/admin/seed-host-creator
```

Request:

```json
{
  "host_creator_id": "creator-host",
  "publisher_entry": {
    "node_id": "publisher",
    "authority_url": "http://publisher-authority:8080",
    "receiver_url": "http://publisher-receiver:8081",
    "pub_key": "base64..."
  },
  "exit_bridge_a_entry": {
    "bridge_id": "exit-bridge-a",
    "identity_pub": "base64...",
    "ingress_endpoints": [
      { "kind": "direct", "ip_addr": "10.x.y.z", "port": 443 }
    ],
    "udp_punch_port": 443,
    "reachability_class": "direct",
    "lease_expiry_ms": 1780000000000,
    "entry_expiry_ms": 1780000000000,
    "capabilities": ["bridge-data-v1"],
    "publisher_sig": "base64..."
  },
  "bootstrap_genesis": false,
  "force": false
}
```

Note: `publisher_entry` carries no `publisher_sig` (per Phase 1 Trust Root Rule —
Publisher does not sign its own entry; the receiving creator validates the entry's
`pub_key` against its locally configured Publisher trust root).

Response:

```json
{
  "host_creator_id": "creator-host",
  "self_onboarding_state": "onboarded",
  "host_role_state": "host_seeded",
  "seeded_bridge_id": "exit-bridge-a",
  "publisher_node_id": "publisher",
  "chain_id": "seed-host-creator-...",
  "genesis": false,
  "forced": false,
  "idempotent": false
}
```

### Validation Rules

- `host_creator_id` must match the selected node's `actor_id`. Reject with
  `host_creator_id_mismatch` otherwise.
- `publisher_entry.pub_key` must match the locally configured Publisher trust root.
  Reject with `publisher_trust_mismatch` otherwise.
- `exit_bridge_a_entry.publisher_sig` must verify against the Publisher trust root.
  Reject with `bridge_signature_invalid` otherwise.
- `exit_bridge_a_entry.lease_expiry_ms` and `exit_bridge_a_entry.entry_expiry_ms` must both
  be in the future. Reject with `bridge_expired` otherwise.
- `exit_bridge_a_entry.reachability_class` must be `direct` or `brokered`. A
  `relay_only` entry is rejected with `bridge_relay_only_ineligible` (only `direct`
  bridges can act as ExitBridgeA per `GBN-ARCH-001-V2` §4.2).

### Onboarding Precondition (T1.6)

`GBN-ARCH-001-V2` §2.1: a HostCreator is "an ordinary creator that already has a working
path to the Publisher." Default behavior:

- If the target's `self_onboarding_state != onboarded`, return
  `host_creator_not_onboarded` with HTTP 409. The operator must onboard the node as a
  NewCreator first (Phases 3+4), then run `SeedHostCreator`.

#### `bootstrap_genesis: true` escape hatch

In a fresh cluster, no creator is yet onboarded — there is a chicken-and-egg problem.
The escape hatch handles the very first HostCreator only:

- When `bootstrap_genesis=true`, the precondition is bypassed. The seed installs both
  `host_role_state=host_seeded` AND `self_onboarding_state=onboarded` directly,
  populating the local table with the same Publisher and ExitBridgeA entries it would
  have learned through a real onboarding.
- The endpoint logs `host_creator_genesis_seed_used` at WARN level and includes
  `genesis: true` in the response, so this is auditable.
- Smoke 2 (Phase 6) uses `bootstrap_genesis=true` for the first HostCreator and
  chains real onboarding for every NewCreator after that.
- Documentation in this file is the authoritative explanation; production deployments
  must not call with `bootstrap_genesis=true` outside the genesis bring-up.

### Idempotency Rule (T1.7)

Repeated calls to `seed-host-creator` are common (operator restarts a script, or runs
both AWS and k8s smoke runs against the same target). The endpoint enforces:

- If the target already has `host_role_state=host_seeded` AND the new payload is
  byte-identical to the one stored in the prior `HostCreatorSeedState` (publisher
  entry, exit-bridge entry, `bootstrap_genesis`), return `200 OK` with the same
  `chain_id` recorded for that prior seed and unchanged state.
- If the payload differs, return `409 Conflict` with code `seed_already_present`
  unless the request includes `force: true`. With `force=true`, the prior state is
  cleared via the same path as `reset-creator-state`, and the new seed is applied
  with a fresh `chain_id`.

---

## Relay-Control Script Flow

Add `SeedHostCreator` to:

- `prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh`
- `prototype/gbn-bridge-proto/infra/scripts/k8s-control-interactive.sh`
- shared `_seed_actions.sh` library (per Phase 6 §2.7 in T1.15 — the menu logic is
  written once in `_seed_actions.sh` and sourced from both top-level scripts).

Operator flow (identical across AWS and local k8s; only the transport differs):

1. Discover live nodes filtering by `role=creator`. Present `creator-host` and
   `creator-new` only.
2. Prompt: select HostCreator node.
3. Discover Publisher (single role per §3.5). The script fetches metadata from both
   surfaces (`publisher-authority` and `publisher-receiver`) and constructs one
   `publisher_entry` payload combining `authority_url` + `receiver_url`.
4. Discover live nodes filtering by `role=exit_bridge` and
   `reachability_class=direct`. Prompt: select ExitBridgeA node.
5. Resolve the selected ExitBridgeA's `bridge_id` from `GET /v1/admin/node-metadata`,
   then fetch `GET /v1/admin/bridges/{bridge_id}/dht-entry` from the Publisher
   authority surface. The authority signs the DHT entry from its active bridge registry
   with the Publisher signing key, so the script never fabricates `publisher_sig`.
   The script copies that returned object into `exit_bridge_a_entry`.
6. Resolve genesis flag: if `GET /v1/admin/local-dht` on the HostCreator target reports
   `self_onboarding_state=none`, ask the operator to confirm they want
   `bootstrap_genesis=true` (default no — they should onboard the host first). If
   `self_onboarding_state=onboarded`, proceed without genesis.
7. POST `/v1/admin/seed-host-creator` to the HostCreator target.
8. Print resulting `chain_id` and seed state.
9. Offer to collect traces by `chain_id`.

Per Master plan §2.8, all script invocations must run inside WSL2 Ubuntu; the script's
opening WSL guard from §2.8 fails fast if invoked from PowerShell.

Implementation note: Phase 2 added the authority-side helper route above because the
strict HostCreator endpoint verifies Publisher signatures and shell tooling cannot
validly derive them from bridge metadata alone.

---

## Observability

Emit logs/spans:

- `host_creator_seed_requested`
- `host_creator_seed_validated`
- `host_creator_seed_stored`
- `host_creator_genesis_seed_used` (WARN level, only when `bootstrap_genesis=true`)
- `host_creator_seed_idempotent_replay` (when the same payload is replayed)
- `host_creator_seed_force_replaced` (when `force=true` overwrites prior state)

Each must include:

- `chain_id`
- `host_creator_id`
- `seed_bridge_id`
- `publisher_node_id`
- `genesis: bool`
- `forced: bool`

---

## Tests

Add tests in
`prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/tests/admin_seed_host.rs`:

- valid seed against an onboarded host stores `host_role_state=host_seeded`;
- mismatched `host_creator_id` is rejected with `host_creator_id_mismatch`;
- `publisher_entry.pub_key` not matching trust root is rejected with
  `publisher_trust_mismatch`;
- expired bridge entry (either `lease_expiry_ms` or `entry_expiry_ms` in the past) is
  rejected with `bridge_expired`;
- bridge signature mismatch is rejected with `bridge_signature_invalid`;
- `relay_only` bridge is rejected with `bridge_relay_only_ineligible`;
- precondition: `self_onboarding_state=none` blocks the seed by default, with code
  `host_creator_not_onboarded`;
- escape hatch: `bootstrap_genesis=true` against a `state=none` node succeeds and
  installs both states;
- idempotent replay: identical payload returns same `chain_id`;
- conflicting payload without `force` returns `409 seed_already_present`;
- conflicting payload with `force=true` clears prior state and re-seeds;
- local-DHT dump reflects `host_role_state=host_seeded` after seeding.

Shell syntax checks:

```bash
bash -n prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh
bash -n prototype/gbn-bridge-proto/infra/scripts/k8s-control-interactive.sh
bash -n prototype/gbn-bridge-proto/infra/scripts/_seed_actions.sh
```

Run inside WSL2 Ubuntu:

```bash
uname -a | grep -i microsoft >/dev/null || { echo "Pass 3 tooling requires WSL2 Ubuntu" >&2; exit 1; }
cd prototype/gbn-bridge-proto
cargo test -p gbn-bridge-publisher --test admin_seed_host
```

---

## Acceptance Criteria

- Operator can seed any onboarded creator pod as HostCreator without `bootstrap_genesis`.
- Operator can seed a fresh `creator-host` with `bootstrap_genesis=true` exactly once
  (the script's confirmation prompt prevents accidental misuse).
- `GET /v1/admin/local-dht` on a seeded HostCreator shows
  `host_role_state=host_seeded` AND a non-null `publisher_entry` AND a non-null
  ExitBridgeA entry in `bridge_entries`.
- HostCreator local state contains exactly the selected Publisher and ExitBridgeA
  entries — no extras leak in.
- Repeated invocation with identical payload is a no-op and returns the same
  `chain_id`. Repeated invocation with different payload requires `force=true`.
- No NewCreator bootup is triggered in this phase.
- V1 (`prototype/gbn-proto/**`) is unchanged.
- Parent plan status tracker is updated.

---

## Completion Evidence

Implemented:

- `POST /v1/admin/seed-host-creator` on creator admin listeners.
- `GET /v1/admin/bridges/{bridge_id}/dht-entry` on the Publisher authority admin
  listener to return a Publisher-signed ExitBridge DHT entry.
- `HostCreatorSeedState.chain_id` and `bootstrap_genesis` persisted in local DHT state.
- Shared operator action library `prototype/gbn-bridge-proto/infra/scripts/_seed_actions.sh`.
- `SeedHostCreator` menu action in both local k8s and AWS operator scripts.
- Focused test suite
  `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/tests/admin_seed_host.rs`.

Validated:

```bash
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check -p gbn-bridge-protocol -p gbn-bridge-publisher -p gbn-bridge-creator -p gbn-bridge-cli
cargo test -p gbn-bridge-publisher --test admin_seed_host
cargo test -p gbn-bridge-publisher --test admin_local_dht
cargo test -p gbn-bridge-publisher --test admin_routes
cargo test -p gbn-bridge-publisher --test admin_send_dummy
cargo test -p gbn-bridge-protocol --test dht_types
bash -lc 'bash -n infra/scripts/_seed_actions.sh && bash -n infra/scripts/k8s-control-interactive.sh && bash -n infra/scripts/relay-control-interactive-v2.sh'
```

Deferred to later phases:

- Live operator walkthrough against the Phase 0 cluster after Phase 3 introduces
  `SeedNewCreator`; Phase 2 does not trigger NewCreator bootup.
