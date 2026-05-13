# GBN-PROTO-013 - Execution Phase 8 - Mobile To AWS Geo Validation

**Status:** Pending Rewrite
**Last Updated:** 2026-05-12
**Parent Plan:** [GBN-PROTO-013](GBN-PROTO-013-Conduit-Mobile-Creator-Public-Internet-Validation-Execution-Plan.md)
**Depends On:** Phases 1-7 complete

> Superseded note: the `2026-05-12` Phase 5 topology decision moved Publisher and
> HostCreator into AWS for canonical physical mobile validation. This phase must be
> rewritten to validate AWS Publisher/HostCreator plus non-U.S. AWS ExitBridges, not a
> local-k8s Publisher hybrid.

## Objective

Run the same Android creator app used in Phase 5 against the hybrid topology:

- local k8s Publisher and HostCreator remain the authority/bootstrap path;
- AWS ExitBridges run in `ca-central-1`;
- mobile creator uses Publisher-signed AWS bridge entries learned through bootstrap and
  catalog handoff;
- evidence is collected from Android, local k8s, S3, and CloudWatch.

This phase closes the AWS geolocation portion of the remaining mobile-network validation
gap without moving Publisher out of local k8s.

Update the parent plan status tracker when this phase is complete.

---

## Preconditions

- Phase 5 mobile local-k8s public validation is green.
- Phase 7 AWS ExitBridge deployment is healthy in `ca-central-1`.
- Local k8s Publisher public endpoint is active.
- HostCreator public bootstrap QR is regenerated for the hybrid run id.
- Publisher bridge catalog contains expected AWS bridge entries.
- S3 evidence upload grant is prepared.
- Phone uses the same installed app build as Phase 5 unless the report records a new build
  id and reruns the Phase 3 device smoke.

---

## Mobile Run Flow

1. Select `hybrid_local_publisher_aws_bridges`.
2. Start runtime or reset to a clean creator state.
3. Scan the HostCreator QR generated from the local k8s HostCreator.
4. Import HostCreator seed.
5. Tap `BootstrapNewCreator`.
6. Verify the resulting bridge catalog includes `region=ca-central-1` AWS bridge entries.
7. Tap `SendDummy`.
8. Build and send a full upload.
9. Run forced bridge/lane failure against one AWS bridge.
10. Export evidence and upload to S3.

Expected:

- Publisher public key/DHT and Seed ExitBridgeB DHT are learned through encrypted
  bootstrap payload, not run-profile preload.
- Bridge catalog includes AWS bridge ids signed by the local k8s Publisher.
- Selected routes/lanes use AWS `ca-central-1` bridge entries for at least one SendDummy
  and one upload run.
- Local k8s Publisher/Receiver logs and AWS CloudWatch logs share ChainIDs.
- Failover either reroutes to another AWS bridge or records an explicit degraded state
  with suspect bridge evidence.

---

## Evidence Correlation

The collector must merge:

| Source | Required Files |
|---|---|
| S3 mobile bundle | `evidence.json`, `local_dht.json`, `events.jsonl`, `trace_events.jsonl`, `network_context.json`, `remote_trace_queries.json` |
| Local k8s | Publisher authority logs, receiver logs, HostCreator logs, observability traces |
| AWS | CloudWatch logs for selected bridge ids, ECS task metadata, endpoint descriptors |
| Public endpoint setup | hybrid endpoint map, HostCreator QR manifest, AWS bridge catalog snapshot |

Every operation in the final report must be traceable by ChainID across at least the app,
local k8s Publisher/Receiver, and one AWS ExitBridge CloudWatch stream.

---

## Validation

Run from WSL2 Ubuntu:

```bash
uname -a | grep -i microsoft >/dev/null || { echo "Pass 4 tooling requires WSL2 Ubuntu" >&2; exit 1; }

cd prototype/gbn-bridge-proto
infra/scripts/aws-pass4-bridge-only-verify.sh \
  --region ca-central-1 \
  --expect-bridge-count 3 \
  --require-local-publisher-signed-catalog

infra/scripts/k8s-pass4-mobile-hybrid-collector.sh \
  --run-id <run_id> \
  --chain-id <mobile_chain_id> \
  --evidence-s3-key mobile-evidence/<run_id>/<chain_id>/<bundle_id>.zip \
  --aws-region ca-central-1 \
  --require-aws-bridge-route \
  --require-cloudwatch-evidence \
  --require-local-publisher-evidence
```

The collector fails if:

- the mobile bundle lacks AWS bridge ids;
- selected AWS bridge ids are absent from CloudWatch logs;
- local Publisher catalog entries are unsigned or signed by the wrong trust root;
- Publisher is an AWS Publisher instead of local k8s Publisher;
- mobile evidence was produced by a different app build than the Phase 5 accepted build
  without a documented rerun.

---

## Tests

Add focused tests for:

- app hybrid profile rejects preloaded Publisher/bridge bootstrap state;
- mobile local DHT marks AWS bridge entries with region/source metadata;
- SendDummy can require an AWS bridge route for validation mode;
- upload can require at least one AWS bridge lane for validation mode;
- collector correlates ChainID across S3 bundle, local k8s logs, and CloudWatch logs;
- collector fails when CloudWatch evidence is missing for selected AWS bridge ids;
- teardown after hybrid run does not remove local k8s artifacts before reports are
  archived.

Run:

```bash
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo test --workspace

cd mobile/android
./gradlew test
./gradlew connectedDebugAndroidTest
```

---

## Acceptance Criteria

- Same Android app build from Phase 5 is used, or a new app build is documented and
  re-smoked.
- Local k8s Publisher remains the Publisher authority/receiver.
- AWS bridges run in `ca-central-1`.
- Mobile bootstrap learns AWS bridge entries through Publisher-signed catalog flow.
- `SendDummy` uses at least one AWS bridge route and succeeds.
- Full upload uses AWS bridge lane(s) and succeeds with content hash match.
- Forced AWS bridge failure reroutes or records explicit degraded terminal state.
- Evidence bundle is uploaded to S3 and retrieved on this workstation.
- CloudWatch logs correlate selected AWS bridge ids with mobile ChainIDs.
- Local k8s Publisher/Receiver logs correlate with the same ChainIDs.
- AWS teardown is run or explicitly deferred with reason and owner.
- V1 preservation checks return no files.
- Parent plan status tracker is updated.

---

## Completion Evidence

When this phase is implemented, archive:

- mobile hybrid evidence ZIP from S3;
- S3 retrieval transcript and hash verification;
- local k8s Publisher/Receiver/HostCreator trace bundle;
- AWS CloudWatch trace bundle;
- AWS bridge catalog snapshot;
- device/network context;
- SendDummy report;
- upload report;
- forced failure report;
- AWS teardown transcript or deferred-teardown note;
- V1 preservation command output.
