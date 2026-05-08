# GBN-PROTO-012 - Execution Phase 3 - SeedNewCreator API And First-Contact Join

**Status:** Completed
**Last Updated:** 2026-05-08
**Parent Plan:** [GBN-PROTO-012](GBN-PROTO-012-Conduit-Architecture-Correct-Bootstrap-Execution-Plan.md)
**Depends On:** Phase 0 (creator pods deployed), Phase 1 (state model), Phase 2
(HostCreator seeded)

## Objective

Implement the second external input and the first half of the documented first-time
creator bootup flow:

1. `NewCreator` receives HostCreator metadata from the operator.
2. `NewCreator` sends a join request to `HostCreator`.
3. `HostCreator`, using seeded `ExitBridgeA`, relays the encrypted join request to the
   Publisher.

This phase must remove the current synthetic shortcut where a selected actor calls the
Publisher directly and sets `host_creator_id` and `relay_bridge_id` to itself.

Per Master plan §3.5, "Publisher" is a single role with two deployment surfaces. Phase 3
references "Publisher (authority surface)" wherever the join request must reach the
authority half (lease validation, bootstrap orchestration, signing).

Update the parent plan status tracker when this phase is complete.

Implementation note: the final Publisher authority request is signed by the HostCreator,
because the existing authority authentication contract requires
`actor_id == host_creator_id` for `/v1/bootstrap/join`. The NewCreator still signs the
private HostCreator relay envelope; HostCreator validates it, fills in its seeded
ExitBridgeA id, signs the Publisher join envelope as HostCreator, and relays it to the
Publisher authority surface. This preserves the observable actor chain and removes the
Pass 2 `new_creator_id == host_creator_id == relay_bridge_id` shortcut.

---

## Admin API

Add:

```http
POST /v1/admin/seed-new-creator
```

Request:

```json
{
  "new_creator_id": "creator-new",
  "host_creator_entry": {
    "node_id": "creator-host",
    "ip_addr": "10.x.y.z",
    "pub_key": "base64...",
    "udp_punch_port": 443,
    "entry_expiry_ms": 1780000000000,
    "publisher_sig": "base64-or-host-seed-signature"
  },
  "start_bootstrap": true,
  "force": false
}
```

Response:

```json
{
  "new_creator_id": "creator-new",
  "host_creator_id": "creator-host",
  "self_onboarding_state": "bootstrapping",
  "chain_id": "seed-new-creator-..."
}
```

If `start_bootstrap=false`, store the seed in the local table and transition
`self_onboarding_state` to `new_creator_seeded`, but do not begin the join workflow.
The relay-control script default is `true`. The `false` mode is for diagnosing
failures — operator can seed, inspect `GET /v1/admin/local-dht`, and trigger bootstrap
later via `POST /v1/admin/start-bootstrap` (admin-only, no operator-script binding by
default; documented for diagnostic use).

### Validation Rules

- `new_creator_id` must match the selected node's `actor_id`. Reject with
  `new_creator_id_mismatch` otherwise.
- `host_creator_entry.publisher_sig` must verify against the Publisher trust root or
  the HostCreator's seed signature. Reject with `host_creator_signature_invalid`
  otherwise.
- `host_creator_entry.entry_expiry_ms` must be in the future. Reject with
  `host_creator_expired` otherwise.
- The HostCreator referenced by `host_creator_entry` must report
  `host_role_state=host_seeded` when probed by the operator script before submission;
  the script enforces this precondition (the API itself cannot enforce it without a
  network call back to HostCreator and is intentionally permissive — it accepts the
  payload as-is and lets the workflow surface "host not seeded" errors at runtime).
- The target's prior `self_onboarding_state` must be `none` or `failed`. Other states
  imply an in-flight or completed onboarding; reject with
  `new_creator_already_seeded` unless `force=true`.

### Idempotency Rule (T1.7)

Same shape as Phase 2:

- Byte-identical payload against a target whose state is `new_creator_seeded` or
  `bootstrapping` returns `200 OK` with the original `chain_id`.
- Differing payload returns `409 Conflict` with `seed_already_present` unless
  `force=true`. With `force=true`, the prior state is reset (same path as
  `/v1/admin/reset-creator-state`) and the new seed is applied with a fresh
  `chain_id`.

---

## Required Runtime Flow

