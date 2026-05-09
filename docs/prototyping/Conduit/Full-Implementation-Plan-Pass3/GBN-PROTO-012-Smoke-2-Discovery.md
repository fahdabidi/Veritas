# GBN-PROTO-012 - Execution Phase 8 - Smoke 2 Discovery / Bootup Suite Implementation (Pass 3 Successor To GBN-PROTO-009)

**Document ID:** GBN-PROTO-012-Smoke-2
**Status:** Completed
**Last Updated:** 2026-05-09
**Phase:** 8 (Smoke 2 — Discovery / Bootup Suite Implementation)
**Supersedes (in scope only):** [GBN-PROTO-009 Local Kubernetes Discovery Smoke Test Plan](../Full-Implementation-Plan-Pass2/GBN-PROTO-009-Local-Kubernetes-Discovery-Smoke-Test-Plan.md)
**Parent Plan:** [GBN-PROTO-012](GBN-PROTO-012-Conduit-Architecture-Correct-Bootstrap-Execution-Plan.md)
**Depends On:** Phases 0–6 complete; Phase 7 (Smoke 1) green

This is **Smoke 2** in the Pass 3 local Kubernetes Conduit suite. It validates that
the architecture-correct first-time creator bootup flow from `GBN-ARCH-001-V2` §3.3
runs end-to-end and that `creator-new` ends up with a populated, signed, and verified
local DHT / discovery table.

The Pass 2 file `GBN-PROTO-009-Local-Kubernetes-Discovery-Smoke-Test-Plan.md` is left
unchanged. The Pass 2 baseline `discovery-probe` shortcut is **not** treated as Smoke
2 success in Pass 3.

Completed 2026-05-09. Phase 8 implements `k8s-smoke-discovery-v3.sh`, adds the
Publisher authority `GET /v1/admin/bootstrap-session` inspection endpoint, and hardens
local Tempo validation with stable memory/ballast values plus a higher TraceQL search
limit. Live validation passed in k3d with `creator-new` onboarded, 10 active local DHT
bridge entries, a distinct NewCreator/HostCreator/ExitBridgeA/ExitBridgeB chain, and
all 16 bootstrap events present in Tempo.

---

## 1. Goal

Prove that:

1. The Phase 0 cluster (10 ExitBridges, 2 creators, both Publisher surfaces, Postgres,
   observability) is up.
2. Operator can run `SeedHostCreator` (with `bootstrap_genesis=true` for the very
   first run) and reach `host_role_state=host_seeded` on `creator-host`.
3. Operator can run `SeedNewCreator` and `creator-new` reaches a terminal state
   (`onboarded` or `fanout_partial`) within the timeout.
4. `creator-new`'s local DHT contains:
   - its own Publisher-signed creator entry (validated against the Publisher trust
     root);
   - 10 bridge entries (one per Publisher-seeded ExitBridge DHT entry), each
     Publisher-signed, with valid `lease_expiry_ms` and `entry_expiry_ms`, and
     correct `reachability_class`;
   - the seed bridge entry marked `active=true`;
   - all remaining fanout bridges marked `active=true` (or at least 5 if degraded
     `fanout_partial` is acceptable).
5. The actor chain in distributed traces matches §3.3:
   `creator-new → creator-host → ExitBridgeA → publisher-authority → ExitBridgeB →
   creator-new` (forward + return + seed handoff).
6. All 16 bootup events from Master plan §2.5 are present in Tempo for the Smoke 2
   bootstrap session, indexed by `chain_id` and `bootstrap_session_id`.

---

## 2. Pre-Conditions

- WSL2 Ubuntu host (Master plan §2.8 guard at top of script).
- Smoke 1 has just passed (instrumentation alive).
- `creator-host` and `creator-new` are Ready.
- 10 ExitBridge pods are Ready and registered in the Publisher authority registry.
- `InitializePublisherDht` succeeds and reports 10 initialized Publisher-side
  ExitBridge DHT entries before `SeedNewCreator` is allowed to start.

---

## 3. Implemented Command

```bash
bash prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-discovery-v3.sh \
  --require-observability \
  --bootstrap-timeout 120 \
  --trace-timeout 300 \
  --min-active-bridges 5 \
  --allow-fanout-partial
```

Flags:

