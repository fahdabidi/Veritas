# GBN-PROTO-013 - Smoke 4 - Mobile Churn / Failover

**Status:** Pending
**Last Updated:** 2026-05-11
**Parent Plan:** [GBN-PROTO-013](GBN-PROTO-013-Conduit-Mobile-Creator-Public-Internet-Validation-Execution-Plan.md)
**Depends On:** Smoke 2 and Smoke 3 complete

## Objective

Prove the mobile creator runtime handles bridge/lane failure and network churn without
falling back to admin shortcuts. This smoke exercises both the local public path and the
hybrid AWS bridge path when available.

---

## Required Scenarios

| Scenario | Environment | Requirement |
|---|---|---|
| Forced SendDummy bridge failure | Local k8s public | Route is reselected from mobile local DHT or explicit degraded state is recorded |
| Forced upload lane failure | Local k8s public | Pending chunks move to active lanes or explicit degraded state is recorded |
| AWS bridge forced failure | Hybrid AWS | Mobile runtime marks AWS bridge suspect and routes to another signed bridge when possible |
| App background/resume during upload | Local or hybrid | Foreground service preserves active ChainID and operation state |
| Network type observation | Local or hybrid | Evidence records cellular/public network context before and after operation |

Actual phone carrier network changes may be operator-dependent. The canonical required
failure input is bridge/lane failure; network toggling is supporting evidence only.

---

## Required Commands

```bash
cd prototype/gbn-bridge-proto
infra/scripts/k8s-pass4-mobile-local-collector.sh \
  --run-id <run_id> \
  --chain-id <failover_chain_id> \
  --evidence-s3-key <s3_key> \
  --require-failover

infra/scripts/k8s-pass4-mobile-hybrid-collector.sh \
  --run-id <run_id> \
  --chain-id <aws_failover_chain_id> \
  --evidence-s3-key <s3_key> \
  --aws-region ca-central-1 \
  --require-cloudwatch-evidence \
  --require-failover
```

---

## Pass Conditions

- Failure injection is visible in app events.
- Mobile local DHT marks failed bridges/lane candidates suspect or inactive with reason.
- Route/lane reselection uses signed local DHT entries.
- No mobile action calls private admin endpoints.
- Operation succeeds or records explicit degraded terminal state.
- Evidence includes timing from failure observation to reroute/degraded result.
- Local k8s and CloudWatch traces correlate by ChainID where applicable.
- S3 evidence bundle is retrieved and hash-verified.

---

## Report Artifacts

Archive under `Test-Reports/`:

- local failover evidence ZIP from S3;
- hybrid failover evidence ZIP from S3 when AWS path is run;
- local k8s trace/log bundle;
- CloudWatch trace/log bundle for AWS scenario;
- DHT before/after snapshots;
- timing summary;
- screenshots or event excerpts showing foreground service continuity;
- V1 preservation output.