Implement these internal calls. The observable data path must show the correct actor
chain — even if for local-prototype convenience the implementation uses
localhost-admin-triggered private endpoints internally, every span/log must reflect
the documented `NewCreator → HostCreator → ExitBridgeA → Publisher (authority surface)`
path.

```text
NewCreator
  -> receives HostCreator metadata through /v1/admin/seed-new-creator
  -> creates CreatorJoinRequest for itself, signed with NewCreator's identity key
  -> sends request to HostCreator (over the HostCreator's admin or pairing endpoint)

HostCreator
  -> validates host_role_state=host_seeded
  -> validates NewCreator request signature
  -> wraps the join request in a HostJoinRelay envelope and forwards to seeded
     ExitBridgeA via the bridge's existing data path

ExitBridgeA
  -> forwards opaque join request to Publisher (authority surface)

Publisher (authority surface)
  -> handles CreatorJoinRequest with host_creator_id and relay_bridge_id set correctly
  -> records the bootstrap session for Phase 4 to complete
```

If the HostCreator is offline or returns an error, the NewCreator's
`self_onboarding_state` transitions to `failed` with `last_error` populated. Operator
runs `/v1/admin/reset-creator-state` to retry.

The join request payload must include:

- NewCreator id (`new_creator_id`)
- NewCreator public key
- NewCreator observed endpoint metadata (IP and UDP punch port)
- HostCreator id (`host_creator_id`)
- ExitBridgeA id (`relay_bridge_id`)
- `chain_id` (carried from `seed-new-creator` response)
- Request signature from NewCreator

Use the existing `CreatorJoinRequest` message from
`gbn-bridge-protocol` (per `GBN-ARCH-001-V2` §5.2). Add fields if any of the above are
missing. Do not overload existing message variants.

---

## Relay-Control Script Flow

Add `SeedNewCreator` to:

- `prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh`
- `prototype/gbn-bridge-proto/infra/scripts/k8s-control-interactive.sh`
- shared `_seed_actions.sh`

Operator flow:

1. Discover live nodes filtering by `role=creator`. Present `creator-host` and
   `creator-new`. Refuse if both are reporting `state=onboarded` (no work to do).
2. Prompt: select NewCreator node (typically `creator-new`).
3. Prompt: select existing HostCreator node (typically `creator-host`).
4. Verify the HostCreator: query `GET /v1/admin/local-dht` on it; refuse to proceed
   unless `host_role_state=host_seeded`. Print the relevant Publisher and ExitBridgeA
   metadata for operator confirmation.
5. Build the `host_creator_entry` payload from HostCreator's `node-metadata` plus its
   stored `host_seed_signature` field.
6. POST `/v1/admin/seed-new-creator` to NewCreator with `start_bootstrap=true`.
7. Poll `GET /v1/admin/local-dht` on NewCreator. Print intermediate transitions:
   `bootstrapping → seed_bridge_assigned → seed_tunnel_active → bridge_set_received
   → fanout_in_progress → onboarded` (or `fanout_partial`, `fanout_failed`,
   `seed_tunnel_failed`).
8. Stop polling when state reaches a terminal state (`onboarded`, `fanout_partial`,
   `fanout_failed`, `seed_tunnel_failed`) or after the operator-configurable
   timeout (default 120 seconds).
9. Print local DHT summary: bridge entries count, active count, terminal state,
   `chain_id`.
10. Offer trace collection by `chain_id`.

Per Master plan §2.8, all script invocations require WSL2 Ubuntu.

---

## Observability

Emit logs/spans:

- `new_creator_seed_requested` (admin endpoint accepted)
- `new_creator_seed_stored` (state persisted to local table)
- `new_creator_seed_idempotent_replay`
- `new_creator_seed_force_replaced`
- `new_creator_join_started` (bootstrap workflow started)
- `host_creator_join_received` (HostCreator received the request)
- `host_creator_join_relayed_via_bridge` (HostCreator dispatched via ExitBridgeA)
- `publisher_join_received` (Publisher authority surface received the request)

Each event must include `chain_id`, `new_creator_id`, `host_creator_id`,
`relay_bridge_id`, and `bootstrap_session_id` (where applicable, per Phase 1 model).

These 5 first-half events are the first half of the 16 events Master plan §2.5
requires; Phase 4 emits the remaining 11.