- `--require-observability`: as Smoke 1.
- `--bootstrap-timeout N`: max seconds to poll `local-dht` (default 120).
- `--trace-timeout N`: max seconds to wait for Tempo event evidence (default 300).
- `--min-active-bridges N`: minimum active bridge count for a `fanout_partial` to be
  accepted (default 5). `onboarded` always passes.
- `--allow-fanout-partial`: if set, `fanout_partial ≥ min-active-bridges` is treated
  as success. Default off (strict).

---

## 4. Probe Sequence

1. Generate `chain_id = smoke-2-<UUID>`.
2. Verify `creator-host` is in `state=none`. If not, run
   `POST /v1/admin/reset-creator-state` on it.
3. Verify `creator-new` is in `state=none`. If not, reset.
4. **SeedHostCreator**: discover Publisher and one direct ExitBridge (call this
   ExitBridgeA). Build the seed payload. POST
   `/v1/admin/seed-host-creator?chain_id=<chain_id>` to `creator-host` with
   `bootstrap_genesis=true`. Assert response has `host_role_state=host_seeded`
   and echoes the same `chain_id`.
5. **InitializePublisherDht**: POST
   `/v1/admin/publisher-dht/initialize?chain_id=<chain_id>` to the Publisher
   authority surface. Assert `initialized_bridge_count == 10`,
   `publisher_dht_entry_count == 10`, and response `chain_id == <chain_id>`.
6. **SeedNewCreator**: build the `host_creator_entry` payload from `creator-host`'s
   metadata + seed signature. POST
   `/v1/admin/seed-new-creator?chain_id=<chain_id>` to `creator-new` with
   `start_bootstrap=true`. Assert response has
   `self_onboarding_state=bootstrapping` and echoes the same `chain_id`.
7. Poll `GET /v1/admin/local-dht` on `creator-new` every 1 s for up to
   `--bootstrap-timeout` s. Track every state transition.
8. Stop when `self_onboarding_state` reaches a terminal state.
9. Run assertions.

---

## 5. Assertions

### 5.1 Terminal State

- `self_onboarding_state ∈ { onboarded, fanout_partial (if --allow-fanout-partial) }`.
- For `fanout_partial`, count of `bridge_entries[*].active==true` ≥
  `--min-active-bridges`.

### 5.2 Local DHT Content

`GET /v1/admin/local-dht` on `creator-new` returns:

