# GBN-PROTO-013 - Smoke 2 - Mobile AWS Public Path

**Status:** Pending
**Last Updated:** 2026-05-12
**Parent Plan:** [GBN-PROTO-013](GBN-PROTO-013-Conduit-Mobile-Creator-Public-Internet-Validation-Execution-Plan.md)
**Depends On:** Phases 1-5 complete

> File-name note: this file keeps its original `Mobile-Local-K8s` path to avoid link
> churn. The canonical Smoke 2 target is now AWS.

## Objective

Prove a physical Android phone on a mobile network can bootstrap and move data against an
AWS-deployed Publisher, HostCreator, and ExitBridge topology over public internet.

This smoke is the canonical mobile-network gap closure gate. Local k8s remains a
regression baseline only; it is not the Smoke 2 sign-off environment.

---

## Required Flow

1. Run Phase 1 strict bootstrap and SendDummy gates against local k8s as regression
   prerequisites.
2. Deploy the Phase 5 AWS public topology.
3. Verify AWS public DNS/TLS/UDP reachability and public admin denial.
4. Generate the AWS HostCreator `BootstrapDHTQRCode`.
5. Disable Wi-Fi on the phone.
6. Select `aws_public`.
7. Import the AWS run profile.
8. Start runtime.
9. Scan AWS HostCreator QR.
10. Import HostCreator seed.
11. Run `BootstrapNewCreator`.
12. Run `SendDummy`.
13. Run `BuildUploadSession`.
14. Run `SendUpload`.
15. Run forced failover or forced lane failure against an AWS ExitBridge.
16. Export evidence and upload to S3.
17. Retrieve evidence from S3 on this workstation.
18. Collect AWS CloudWatch traces/logs by ChainID.
19. Tear down the AWS topology.

---

## Required Commands

Run from WSL2 Ubuntu:

```bash
cd prototype/gbn-bridge-proto

infra/scripts/k8s-smoke-bootstrap-strict-v4.sh --require-observability
infra/scripts/k8s-smoke-senddummy-strict-v4.sh --require-observability

RUN_ID="pass4-smoke2-aws-$(date -u +%Y%m%dT%H%M%SZ)"
AWS_PROFILE_CONFIG="infra/pass4/aws/run-profile.aws-public.live.json"

infra/scripts/aws-pass4-full-topology-plan.sh \
  --config "$AWS_PROFILE_CONFIG" \
  --run-id "$RUN_ID" \
  --publisher-region us-east-1 \
  --bridge-region ca-central-1 \
  --bridge-count 3

infra/scripts/aws-pass4-full-topology-up.sh \
  --config "$AWS_PROFILE_CONFIG" \
  --run-id "$RUN_ID"

infra/scripts/aws-pass4-full-topology-verify.sh \
  --artifact-dir "target/pass4-aws-public/$RUN_ID" \
  --require-no-public-admin \
  --require-hostcreator-qr \
  --require-public-dht-endpoints \
  --require-cloudwatch

infra/scripts/aws-pass4-mobile-collector.sh \
  --run-id "$RUN_ID" \
  --chain-id <bootstrap_chain_id> \
  --chain-id <senddummy_chain_id> \
  --chain-id <upload_chain_id> \
  --chain-id <failover_chain_id> \
  --evidence-s3-key <s3_key> \
  --publisher-region us-east-1 \
  --bridge-region ca-central-1 \
  --require-bootstrap \
  --require-send-dummy \
  --require-upload \
  --require-failover \
  --require-cloudwatch

infra/scripts/aws-pass4-full-topology-down.sh \
  --artifact-dir "target/pass4-aws-public/$RUN_ID"
```

---

## Pass Conditions

- Phone run uses cellular/mobile-network path for canonical evidence.
- AWS endpoint map contains distinct public protocol endpoints for Publisher,
  HostCreator, and selected ExitBridges.
- Public admin exposure check is negative.
- Mobile bootstrap starts with AWS HostCreator seed only.
- Publisher public key/DHT and Seed ExitBridgeB DHT are learned from encrypted Publisher
  bootstrap payload.
- Seed ExitBridgeB returns signed bridge catalog.
- Remaining bridge fanout progress is recorded before active marking.
- SendDummy route source is `local_dht`.
- SendDummy selected route uses an AWS ExitBridge.
- Upload completes with content hash match through AWS bridge lane(s).
- Forced failure reroutes or records explicit degraded terminal state.
- S3 evidence bundle is retrieved and hash-verified on this workstation.
- AWS CloudWatch logs contain matching ChainIDs for Publisher, HostCreator, and selected
  ExitBridges.
- AWS teardown completes or is explicitly deferred with owner, reason, and cleanup date.

---

## Report Artifacts

Archive under `Test-Reports/`:

- mobile evidence ZIP from S3;
- S3 retrieval transcript;
- AWS public endpoint map;
- AWS deployment plan and resource/cost summary;
- HostCreator QR manifest;
- CloudWatch trace/log bundle;
- bootstrap, SendDummy, upload, and failover result summaries;
- device/network context;
- AWS teardown transcript;
- V1 preservation output.
