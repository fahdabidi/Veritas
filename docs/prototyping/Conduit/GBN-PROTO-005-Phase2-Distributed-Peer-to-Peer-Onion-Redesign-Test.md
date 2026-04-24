# GBN-PROTO-005 - Conduit Phase 11 Test Results

**Status:** Tooling implemented and baseline evidence recorded; live AWS/mobile validation is still pending  
**Related Matrix:** [prototype/gbn-bridge-proto/docs/mobile-test-matrix.md](../../prototype/gbn-bridge-proto/docs/mobile-test-matrix.md)  
**Baseline Release:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)

---

## 1. Summary

Phase 11 adds V2-only mobile-validation tooling and records the current state of
evidence. The committed Conduit harness already proves several mobile-like
recovery behaviors locally, but true carrier/mobile results still require a
live AWS deployment and real network churn.

## 2. Current Evidence

| Scenario | Current Evidence | Status |
|---|---|---|
| App restart with cached catalog | `creator_bootstrap::returning_creator_refresh_uses_cached_catalog_and_retries_next_bridge` | local harness pass |
| Stale bridge recovery | `reachability::signed_downgrade_clears_transport_state_and_blocks_weak_repromotion` plus `integration::test_catalog_refresh` | local harness pass |
| First-time bootstrap | `creator_bootstrap::host_creator_bootstrap_establishes_seed_tunnel_and_updates_local_dht` plus `integration::test_first_creator_bootstrap` | local harness pass |
| UDP punch ACK | `bridge_runtime::seed_bridge_establishes_acks_and_returns_bootstrap_payload` plus `integration::test_udp_punch_ack` | local harness pass |
| Bridge failover | `data_path::mid_session_failover_reassigns_pending_frames_to_another_bridge` plus `integration::test_creator_failover` | local harness pass |
| Reuse after insufficient fanout | `integration::test_bridge_reuse_timeout` | local harness pass |
| Batched onboarding rollover | `authority_flow::eleventh_join_request_rolls_into_the_next_batch` plus `integration::test_batch_bootstrap` | local harness pass |
| Network switch / IP churn on live carrier path | no live measurement yet | pending |
| Live AWS/mobile bootstrap latency | no live measurement yet | pending |

## 3. Current Conclusion

The Conduit prototype already demonstrates the expected control-flow behavior in
the local harness:

- cached-catalog restart recovery works
- signed stale-entry recovery works
- first-contact bootstrap through HostCreator works
- seed and follow-on punch ACK correlation works
- failover and bridge reuse work
- batch rollover behavior works

What is still missing is measured real-world evidence for:

- mobile IP churn
- network switching between carrier / Wi-Fi paths
- live AWS bootstrap latency
- coordinated UDP punch success through real NAT behavior

## 4. Tooling Added In Phase 11

- local proxy runner: `prototype/gbn-bridge-proto/infra/scripts/mobile-validation.sh --mode local`
- AWS/mobile runner: `prototype/gbn-bridge-proto/infra/scripts/mobile-validation.sh --mode aws`
- metrics collector: `prototype/gbn-bridge-proto/infra/scripts/collect-bridge-metrics.sh`
- scenario matrix: `prototype/gbn-bridge-proto/docs/mobile-test-matrix.md`

## 5. Remaining Work Before Phase 11 Sign-Off

1. Deploy the committed Phase 10 stack into AWS.
2. Run `mobile-validation.sh --mode aws` against the deployed stack.
3. Capture real log and ECS metrics with `collect-bridge-metrics.sh`.
4. Record bootstrap latency, failover latency, and batch rollover timing from a live run.
5. Execute at least one real network-change or IP-churn scenario from a mobile-like client path.

## 6. V1 Preservation

No V1 deployment or validation files were changed as part of this phase.
