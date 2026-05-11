# GBN-PROTO-013 - Smoke 3 - Mobile AWS Geo Path

**Status:** Pending
**Last Updated:** 2026-05-11
**Parent Plan:** [GBN-PROTO-013](GBN-PROTO-013-Conduit-Mobile-Creator-Public-Internet-Validation-Execution-Plan.md)
**Depends On:** Phases 1-8 complete

## Objective

Prove the same Android app can use the local k8s Publisher plus non-U.S. AWS ExitBridges
over public internet, with evidence from mobile logs, local k8s logs, and AWS CloudWatch.

This smoke closes the AWS geolocation validation path while keeping Publisher local.

---

## Required Flow

1. Verify local k8s public Publisher/HostCreator ingress.
2. Deploy or verify AWS ExitBridges in `ca-central-1`.
3. Confirm AWS bridge entries are signed into the local Publisher catalog.
4. Disable Wi-Fi on phone for canonical run.
5. Select `hybrid_local_publisher_aws_bridges`.
6. Scan HostCreator QR.
7. Run mobile bootstrap.
8. Confirm mobile DHT contains AWS bridge entries with `region=ca-central-1`.
9. Run `SendDummy` requiring an AWS bridge route.
10. Run `SendUpload` requiring AWS bridge lane use.
11. Export and upload evidence to S3.
12. Retrieve mobile evidence from S3.
13. Collect local k8s logs and CloudWatch logs by ChainID.

---

## Required Commands

```bash
cd prototype/gbn-bridge-proto
infra/scripts/aws-pass4-bridge-only-verify.sh \
  --region ca-central-1 \
  --expect-bridge-count 3 \
  --require-local-publisher-signed-catalog

infra/scripts/k8s-pass4-mobile-hybrid-collector.sh \
  --run-id <run_id> \
  --chain-id <mobile_chain_id> \
  --evidence-s3-key <s3_key> \
  --aws-region ca-central-1 \
  --require-aws-bridge-route \
  --require-cloudwatch-evidence \
  --require-local-publisher-evidence
```

---

## Pass Conditions

- Local k8s Publisher remains the only Publisher authority/receiver.
- AWS ExitBridges run in `ca-central-1`.
- Mobile bootstrap uses HostCreator QR and encrypted Publisher bootstrap payload.
- Mobile DHT includes Publisher-signed AWS bridge entries.
- SendDummy uses at least one AWS bridge route.
- Upload uses at least one AWS bridge lane and completes with content hash match.
- CloudWatch logs contain selected bridge ids and matching ChainIDs.
- Local k8s Publisher/Receiver logs contain matching ChainIDs.
- S3 mobile evidence bundle is retrieved and hash-verified.
- AWS resources are torn down or documented as intentionally retained.

---

## Report Artifacts

Archive under `Test-Reports/`:

- mobile hybrid evidence ZIP from S3;
- S3 retrieval transcript;
- AWS bridge endpoint map;
- local Publisher signed catalog snapshot;
- CloudWatch log bundle;
- local k8s trace/log bundle;
- SendDummy/upload summaries;
- device/network context;
- AWS teardown transcript or retention note;
- V1 preservation output.
