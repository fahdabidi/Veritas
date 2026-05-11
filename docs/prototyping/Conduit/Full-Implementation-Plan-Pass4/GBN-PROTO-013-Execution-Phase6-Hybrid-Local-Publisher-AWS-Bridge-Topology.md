# GBN-PROTO-013 - Execution Phase 6 - Hybrid Local-Publisher / AWS-Bridge Topology

**Status:** Pending
**Last Updated:** 2026-05-11
**Parent Plan:** [GBN-PROTO-013](GBN-PROTO-013-Conduit-Mobile-Creator-Public-Internet-Validation-Execution-Plan.md)
**Depends On:** Phases 1-5 complete

## Objective

Prepare the hybrid public-internet topology for AWS geolocation validation:

- the Publisher remains the existing local k8s Publisher;
- the local k8s HostCreator remains the QR bootstrap source;
- AWS runs only ExitBridge tasks in a non-U.S. region;
- AWS ExitBridges register with the local k8s Publisher over the public internet;
- admin access remains private through WSL2/k8s tooling and AWS ECS Exec.

This phase creates the topology plan and control scripts. Phase 7 deploys the non-U.S.
ExitBridge fleet, and Phase 8 runs the mobile app against it.

Update the parent plan status tracker when this phase is complete.

---

## Topology

```text
Android phone
  -> public HostCreator bootstrap endpoint (local k8s)
  -> public Publisher protocol endpoint (local k8s)
  -> AWS ExitBridge public endpoints in ca-central-1

AWS ExitBridges in ca-central-1
  -> register/progress with local k8s Publisher public endpoint
  -> emit logs to CloudWatch

Local k8s Publisher/Receiver
  -> remains authority and receiver
  -> signs bridge catalog containing AWS bridge descriptors
  -> emits local logs/traces
```

There is no AWS Publisher in Pass 4. Any AWS stack component that would create or expose a
Publisher for this validation must be disabled or omitted.

---

## AWS Bridge-Only Stack Requirements

Create or adapt an AWS deployment path that can run ExitBridge-only services:

- region: `ca-central-1`;
- default bridge count: 3 for cost-minimum proof;
- optional bridge count: 10 for parity run;
- no public admin listener;
- CloudWatch log group per bridge service or structured stream naming per bridge id;
- security group allows only required public protocol ports and egress to the local
  Publisher public endpoint;
- IAM task role includes only needed logging, ECS Exec, and runtime permissions;
- ECS Exec is enabled for private operator diagnostics;
- public endpoint descriptors include AWS region and bridge id.

The stack must accept the local k8s Publisher public endpoint as configuration and must
not synthesize a second Publisher authority.

---

## Cross-Environment Registration

AWS ExitBridges register with the local k8s Publisher over the public internet.

Required registration fields:

| Field | Requirement |
|---|---|
| `bridge_id` | Stable per AWS task/service instance |
| `identity_pub` | Bridge public identity key |
| `ingress_endpoints[]` | Public AWS endpoints reachable by phone |
| `region` | `ca-central-1` for first run |
| `reachability_class` | `direct` unless the deployment proves a relay-only path |
| `capabilities[]` | Must include mobile upload/dummy routing capabilities |
| `chain_id` | Registration/progress ChainID |

Publisher signs the resulting bridge entries only after verifying required fields and
reachability metadata.

---

## Local k8s Publisher Configuration

Phase 6 extends the Phase 4 public endpoint map with a hybrid bridge catalog mode:

```json
{
  "profile": "hybrid_local_publisher_aws_bridges",
  "publisher_public_endpoint": "https://publisher.example.test",
  "hostcreator_bootstrap_endpoint": "https://hostcreator.example.test",
  "aws_exitbridge_region": "ca-central-1",
  "aws_exitbridge_count": 3,
  "catalog_source": "local_publisher_signed"
}
```

The local Publisher must be able to distinguish local k8s ExitBridge entries from AWS
ExitBridge entries in DHT/catalog dumps and trace evidence.