---

## Tests

Add tests in
`prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/tests/admin_seed_new.rs`:

- valid seed against an unseeded NewCreator stores `self_onboarding_state=new_creator_seeded`
  when `start_bootstrap=false`;
- valid seed with `start_bootstrap=true` transitions to `bootstrapping` and emits
  `new_creator_join_started`;
- mismatched `new_creator_id` is rejected with `new_creator_id_mismatch`;
- expired `host_creator_entry` is rejected with `host_creator_expired`;
- bad host signature is rejected with `host_creator_signature_invalid`;
- target whose state is `bootstrapping` rejects new payload without `force=true`;
- `force=true` clears prior state via the reset path and re-seeds;
- HostCreator that is not seeded rejects join requests with
  `host_creator_not_seeded`;
- NewCreator join request payload contains distinct `new_creator_id`,
  `host_creator_id`, and `relay_bridge_id` (the synthetic shortcut from Pass 2 must
  be gone — assert with three different values);
- Publisher (authority surface) records the bootstrap session with the same three
  ids set correctly and a populated `chain_id`.

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
cargo test -p gbn-bridge-publisher --test admin_seed_new
cargo test -p gbn-bridge-creator --test join_path
```

---

## Acceptance Criteria

- `SeedNewCreator` can trigger a join request from `creator-new` through `creator-host`
  to the Publisher (authority surface), with the payload reaching the Publisher.
- Logs/traces prove HostCreator (`creator-host`) and ExitBridgeA (one of the 10
  `exit-bridge` pods) are distinct actors in the path; the synthetic
  `host_creator_id == relay_bridge_id == new_creator_id` shortcut is removed.
- After Phase 3, `GET /v1/admin/local-dht` on `creator-new` reports
  `self_onboarding_state=bootstrapping` (the workflow is in flight; Phase 4 takes it
  to `onboarded`).
- Direct Publisher bootstrap from the legacy `discovery-probe` admin endpoint is no
  longer treated as Smoke 2 success (this is enforced in the new Pass 3 Smoke 2
  successor doc, not by Phase 3 itself).
- Idempotent replay returns the same `chain_id`; conflicting payload requires
  `force=true`.
- V1 (`prototype/gbn-proto/**`) is unchanged.
- Parent plan status tracker is updated.

---

## Completion Evidence

Implemented:

- `POST /v1/admin/seed-new-creator` on creator admin listeners.
- Diagnostic `POST /v1/admin/start-bootstrap` on creator admin listeners.
- Private `POST /v1/admin/host/join` on HostCreator admin listeners.
- `POST /v1/admin/creator-dht-entry` on the Publisher authority admin listener to
  return a Publisher-signed HostCreator DHT entry for operator seeding.
- `NewCreatorSeedState.chain_id` and `start_bootstrap` persisted in local DHT state.
- `creator-runner` now installs creator identity config into its admin state so it can
  sign NewCreator relay envelopes and HostCreator authority envelopes.
- `SeedNewCreator` shared operator action in
  `prototype/gbn-bridge-proto/infra/scripts/_seed_actions.sh`, surfaced in both local
  k8s and AWS operator menus.
- Focused test suites:
  `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/tests/admin_seed_new.rs`
  and `prototype/gbn-bridge-proto/crates/gbn-bridge-creator/tests/join_path.rs`.

Validated:

```bash
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check -p gbn-bridge-protocol -p gbn-bridge-publisher -p gbn-bridge-creator -p gbn-bridge-cli
cargo test -p gbn-bridge-publisher --test admin_seed_new
cargo test -p gbn-bridge-creator --test join_path
cargo test -p gbn-bridge-publisher --test admin_seed_host
cargo test -p gbn-bridge-publisher --test admin_local_dht
cargo test -p gbn-bridge-publisher --test admin_routes
cargo test -p gbn-bridge-publisher --test admin_send_dummy
cargo test -p gbn-bridge-protocol --test dht_types
bash -lc 'bash -n infra/scripts/_seed_actions.sh && bash -n infra/scripts/k8s-control-interactive.sh && bash -n infra/scripts/relay-control-interactive-v2.sh'
```

Deferred to Phase 4:

- Bootstrap payload return through ExitBridgeB, bridge set delivery, local DHT bridge
  population, and reachability ACK activation.