- `creator_entry.publisher_sig` verifies against the Publisher trust root pubkey
  (read once from the Publisher's `node-metadata` and pinned for the run).
- `creator_entry.entry_expiry_ms > now_ms`.
- `bridge_entries.length == 10`.
- For every `bridge_entries[i]`:
  - `publisher_sig` verifies;
  - `lease_expiry_ms > now_ms` AND `entry_expiry_ms > now_ms`;
  - `reachability_class ∈ { direct, brokered }` (no `relay_only` in bootstrap set);
  - `ingress_endpoints.length ≥ 1`;
  - `capabilities` is non-empty;
  - `bridge_id` matches one of the 10 deployed `exit-bridge-*` pods.
- The seed bridge `bridge_id` (from the response of `seed-new-creator` or from the
  `chain_id`-correlated trace) is marked `active=true`.
- `current_bootstrap_session.last_state` matches the terminal `self_onboarding_state`.

### 5.3 Distinct Actor Chain

The synthetic Pass-2 shortcut where `host_creator_id == relay_bridge_id ==
new_creator_id` must be gone. From the bootstrap session record on the Publisher
(authority surface) and from the Tempo trace:

- `new_creator_id == "creator-new"`;
- `host_creator_id == "creator-host"` (distinct);
- `relay_bridge_id == ExitBridgeA's bridge_id` (distinct from both above);
- `seed_bridge_id != relay_bridge_id` (distinct ExitBridgeB);
- All four ids are different.

### 5.4 Trace Coverage (16 Events)

For the Smoke 2 `chain_id`, Tempo returns spans covering all 16 events from Master
plan §2.5:

Forward:

1. `host_creator_seed_stored` (from Phase 2)
2. `new_creator_seed_stored` (from Phase 3)
3. `new_creator_join_started`
4. `host_creator_join_relayed_via_bridge`
5. `publisher_join_received`

Return path:

6. `publisher_response_to_host_via_bridge`
7. `host_response_received_from_bridge`
8. `host_relayed_response_to_new_creator`
9. `new_creator_bootstrap_response_received`

Seed punch and progress:

10. `seed_bridge_payload_received`
11. `seed_bridge_punch_progress_publisher`
12. `new_creator_seed_tunnel_ack`
13. `new_creator_punch_progress_publisher`
14. `seed_bridge_bridge_set_returned`
15. `new_creator_local_dht_updated`
16. `new_creator_bridge_entry_active` (≥ 1 occurrence; one per active bridge)

Every matched span must carry the same `chain_id` echoed by SeedHostCreator,
InitializePublisherDht, and SeedNewCreator. A span with the expected event name
but a different chain id is a failure, not supporting evidence.

### 5.5 No Direct-Authority Shortcut

Search Tempo for `traceql: { chain_id="<chain_id>" && span.name="discovery_probe" }`
within the test window. Assert ≥ 0 hits and **no spans whose `actor_id` is a
non-creator pod and whose `event` is `creator_bootstrap_response_received`**. The
legacy `discovery-probe` shortcut on Publisher/bridge pods would produce such spans;
their absence proves the architecture-correct flow ran.

Implementation note: the discovery-probe TraceQL query is a strict zero-hit
assertion. Any `discovery_probe` span under the Smoke 2 `chain_id` fails the run.

---

## 6. Artifacts

Written to
`prototype/gbn-bridge-proto/target/k8s-smoke-artifacts/smoke-2-discovery/<run-id>/`:

- `pods.json`
- `chain-id.txt`
- `seed-host-creator-result.json`
- `seed-new-creator-result.json`
- `local-dht-progression.jsonl` (one row per poll iteration)
- `local-dht-final.json`
- `bootstrap-session.json` (from Publisher authority)
- `traces-by-event.json` (16 events, span counts per event)
- `trace-evidence.tempo-traces.json` (full trace dump)
- `failure-evidence.json` (only if test fails: which assertion, which value)
- `summary.md`

---

## 7. Failure Modes And Triage

| Failure | Likely Cause |
|---|---|
| `creator-new` stuck in `bootstrapping` | HostCreator → Publisher path broken; check ExitBridgeA reachability |
| `seed_tunnel_failed` | UDP punch port not exposed in k8s; check exit-bridge service ports |
| `fanout_failed` | Publisher BridgeBatchAssign not reaching remaining bridges; check authority control session |
| Local DHT bridge count != 10 | Publisher created bootstrap with wrong count; check `bridge_count_target` in publisher logic |
| 16-event trace missing return-path events | T0.5 not implemented; Phase 4 return-path block missing |
| Tempo search returns only a subset of events | TraceQL result limit too low; `VERITAS_TEMPO_SEARCH_LIMIT` defaults to 200 for Smoke 2 |
| Tempo port-forward fails or pod restarts | Observability backend degraded; verify Tempo is not OOM-killed and that `tempo.memBallastSizeMbs` is below its memory limit |
| All four actor ids equal | Pre-Pass-3 shortcut still in place; Phase 3 was not actually completed |

---

## 8. Out Of Scope

- Route construction and SendDummy (Smoke 3).
- AWS validation (handled in Phase 6 AWS acceptance).
- Bridge fanout latency / scale measurements.
- Sanitization / chunking pipeline (Master plan §6).

---

## 9. Implementation Tasks

1. Add `infra/scripts/k8s-smoke-discovery-v3.sh` (new file).
2. Extend `k8s-smoke-common.sh` with `bootstrap_session_query(chain_id)`.
3. WSL2 guard at top of script.
4. Print artifact directory at end (success or failure).

---

## 10. Acceptance

- `bash -n prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-discovery-v3.sh` passes.
- Against a fresh `k8s-up.sh` cluster, the script exits 0.
- All 16 events present in Tempo.
- Distinct actor chain proven.
- Live local validation passed on 2026-05-09 with artifacts under
  `prototype/gbn-bridge-proto/target/k8s-smoke-artifacts/smoke-2-discovery/20260508-184504-1982332/`.
- `git diff --stat -- docs/prototyping/Conduit/Full-Implementation-Plan-Pass2/` is empty.
- V1 (`prototype/gbn-proto/**`) is unchanged.
