# GBN-PROTO-013 - Smoke 2 - Mobile Local-k8s Public Path

**Status:** Pending
**Last Updated:** 2026-05-11
**Parent Plan:** [GBN-PROTO-013](GBN-PROTO-013-Conduit-Mobile-Creator-Public-Internet-Validation-Execution-Plan.md)
**Depends On:** Phases 1-5 complete

## Objective

Prove a physical Android phone on a mobile network can bootstrap and move data against the
local k8s Publisher, HostCreator, and ExitBridges over public internet.

This smoke is the canonical local mobile-network gap closure gate.

---

## Required Flow

1. Run Phase 1 strict bootstrap and SendDummy gates.
2. Prepare Phase 4 public ingress.
3. Generate HostCreator `BootstrapDHTQRCode`.
4. Disable Wi-Fi on the phone.
5. Select `local_k8s_public`.
6. Start runtime.
7. Scan HostCreator QR.
8. Import HostCreator seed.
9. Run `BootstrapNewCreator`.
10. Run `SendDummy`.
11. Run `BuildUploadSession`.
12. Run `SendUpload`.
13. Run forced failover or forced lane failure.
14. Export evidence and upload to S3.
15. Retrieve evidence from S3 on this workstation.
16. Collect local k8s traces/logs by ChainID.
17. Tear down public ingress.

---

## Required Commands

Run from WSL2 Ubuntu:

```bash
cd prototype/gbn-bridge-proto
infra/scripts/k8s-smoke-bootstrap-strict-v4.sh --require-observability
infra/scripts/k8s-smoke-senddummy-strict-v4.sh --require-observability
infra/scripts/k8s-pass4-public-ingress-prepare.sh --profile local_k8s_public --run-id <run_id>
infra/scripts/k8s-pass4-public-ingress-verify.sh --require-no-public-admin --require-hostcreator-qr
infra/scripts/k8s-pass4-mobile-local-collector.sh \
  --run-id <run_id> \
  --chain-id <mobile_chain_id> \
  --evidence-s3-key <s3_key> \
  --require-bootstrap \
  --require-send-dummy \
  --require-upload \
  --require-failover
infra/scripts/k8s-pass4-public-ingress-down.sh --run-id <run_id>
```

---

## Pass Conditions

- Phone run uses cellular/mobile-network path for canonical evidence.
- Mobile bootstrap starts with HostCreator seed only.
- Publisher public key/DHT and Seed ExitBridgeB DHT are learned from encrypted Publisher
  bootstrap payload.
- Seed ExitBridgeB returns signed bridge catalog.
- Remaining bridge fanout progress is recorded before active marking.
- SendDummy route source is `local_dht`.
- Upload completes with content hash match.
- Forced failure reroutes or records explicit degraded terminal state.
- S3 evidence bundle is retrieved and hash-verified on this workstation.
- Local k8s traces/logs contain matching ChainIDs.
- Public admin exposure check is negative.
- Public ingress teardown completes.

---

## Report Artifacts

Archive under `Test-Reports/`:

- mobile evidence ZIP from S3;
- S3 retrieval transcript;
- public endpoint map;
- HostCreator QR manifest;
- local k8s trace/log bundle;
- bootstrap, SendDummy, upload, and failover result summaries;
- device/network context;
- teardown transcript;
- V1 preservation output.
