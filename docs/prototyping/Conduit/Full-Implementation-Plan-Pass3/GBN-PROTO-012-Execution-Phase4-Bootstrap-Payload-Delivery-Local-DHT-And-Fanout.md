# GBN-PROTO-012 - Execution Phase 4 - Bootstrap Payload Delivery, Local DHT, And Fanout

**Status:** Completed
**Last Updated:** 2026-05-08
**Parent Plan:** [GBN-PROTO-012](GBN-PROTO-012-Conduit-Architecture-Correct-Bootstrap-Execution-Plan.md)
**Depends On:** Phase 0–3 complete

## Objective

Complete `GBN-ARCH-001-V2` section 3.3 steps 3 through 12 after the join request reaches
the Publisher (authority surface):

1. Publisher (authority surface) initializes and maintains its Publisher-side signed
   DHT view for all 10 active ExitBridge nodes, then creates the NewCreator entry and
   selects 9 bridge bootstrap entries from that Publisher DHT view.
2. Publisher selects `ExitBridgeB`, distinct from `ExitBridgeA` where possible.
3. Publisher sends the bootstrap payload to ExitBridgeB.
4. ExitBridgeB ACKs and starts punching toward NewCreator.
5. **Publisher returns the bootstrap response back through
   `Publisher → ExitBridgeA → HostCreator → NewCreator`** (per §3.3 step 6) so the
   NewCreator learns ExitBridgeB's identity and starts the receive-side punch.
6. NewCreator and ExitBridgeB exchange UDP probes and ACKs; both sides emit
   `bootstrap_progress` to the Publisher (authority surface) per §3.3 step 7.
7. NewCreator requests the seeded bridge set from ExitBridgeB.
8. ExitBridgeB returns the signed bridge entries.
9. NewCreator stores entries in its local DHT / discovery state.
10. Remaining 8 bridges and NewCreator perform punch / ACK fanout (Publisher signals
    them via `BridgeBatchAssign`).
11. NewCreator marks successful bridge entries `active`.

Per Master plan §3.5, `Publisher` is one role with two surfaces. All Phase 4 references
to "Publisher" refer to the authority surface for orchestration and the receiver
surface for payload sink. Operator scripts treat them as one role.

Before any Phase 4 bootstrap succeeds, the Publisher node must already hold signed DHT
entries for the 10 initialized ExitBridges. The bootstrap flow may refresh that view,
but it must build the NewCreator payload from the Publisher's stored bridge DHT entries,
not from shell-side pod inference or transient response construction.

Update the parent plan status tracker when this phase is complete.

---

## Required Protocol/API Work

Add or complete runtime surfaces. Each documented sub-step maps to a specific message
from `GBN-ARCH-001-V2` §5.2 / §5.3 — no overloading, no `send-dummy` reuse.

### Publisher-Side ExitBridge DHT Initialization

The Publisher has full visibility over active ExitBridges and must maintain its own
signed ExitBridge DHT view before issuing a NewCreator bootstrap payload. This is the
Publisher-side counterpart to the NewCreator's local DHT population.

Required changes:

- Add Publisher authority storage for `publisher_bridge_dht_entries`, keyed by
  `bridge_id`.
- Register, heartbeat, and reclassify operations refresh the stored Publisher DHT
  entry for each active initialized ExitBridge; revoke and expiry paths remove stale
  entries.
- Add `POST /v1/admin/publisher-dht/initialize` to rebuild the Publisher DHT view from
  the current active initialized ExitBridge registry and return the initialized count
  plus bridge ids.
- Add `InitializePublisherDht` to `relay-control-interactive-v2.sh` so the operator can
  explicitly seed the Publisher DHT with all 10 initialized ExitBridges before
  `SeedNewCreator`.
- Bootstrap payload creation reads bridge DHT entries from this Publisher-side DHT
  view. It must not synthesize bridge DHT entries only at response construction time.