---

## Operator Scripts

Add or adapt:

```text
prototype/gbn-bridge-proto/infra/scripts/aws-pass4-bridge-only-plan.sh
prototype/gbn-bridge-proto/infra/scripts/aws-pass4-bridge-only-up.sh
prototype/gbn-bridge-proto/infra/scripts/aws-pass4-bridge-only-down.sh
prototype/gbn-bridge-proto/infra/scripts/k8s-pass4-hybrid-catalog-verify.sh
```

Expected behavior:

- guard for WSL2 Ubuntu;
- require explicit `--region ca-central-1`;
- require explicit `--publisher-public-endpoint`;
- print estimated bridge count and expected resources before deploy;
- deploy only ExitBridge services;
- verify bridge public endpoints;
- verify bridges register with local Publisher;
- fetch CloudWatch log group names and stream names;
- write `hybrid_endpoint_map.json`;
- tear down AWS resources after validation.

---

## Evidence Model

Hybrid evidence spans three locations:

| Location | Evidence |
|---|---|
| Android app | mobile runtime events, local DHT, ChainIDs, selected AWS bridge ids, S3 bundle |
| Local k8s | Publisher authority/receiver logs, HostCreator logs, signed bridge catalog, observability traces |
| AWS `ca-central-1` | ExitBridge CloudWatch logs, ECS task metadata, endpoint descriptors, ECS Exec admin-denial checks |

The mobile app evidence bundle must include CloudWatch query hints for selected AWS bridge
ids and local k8s query hints for Publisher/HostCreator ChainIDs.

---

## Validation

Run from WSL2 Ubuntu:

```bash
uname -a | grep -i microsoft >/dev/null || { echo "Pass 4 tooling requires WSL2 Ubuntu" >&2; exit 1; }

cd prototype/gbn-bridge-proto
infra/scripts/k8s-pass4-public-ingress-verify.sh \
  --profile local_k8s_public \
  --require-no-public-admin

infra/scripts/aws-pass4-bridge-only-plan.sh \
  --region ca-central-1 \
  --bridge-count 3 \
  --publisher-public-endpoint https://publisher.example.test

infra/scripts/k8s-pass4-hybrid-catalog-verify.sh \
  --expect-region ca-central-1 \
  --expect-aws-bridge-count 0 \
  --predeploy
```

This phase can stop before deployment. Phase 7 runs `aws-pass4-bridge-only-up.sh`.

---

## Tests

Add focused tests for:

- AWS bridge-only template excludes Publisher and Receiver services;
- bridge task configuration requires local Publisher public endpoint;
- security group does not expose admin ports;
- bridge descriptor contains region and public endpoint fields;
- local Publisher rejects malformed AWS bridge registration;
- hybrid catalog verifier distinguishes local and AWS bridge entries;
- CloudWatch query hints are generated for each AWS bridge id.

Run:

```bash
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
shellcheck infra/scripts/aws-pass4-bridge-only-*.sh infra/scripts/k8s-pass4-hybrid-catalog-verify.sh
```

---

## Acceptance Criteria

- Hybrid topology explicitly keeps Publisher in local k8s.
- AWS plan creates ExitBridge-only resources in `ca-central-1`.
- Local Publisher public endpoint is the only Publisher authority configured for AWS
  bridges.
- Admin ports are not public in AWS or local k8s.
- CloudWatch log collection plan is defined.
- Hybrid endpoint map schema is documented and generated.
- Phase 5 local mobile validation remains green before hybrid deployment proceeds.
- V1 preservation checks return no files.
- Parent plan status tracker is updated.

---

## Completion Evidence

When this phase is implemented, archive:

- AWS bridge-only plan output;
- hybrid endpoint map example;
- local Publisher public endpoint verification;
- security group/admin exposure review;
- CloudWatch query hint sample;
- Phase 5 evidence reference;
- V1 preservation command output.
