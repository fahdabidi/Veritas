# Veritas Conduit V2 Infrastructure

This directory contains the V2-only deployment assets for the Conduit track.
These files are intentionally isolated from the frozen V1 Lattice workspace
under `prototype/gbn-proto/`.

There are now two V2 deployment surfaces in the repo:

- the earlier prototype stack used during `GBN-PROTO-005`
- the full-implementation stack introduced during `GBN-PROTO-006` Phase 8

## Naming Rules

| Surface | Convention | Example |
|---|---|---|
| Environment variables | `GBN_BRIDGE_` | `GBN_BRIDGE_PUBLISHER_URL` |
| Container images | `gbn-conduit-full-` | `gbn-conduit-full-authority` |
| CloudFormation stacks | `gbn-conduit-full-` | `gbn-conduit-full-dev` |
| Metrics namespace | `GBN/BridgeProto` | `GBN/BridgeProto` |

## Assets

| Path | Purpose |
|---|---|
| `../Dockerfile.bridge` | builds the V2 ExitBridge deployment binary |
| `../Dockerfile.bridge-publisher` | legacy prototype publisher image from the earlier simulation track |
| `../Dockerfile.publisher-authority` | builds the real Conduit publisher-authority service image |
| `../Dockerfile.publisher-receiver` | builds the real Conduit publisher-receiver service image |
| `../docker-compose.bridge-smoke.yml` | earlier BusyBox smoke-only placeholder topology |
| `../docker-compose.conduit-e2e.yml` | real local Conduit authority / receiver / bridge / Postgres topology |
| `cloudformation/phase2-bridge-stack.yaml` | deploys isolated ECS/Fargate publisher and ExitBridge services |
| `cloudformation/conduit-full-stack.yaml` | deploys the full Conduit authority / receiver / bridge / Postgres control plane |
| `cloudformation/parameters.json` | example parameter file with placeholders |
| `scripts/build-and-push.sh` | builds and pushes V2 images to ECR |
| `scripts/build-and-push-conduit-full.sh` | builds and pushes authority / receiver / bridge images for the full stack |
| `scripts/deploy-bridge-test.sh` | deploys the V2 CloudFormation stack |
| `scripts/deploy-conduit-full.sh` | deploys the full Conduit CloudFormation stack |
| `scripts/status-snapshot.sh` | prints stack and ECS service status |
| `scripts/bootstrap-smoke.sh` | verifies stack wiring and ECS task-definition environment |
| `scripts/smoke-conduit-full.sh` | prints stack outputs, ECS service counts, and trace-visible log groups for the full stack |
| `scripts/mobile-validation.sh` | runs the Phase 11 local proxy or AWS/mobile validation workflow |
| `scripts/mobile-validation-full.sh` | runs the GBN-PROTO-006 full implementation local or AWS/mobile validation workflow |
| `scripts/collect-bridge-metrics.sh` | collects ECS and CloudWatch evidence for a deployed Phase 10/11 stack |
| `scripts/collect-conduit-traces.sh` | collects full-stack CloudFormation, ECS, and CloudWatch `chain_id` evidence |
| `scripts/relay-control-interactive-v2.sh` | small interactive wrapper around status, smoke, and teardown |
| `scripts/teardown-bridge-test.sh` | deletes only `gbn-bridge-phase2-*` stacks |
| `scripts/teardown-conduit-full.sh` | deletes only `gbn-conduit-full-*` stacks |

## Prototype Flow

```bash
cd prototype/gbn-bridge-proto

infra/scripts/build-and-push.sh \
  --region us-east-1 \
  --tag phase10

infra/scripts/deploy-bridge-test.sh \
  --region us-east-1 \
  --stack-name gbn-bridge-phase2-dev \
  --environment dev \
  --vpc-id vpc-REPLACE_ME \
  --subnet-ids subnet-REPLACE_ME_A,subnet-REPLACE_ME_B \
  --publisher-image ACCOUNT_ID.dkr.ecr.us-east-1.amazonaws.com/gbn-bridge-proto-publisher:phase10 \
  --bridge-image ACCOUNT_ID.dkr.ecr.us-east-1.amazonaws.com/gbn-bridge-proto-exit-bridge:phase10

infra/scripts/bootstrap-smoke.sh \
  --region us-east-1 \
  --stack-name gbn-bridge-phase2-dev

infra/scripts/mobile-validation.sh \
  --mode aws \
  --region us-east-1 \
  --stack-name gbn-bridge-phase2-dev
```