- In the Pass 3 topology, `InitializePublisherDht` must materialize 10 signed bridge
  entries. Bootstrap then excludes ExitBridgeA and returns exactly 9 Publisher-signed
  bridge entries to the NewCreator.
- Missing, stale, unsigned, or below-threshold Publisher DHT state fails fast before
  payload issue with an operator-visible error such as
  `PublisherBridgeDhtEntryMissing` or `InsufficientBootstrapBridges`.

### Protocol Message Mapping (T0.7)

| Sub-step | Direction | Message |
|---|---|---|
| Publisher creates bootstrap payload | (internal) | `CreatorBootstrapResponse` body; assembled in Publisher authority service from stored Publisher DHT entries |
| Publisher → ExitBridgeB seed payload delivery | Publisher → Bridge | `BridgePunchStart` carrying `CreatorBootstrapResponse` payload |
| ExitBridgeB seed-payload ACK | Bridge → Publisher | `BootstrapProgress` with `phase=seed_payload_acked` |
| ExitBridgeB → NewCreator punch probes | Bridge → Creator | `BridgePunchProbe` |
| Publisher → ExitBridgeA bootstrap response | Publisher → Bridge | `CreatorBootstrapResponse` (encrypted; bridge does not decrypt) |
| ExitBridgeA → HostCreator bootstrap response | Bridge → Creator | Forwarded `CreatorBootstrapResponse` |
| HostCreator → NewCreator bootstrap response | Creator → Creator | Forwarded `CreatorBootstrapResponse` (over the existing pairing path established in Phase 3) |
| NewCreator → ExitBridgeB punch probes | Creator → Bridge | `BridgePunchProbe` |
| NewCreator and ExitBridgeB tunnel ACKs | Both | `BridgePunchAck` |
| NewCreator and ExitBridgeB progress to Publisher | Both → Publisher | `BootstrapProgress` (one per side, see §Bidirectional Progress below) |
| NewCreator → ExitBridgeB bridge-set request | Creator → Bridge | `BridgeSetRequest` |
| ExitBridgeB → NewCreator bridge-set response | Bridge → Creator | `BridgeSetResponse` |
| NewCreator local DHT update | (internal) | applies `BridgeSetResponse.bridge_entries` to `LocalDiscoveryTable` |
| Publisher → 8 remaining bridges fanout | Publisher → Bridges | `BridgeBatchAssign` |
| Each remaining bridge → NewCreator punch probes | Bridge → Creator | `BridgePunchProbe` |
| Each tunnel ACK | Both | `BridgePunchAck` |
| NewCreator marks each bridge `active` | (internal) | sets `BridgeDhtEntry.active=true` and emits `new_creator_bridge_entry_active` event |

If any of these messages do not exist in `gbn-bridge-protocol` yet, add a new variant
in this phase. Reusing existing variants such as `CatalogRefresh` for bootstrap is
forbidden; downstream receivers must be able to differentiate intent at the message
type level.

---

## Bootstrap Response Return Path (T0.5)

`GBN-ARCH-001-V2` §3.3 step 6 explicitly states:
"The Publisher sends an onion response back through the established path
`Publisher → ExitBridgeA → HostCreator → NewCreator`."

This is how the NewCreator first learns ExitBridgeB's identity. Without it, NewCreator
cannot start the receive-side of the punch; ExitBridgeB would punch into a void.

Implementation:

- The Publisher constructs the `CreatorBootstrapResponse` containing
  `seed_bridge_entry` (ExitBridgeB metadata: ip, pub_key, udp port), Publisher's
  pub_key, and the bootstrap session id.
- The response is encrypted such that ExitBridgeA and HostCreator cannot read its
  contents (only NewCreator can decrypt). For local-prototype purposes, "encrypted"
  may use the existing creator-publisher key envelope; the contract is that ExitBridgeA
  and HostCreator forward opaque bytes.
- ExitBridgeA forwards the opaque response to HostCreator over the same data path
  that carried the join request in Phase 3.
