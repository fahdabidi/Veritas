# GBN-PROTO-013 - Smoke 4 - Mobile Churn / Failover

**Status:** Pending
**Last Updated:** 2026-05-12
**Parent Plan:** [GBN-PROTO-013](GBN-PROTO-013-Conduit-Mobile-Creator-Public-Internet-Validation-Execution-Plan.md)
**Depends On:** Smoke 2 and Smoke 3 complete

## Objective

Prove the mobile creator runtime handles bridge/lane failure and network churn without
falling back to admin shortcuts. The canonical churn/failover path now uses the AWS
public topology from Phase 5 and the non-U.S. AWS bridge fleet when available.

---

## Required Scenarios

| Scenario | Environment | Requirement |
|---|---|---|
| Forced SendDummy bridge failure | AWS public | Route is reselected from mobile local DHT or explicit degraded state is recorded |
| Forced upload lane failure | AWS public | Pending chunks move to active lanes or explicit degraded state is recorded |
| AWS bridge forced failure | AWS geo | Mobile runtime marks AWS bridge suspect and routes to another signed bridge when possible |
| App background/resume during upload | AWS public or AWS geo | Foreground service preserves active ChainID and operation state |
| Network type observation | AWS public or AWS geo | Evidence records cellular/public network context before and after operation |

Actual phone carrier network changes may be operator-dependent. The canonical required
failure input is bridge/lane failure; network toggling is supporting evidence only.

---

## Required Commands

```bash
cd prototype/gbn-bridge-proto
infra/scripts/aws-pass4-mobile-collector.sh \
  --run-id <run_id> \
  --chain-id <failover_chain_id> \
  --evidence-s3-key <s3_key> \
  --publisher-region us-east-1 \
  --bridge-region ca-central-1 \
  --require-cloudwatch \
  --require-failover

infra/scripts/aws-pass4-mobile-collector.sh \
  --run-id <run_id> \
  --chain-id <aws_geo_failover_chain_id> \
  --evidence-s3-key <s3_key> \
  --publisher-region us-east-1 \
  --bridge-region ca-central-1 \
  --require-cloudwatch \
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
- AWS CloudWatch traces correlate by ChainID.
- S3 evidence bundle is retrieved and hash-verified.

---

## Report Artifacts

Archive under `Test-Reports/`:

- AWS public failover evidence ZIP from S3;
- AWS geo failover evidence ZIP from S3 when the cross-region path is run;
- CloudWatch trace/log bundle for AWS Publisher, HostCreator, and ExitBridges;
- DHT before/after snapshots;
- timing summary;
- screenshots or event excerpts showing foreground service continuity;
- V1 preservation output.
