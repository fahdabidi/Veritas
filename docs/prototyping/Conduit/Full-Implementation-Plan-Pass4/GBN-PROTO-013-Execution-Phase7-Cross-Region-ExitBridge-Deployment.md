# GBN-PROTO-013 - Execution Phase 7 - Cross-Region ExitBridge Deployment

**Status:** Pending
**Last Updated:** 2026-05-11
**Parent Plan:** [GBN-PROTO-013](GBN-PROTO-013-Conduit-Mobile-Creator-Public-Internet-Validation-Execution-Plan.md)
**Depends On:** Phase 6 complete

## Objective

Deploy and verify public ExitBridge services in a non-U.S. AWS region for Pass 4 mobile
geolocation validation. The default region is `ca-central-1`.

This phase proves the AWS bridge fleet is reachable, registered with the local k8s
Publisher, represented in Publisher-signed DHT/catalog entries, and observable through
CloudWatch before the Android app uses it in Phase 8.

Update the parent plan status tracker when this phase is complete.

---

## Deployment Shape

Default deployment:

- region: `ca-central-1`;
- bridge count: 3;
- runtime: existing ExitBridge container image compatible with Pass 3/4 protocol;
- public protocol ports only;
- admin private through ECS Exec;
- CloudWatch logs enabled;
- local k8s Publisher public endpoint configured as authority target.

Optional parity deployment:

- bridge count: 10;
- short run only;
- used if cost envelope allows and local Publisher catalog scaling needs parity proof.

The first run uses 3 bridges to demonstrate non-U.S. geolocation with lower expected cost
than a 10-bridge always-on fleet or Australia-region deployment.

---

## Deployment Steps

1. Confirm local k8s public Publisher endpoint is active.
2. Confirm Phase 6 bridge-only stack plan is current.
3. Deploy AWS bridge-only stack in `ca-central-1`.
4. Wait for all ExitBridge tasks/services to report healthy.
5. Verify public protocol reachability for each bridge.
6. Verify admin ports are not public.
7. Verify each bridge registers with local k8s Publisher.
8. Refresh Publisher bridge catalog.
9. Confirm Publisher-signed DHT entries contain AWS region and public endpoint metadata.
10. Write `aws_exitbridge_catalog_snapshot.json`.
11. Capture CloudWatch log stream ids for every bridge.

---

## Required Scripts

```text
prototype/gbn-bridge-proto/infra/scripts/aws-pass4-bridge-only-up.sh
prototype/gbn-bridge-proto/infra/scripts/aws-pass4-bridge-only-verify.sh
prototype/gbn-bridge-proto/infra/scripts/aws-pass4-bridge-only-down.sh
```

`aws-pass4-bridge-only-up.sh` must require:

- `--region ca-central-1`;
- `--bridge-count 3` or `--bridge-count 10`;
- `--publisher-public-endpoint`;
- explicit confirmation for any bridge count above 3.

`aws-pass4-bridge-only-verify.sh` must check:

- ECS service desired/running count;
- task health;
- CloudWatch log stream availability;
- protocol reachability from public internet;
- admin denial from public internet;
- local Publisher catalog contains expected AWS bridge count;
- every AWS bridge entry is signed by the local Publisher trust root.

---

## Geolocation Evidence

Minimum evidence:

- AWS region from deployment configuration and task metadata: `ca-central-1`;
- bridge public endpoints and DNS/IPs;
- Publisher-signed DHT entries containing `region=ca-central-1`;
- CloudWatch log group/stream names;
- optional public IP geolocation lookup as supporting evidence, not as the source of truth.

The report should state that the geolocation guarantee is AWS-region placement, not a
consumer geolocation database claim.

---

## Cost Guardrails

- Default bridge count is 3.
- Stack is torn down after Phase 8 validation unless a follow-up run is scheduled.
- Scripts print resource identifiers before and after deployment.
- Scripts write a teardown checklist and fail if resources remain unexpectedly.
- Optional 10-bridge parity run requires explicit `--allow-10-bridge-parity`.

---

## Validation

Run from WSL2 Ubuntu:

```bash
uname -a | grep -i microsoft >/dev/null || { echo "Pass 4 tooling requires WSL2 Ubuntu" >&2; exit 1; }

cd prototype/gbn-bridge-proto
infra/scripts/aws-pass4-bridge-only-up.sh \
  --region ca-central-1 \
  --bridge-count 3 \
  --publisher-public-endpoint https://publisher.example.test

infra/scripts/aws-pass4-bridge-only-verify.sh \
  --region ca-central-1 \
  --expect-bridge-count 3 \
  --publisher-public-endpoint https://publisher.example.test \
  --require-no-public-admin \
  --require-local-publisher-signed-catalog
```

Expected artifacts:

- `aws_bridge_stack_outputs.json`;
- `aws_exitbridge_endpoint_map.json`;
- `aws_exitbridge_catalog_snapshot.json`;
- `cloudwatch_log_streams.json`;
- public reachability transcript;
- admin-denial transcript.

---

## Tests

Add focused tests for:

- bridge-only deploy script requires non-U.S. region input;
- default bridge count is 3;
- 10-bridge parity requires explicit confirmation flag;
- verify script fails when Publisher catalog lacks AWS bridge entries;
- verify script fails when an AWS bridge entry is unsigned or signed by the wrong trust
  root;
- verify script fails when public admin exposure is detected;
- down script reports remaining AWS resources.

Run:

```bash
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
shellcheck infra/scripts/aws-pass4-bridge-only-*.sh
```

---

## Acceptance Criteria

- ExitBridge-only AWS stack deploys in `ca-central-1`.
- Default deployment uses 3 bridges unless parity run is explicitly requested.
- No AWS Publisher or Receiver is deployed for Pass 4 geo validation.
- AWS bridges register with the local k8s Publisher over public internet.
- Local Publisher-signed catalog includes the expected AWS bridge entries.
- AWS bridge public protocol endpoints are reachable.
- AWS bridge admin endpoints are not public.
- CloudWatch evidence collection is ready for Phase 8.
- Teardown script is available and tested.
- V1 preservation checks return no files.
- Parent plan status tracker is updated.

---

## Completion Evidence

When this phase is implemented, archive:

- AWS deploy transcript;
- AWS verify transcript;
- stack outputs;
- endpoint map;
- Publisher-signed AWS bridge catalog snapshot;
- CloudWatch log stream map;
- public reachability and admin-denial transcripts;
- cost/teardown checklist;
- V1 preservation command output.