- HostCreator forwards the opaque response to NewCreator over the pairing path
  established in Phase 3.
- NewCreator decrypts and applies the seed bridge entry to its local table; transitions
  `self_onboarding_state` to `seed_bridge_assigned`.
- New tracing events fire: `publisher_response_to_host_via_bridge`,
  `host_response_received_from_bridge`, `host_relayed_response_to_new_creator`,
  `new_creator_bootstrap_response_received` (the 4 events Master plan §2.5 added in
  the return-path block).

---

## Bidirectional Punch Progress (T0.6)

§3.3 step 7: "When each side receives a packet from the other, it ACKs the tunnel
**and both sides notify the Publisher** that progress is being made."

Both directions emit `BootstrapProgress` to the Publisher (authority surface):

- ExitBridgeB → Publisher: `seed_bridge_punch_progress_publisher` event with
  `phase=seed_tunnel_active`.
- NewCreator → Publisher: `new_creator_punch_progress_publisher` event with
  `phase=seed_tunnel_active` (NewCreator reaches the Publisher via the seed tunnel
  through ExitBridgeB itself, at this point).

The Publisher records both progress reports against the bootstrap session id so the
Smoke 2 success-vs-degraded detection (below) can distinguish "only seed bridge
acked" from "all 9 bridges acked".

---

## State Transitions

NewCreator's `self_onboarding_state` transitions during Phase 4:

```text
new_creator_seeded
  -> bootstrapping
  -> seed_bridge_assigned        (after CreatorBootstrapResponse decoded)
  -> seed_tunnel_active          (after first BridgePunchAck with ExitBridgeB)
  -> bridge_set_received         (after BridgeSetResponse applied to local table)
  -> fanout_in_progress          (after Publisher dispatches BridgeBatchAssign)
  -> onboarded                   (≥ 1 bridge active AND seed tunnel active)
       OR
  -> fanout_partial              (timeout with at least one bridge active)
       OR
  -> fanout_failed               (timeout with zero bridges active)
       OR
  -> seed_tunnel_failed          (timeout before seed tunnel ACK)
```

`onboarded` requires N ≥ 1 active bridge entries (typically all 9 over time, but the
state is reachable as soon as the first non-seed bridge ACKs, so SendDummy can begin).
`fanout_partial` is a successful-but-degraded terminal state — the operator may
choose to leave the creator in this state or `reset-creator-state` and retry.

`GET /v1/admin/local-dht` exposes after Phase 4:

- `self_onboarding_state` (one of the values above);
- `current_bootstrap_session.session_id`;
- `creator_entry` (Publisher-signed entry for this NewCreator);
- `host_creator_entry` (the entry seeded in Phase 3, retained for traceability);
- `bridge_entries` (all signed bridge entries received from ExitBridgeB);
- per-entry `active` flag and `suspect_until_ms`;
- `last_error` if state is `*_failed`.

---

## Distinct Bridge Rule

When at least 2 active bridges exist (which is always true in the Pass 3 topology of
10 bridges), Publisher must choose:

- `ExitBridgeA` as the relay bridge from HostCreator to Publisher.
- `ExitBridgeB` as the seed bridge for NewCreator.
- `ExitBridgeB != ExitBridgeA`.

The original spec's `seed_bridge_reused=true` flag is removed from the response — with
10 bridges deployed (Phase 0 / Pass 3 D3), the constrained-topology fallback is no
longer needed. If the test environment somehow drops below 2 bridges (which Smoke 2
fail-fast detects upfront), Publisher returns
`bootstrap_payload_insufficient_bridges` and the workflow transitions to
`fanout_failed`.

---

## Failure Recovery (T1.8)

§7 of `GBN-ARCH-001-V2` mandates bridge-failure recovery semantics. Phase 4 implements
the foundation; SendDummy in Phase 5 consumes it.

### Timeouts