## Full-Implementation Flow

```bash
cd prototype/gbn-bridge-proto

infra/scripts/build-and-push-conduit-full.sh \
  --region us-east-1 \
  --tag proto006-phase8

infra/scripts/deploy-conduit-full.sh \
  --region us-east-1 \
  --stack-name gbn-conduit-full-dev \
  --environment dev \
  --vpc-id vpc-REPLACE_ME \
  --service-subnet-ids subnet-REPLACE_ME_A,subnet-REPLACE_ME_B \
  --database-subnet-ids subnet-REPLACE_ME_C,subnet-REPLACE_ME_D \
  --authority-image ACCOUNT_ID.dkr.ecr.us-east-1.amazonaws.com/gbn-conduit-full-authority:proto006-phase8 \
  --receiver-image ACCOUNT_ID.dkr.ecr.us-east-1.amazonaws.com/gbn-conduit-full-receiver:proto006-phase8 \
  --bridge-image ACCOUNT_ID.dkr.ecr.us-east-1.amazonaws.com/gbn-conduit-full-bridge:proto006-phase8 \
  --publisher-signing-key-secret-arn arn:aws:secretsmanager:us-east-1:ACCOUNT_ID:secret:publisher-signing \
  --bridge-signing-seed-secret-arn arn:aws:secretsmanager:us-east-1:ACCOUNT_ID:secret:bridge-signing-seed \
  --publisher-public-key-hex REPLACE_ME

infra/scripts/smoke-conduit-full.sh \
  --region us-east-1 \
  --stack-name gbn-conduit-full-dev

infra/scripts/mobile-validation-full.sh \
  --mode local

infra/scripts/mobile-validation-full.sh \
  --mode aws \
  --region us-east-1 \
  --stack-name gbn-conduit-full-dev \
  --chain-id REPLACE_WITH_LIVE_CHAIN_ID \
  --mobile-context "carrier=REPLACE_ME;network=REPLACE_ME" \
  --require-chain-id
```

## Current Full-Implementation Boundary

The full-implementation surface now deploys three distinct service roles:

- `publisher-authority`
- `publisher-receiver`
- `exit-bridge`

Current service behavior:

- `publisher-authority` owns durable Postgres-backed authority state, bootstrap session orchestration, control command issuance, and receiver-path ingestion
- `publisher-receiver` is a dedicated receiver-facing network service that proxies receiver traffic to the authority service instead of exposing the monolithic prototype entrypoint
- `exit-bridge` runs a real control-session / heartbeat / forwarder loop instead of the earlier placeholder process

Current trace boundary:

- `chain_id` remains present in authority responses
- the receiver proxy preserves raw request/response payloads and logs `chain_id` when present
- bridge logs preserve `chain_id` on applied command ACKs and control/bootstrap progress

The full stack is deployment-capable, but it is still pending live AWS/mobile evidence from later Proto006 phases.

Phase 10 full-validation evidence should be collected with:

- `scripts/mobile-validation-full.sh`
- `scripts/collect-conduit-traces.sh`
- `docs/prototyping/Conduit/Full-Implementation-Plan/GBN-PROTO-006-Conduit-Full-Implementation-Test-Report.md`

## Current Prototype Boundary

The Phase 10 deployment assets validate V2-only stack isolation, image naming,
ECS task wiring, `GBN_BRIDGE_*` environment variables, the default UDP punch
port, and the publisher batch-window setting.

The current binaries are deployment entrypoints for the in-process Conduit
prototype. They keep ECS tasks alive and expose validated configuration, but
they do not yet provide a production network service. Treat live AWS
first-contact bootstrap as a manual prototype scenario until the deployment
entrypoints are replaced by full network listeners.

## V1 Preservation

Do not modify or call V1 deployment files from this directory. In particular,
the Conduit deployment work must not edit:

- `prototype/gbn-proto/infra/cloudformation/**`
- `prototype/gbn-proto/infra/scripts/**`
- `prototype/gbn-proto/Dockerfile.relay`
- `prototype/gbn-proto/Dockerfile.publisher`