- `seed_tunnel_timeout_ms`: default 30 000 (30 s). Time from `seed_bridge_assigned`
  to first `BridgePunchAck` from ExitBridgeB. Configurable via env
  `GBN_BRIDGE_SEED_TUNNEL_TIMEOUT_MS`.
- `fanout_timeout_ms`: default 60 000 (60 s). Time from `fanout_in_progress` to all 9
  bridges responding. Configurable via env `GBN_BRIDGE_FANOUT_TIMEOUT_MS`.
- `suspect_ttl_ms`: default 300 000 (5 min). Time a bridge stays marked
  `suspect_until_ms` after a punch timeout, ACK miss, or BridgeData send failure.

### Transitions on Timeout

- `seed_bridge_assigned/seed_tunnel_active → seed_tunnel_failed` if seed tunnel
  timeout fires before first ExitBridgeB ACK. Publisher receives
  `bootstrap_progress` with `phase=seed_tunnel_failed`.
- `fanout_in_progress → fanout_partial` if fanout timeout fires with at least one
  non-seed bridge ACKed.
- `fanout_in_progress → fanout_failed` if fanout timeout fires with zero non-seed
  bridges ACKed (only the seed tunnel survived).

### Mark Bridge Suspect Action

Triggered on:

- punch timeout (no `BridgePunchAck` within `seed_tunnel_timeout_ms` for that bridge);
- ACK miss after a successful punch (subsequent `BridgeData` send fails);
- explicit `force_bridge_failure` from SendDummy (Phase 5 §Failover Test).

Sets `BridgeDhtEntry.suspect_until_ms = now_ms + suspect_ttl_ms`. Route selection in
Phase 5 skips suspect bridges. After `suspect_until_ms` passes, the bridge becomes
eligible for selection again (no automatic catalog refresh — that is a Pass-4
hardening item).

### Recovery Path

After any `*_failed` state:

- `last_error` is populated with a human-readable summary and the failed
  `bootstrap_session_id`.
- Operator runs `POST /v1/admin/reset-creator-state` (Phase 1) to clear, then
  re-runs `SeedNewCreator` (Phase 3) with a fresh chain.
- Phase 5 SendDummy refuses to operate against any non-`onboarded` and non-`fanout_partial`
  state.

---

## Observability

11 second-half events of the 16-event Master plan §2.5 list:

- `publisher_bootstrap_payload_created`
- `publisher_seed_bridge_selected`
- `seed_bridge_payload_received` (= ExitBridgeB receives `BridgePunchStart`)
- `seed_bridge_punch_started`
- `publisher_response_to_host_via_bridge` (T0.5)
- `host_response_received_from_bridge` (T0.5)
- `host_relayed_response_to_new_creator` (T0.5)
- `new_creator_bootstrap_response_received` (T0.5)
- `seed_bridge_punch_progress_publisher` (T0.6)
- `new_creator_seed_tunnel_ack`
- `new_creator_punch_progress_publisher` (T0.6)
- `new_creator_bridge_set_requested`
- `seed_bridge_bridge_set_returned`
- `new_creator_local_dht_updated`
- `publisher_remaining_bridges_triggered` (= `BridgeBatchAssign` dispatched)
- `new_creator_bridge_entry_active` (one per bridge)

Together with Phase 3's first 5 events, this completes the full 16-event traceability
list. Each event must include `chain_id`, `bootstrap_session_id`, `new_creator_id`,
`host_creator_id`, `relay_bridge_id`, `seed_bridge_id`, and bridge counts where
applicable.

---

## Tests

Add tests in
`prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/tests/admin_bootstrap_flow.rs`:

- Publisher selects a seed bridge distinct from relay bridge when at least 2 bridges
  exist;
- `InitializePublisherDht` materializes exactly 10 registered active ExitBridges into
  the Publisher's signed bridge DHT view before bootstrap begins;
- bootstrap payload includes the NewCreator entry and exactly 9 bridge entries when
  10 bridges are registered (1 ExitBridgeA excluded);
- ExitBridgeB records and ACKs the bootstrap payload via `BootstrapProgress`;
- Publisher → ExitBridgeA → HostCreator → NewCreator return path delivers the
  encrypted `CreatorBootstrapResponse` end-to-end;
- bidirectional punch progress reaches Publisher from both ExitBridgeB and NewCreator;
- NewCreator stores signed entries after `BridgeSetResponse`;
- expired or invalid bridge entries are not stored;
- active flags change only after reachability ACKs;
- `GET /v1/admin/local-dht` returns `self_onboarding_state=onboarded` after the full
  workflow against a 10-bridge cluster;
- seed tunnel timeout transitions to `seed_tunnel_failed` and populates `last_error`;
- fanout timeout with at least one bridge active transitions to `fanout_partial`;
- fanout timeout with zero bridges active transitions to `fanout_failed`;
- `mark_bridge_suspect` sets TTL and route selection in Phase 5 honors it.

Run inside WSL2 Ubuntu:

```bash
uname -a | grep -i microsoft >/dev/null || { echo "Pass 3 tooling requires WSL2 Ubuntu" >&2; exit 1; }
cd prototype/gbn-bridge-proto
cargo test -p gbn-bridge-publisher --test admin_bootstrap_flow
cargo test -p gbn-bridge-creator --test bootstrap_workflow
```

---

## Implementation Notes

Completed 2026-05-08.

- `BootstrapJoinReply` now includes optional signed V2 `creator_dht_entry` and
  `bridge_set` payloads while preserving the legacy `BootstrapDhtEntry` fields used by
  existing runtime callers.
- `BridgeSetResponse` now carries `bridge_dht_entries` in addition to legacy bootstrap
  hints so NewCreator local DHT state can verify Publisher signatures after onboarding.
- Publisher maintains a signed bridge DHT view (`publisher_bridge_dht_entries`) for
  active ExitBridges. Bridge registration, heartbeat renewal, and reclassification
  refresh it; revoke/expiry remove stale entries.
- `relay-control-interactive-v2.sh` now exposes `InitializePublisherDht`, which calls
  `POST /v1/admin/publisher-dht/initialize` to rebuild Publisher DHT entries from all
  active initialized ExitBridges before running bootstrap smoke tests.
- Publisher bootstrap selection consumes stored Publisher DHT entries and rejects a
  relay-only candidate set with
  `InsufficientBootstrapBridges`; with enough direct bridges it excludes ExitBridgeA
  from the seeded bridge set and selects ExitBridgeB separately.
- The local admin bootstrap path consumes the returned payload into
  `LocalDiscoveryTable`, stores the NewCreator's own signed entry, marks received
  bridge entries active after the simulated ACK path, and records active tunnels.
- Publisher authority sessions record seed-payload, seed-tunnel, NewCreator progress,
  and completion events for local smoke validation.

---

## Acceptance Criteria

- A NewCreator (`creator-new`) reaches `self_onboarding_state=onboarded` after
  `SeedNewCreator` against a 10-bridge cluster.
- Publisher DHT contains signed entries for all 10 active ExitBridges before the
  bootstrap payload is built.
- Local DHT includes the NewCreator's own Publisher-signed creator entry plus 9 bridge
  entries with valid signatures and unexpired lease/entry windows.
- The seed bridge entry is active after seed tunnel ACK.
- All 8 remaining reachable bridge entries become active after fanout completion.
- The Publisher response uses the §3.3 step 6 return path
  (`Publisher → ExitBridgeA → HostCreator → NewCreator`); both event traces and
  `bootstrap_session.last_state` reflect this.
- Both sides emit `BootstrapProgress` for the seed tunnel; Publisher records both.
- Forced timeouts produce the documented degraded states without crashing.
- Smoke 2 (Phase 6) can assert local DHT population from the actor's own table
  rather than from authority registry queries or pod-name inference.
- V1 (`prototype/gbn-proto/**`) is unchanged.
- Parent plan status tracker is updated.
