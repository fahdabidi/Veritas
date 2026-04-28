# Veritas Conduit V2 Infrastructure

**A deployment and validation guide for the Conduit bridge-mode prototype: the V2 path that turns Veritas from a local simulation into a distributed publisher, bridge, receiver, and traceable AWS/mobile test system.**

This README is the Conduit infrastructure companion to the released Lattice-facing [root README](../../../README.md). It mirrors the same practical structure, but the scope here is narrower: build, deploy, validate, observe, and tear down the Conduit full-implementation stack without touching the frozen Lattice V1 baseline.

Latest public baseline: [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/latest)

Lattice baseline freeze: [veritas-lattice-0.1.0-baseline](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)

Architecture tracks:

- `Lattice`: V1 onion-mode baseline frozen at [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)
- `Conduit`: V2 bridge-mode architecture under active full-implementation validation

Conduit references:

- System architecture: [GBN-ARCH-000-System-Architecture-V2.md](../../../docs/architecture/GBN-ARCH-000-System-Architecture-V2.md)
- MCN architecture: [GBN-ARCH-001-Media-Creation-Network-V2.md](../../../docs/architecture/GBN-ARCH-001-Media-Creation-Network-V2.md)
- Full implementation plan: [GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md](../../../docs/prototyping/Conduit/Full-Implementation-Plan/GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md)
- Phase 10 validation plan: [GBN-PROTO-006-Execution-Phase10-Live-AWS-And-Mobile-Validation.md](../../../docs/prototyping/Conduit/Full-Implementation-Plan/GBN-PROTO-006-Execution-Phase10-Live-AWS-And-Mobile-Validation.md)
- Test report: [GBN-PROTO-006-Conduit-Full-Implementation-Test-Report.md](../../../docs/prototyping/Conduit/Full-Implementation-Plan/GBN-PROTO-006-Conduit-Full-Implementation-Test-Report.md)

> "Truth needs infrastructure" becomes operational here: real service boundaries, durable authority state, deployment images, AWS smoke validation, and end-to-end `chain_id` evidence.

---

## Project Status

This directory contains the V2-only deployment assets for the Conduit track. These files are intentionally isolated from the frozen V1 Lattice workspace under `prototype/gbn-proto/`.

Current state:

- Conduit full implementation has real deployment images for `publisher-authority`, `publisher-receiver`, and `exit-bridge`.
- The AWS full stack uses ECS/Fargate, Cloud Map service discovery, RDS Postgres, Secrets Manager, and CloudWatch Logs.
- Phase 10 minimal AWS smoke validation has passed against `gbn-conduit-full-dev` in `us-east-1`.
- The minimal smoke stack used one authority, one receiver, and one bridge with `DesiredBridgeCount=1`.
- Full mobile-carrier validation is still pending and must be captured before a final production-readiness decision.
- Root `README.md` remains release-facing and should not be edited for Conduit implementation work until the V2 release is ready.

Current minimal AWS smoke evidence:

| Evidence | Value |
|---|---|
| Stack | `gbn-conduit-full-dev` |
| Region | `us-east-1` |
| Scope | one authority, one receiver, one bridge |
| Bridge count | `DesiredBridgeCount=1` |
| Image tag | `proto006-phase10-fix2` |
| Stack status | `UPDATE_COMPLETE` |
| ECS steady state | authority `1/1`, receiver `1/1`, bridge `1/1` |
| Artifact directory | `/tmp/veritas-proto006-phase10-aws-artifacts-fix2` |

Remaining validation gap:

- Run a real mobile-network path against a deployed `gbn-conduit-full-*` stack.
- Capture explicit `chain_id` evidence with `collect-conduit-traces.sh --chain-id <id> --require-chain-id`.
- Record bootstrap, upload/ACK, failover/churn, and batch-window observations in the Phase 10 test report.

---

## Quick Start

### Prerequisites

Run these commands from WSL on this host. The project tooling, Docker, and valid AWS credentials are expected there.

- Rust toolchain
- Docker with BuildKit support
- AWS CLI v2 authenticated to the target account
- `python3`
- Existing VPC and at least two service subnets
- Existing database subnets for RDS
- Secrets Manager values for publisher and bridge signing material

Confirm the active AWS identity:

```bash
aws sts get-caller-identity
```

Confirm Docker is responsive:

```bash
docker ps
```

### 1) Validate the Conduit workspace locally

```bash
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
```

If OneDrive-backed writes are slow or the default `target/` path is unreliable, use a temp target directory:

```bash
cd prototype/gbn-bridge-proto
cargo test --workspace --target-dir /tmp/veritas-conduit-target
```

### 2) Run the distributed local e2e harness

```bash
cd prototype/gbn-bridge-proto
VERITAS_BRIDGE_TARGET_DIR=/tmp/veritas-proto006-e2e-target \
VERITAS_CONDUIT_E2E_ARTIFACT_DIR=/tmp/veritas-proto006-e2e-artifacts \
  infra/scripts/run-conduit-e2e.sh
```

This validates the local distributed control/data-path harness before spending time or money on AWS.

### 3) Build and push Conduit images

```bash
cd prototype/gbn-bridge-proto
infra/scripts/build-and-push-conduit-full.sh \
  --region us-east-1 \
  --tag proto006-phase10-fix2
```

The script creates missing ECR repositories and pushes:

- `gbn-conduit-full-authority`
- `gbn-conduit-full-receiver`
- `gbn-conduit-full-bridge`

### 4) Deploy a minimal AWS smoke stack

Use `DesiredBridgeCount=1` for smoke validation. Do not scale up until the one-bridge topology is steady.

```bash
cd prototype/gbn-bridge-proto
infra/scripts/deploy-conduit-full.sh \
  --region us-east-1 \
  --stack-name gbn-conduit-full-dev \
  --environment dev \
  --desired-bridge-count 1 \
  --vpc-id vpc-REPLACE_ME \
  --service-subnet-ids subnet-REPLACE_ME_A,subnet-REPLACE_ME_B \
  --database-subnet-ids subnet-REPLACE_ME_C,subnet-REPLACE_ME_D \
  --authority-image ACCOUNT_ID.dkr.ecr.us-east-1.amazonaws.com/gbn-conduit-full-authority:proto006-phase10-fix2 \
  --receiver-image ACCOUNT_ID.dkr.ecr.us-east-1.amazonaws.com/gbn-conduit-full-receiver:proto006-phase10-fix2 \
  --bridge-image ACCOUNT_ID.dkr.ecr.us-east-1.amazonaws.com/gbn-conduit-full-bridge:proto006-phase10-fix2 \
  --publisher-signing-key-secret-arn arn:aws:secretsmanager:us-east-1:ACCOUNT_ID:secret:publisher-signing \
  --bridge-signing-seed-secret-arn arn:aws:secretsmanager:us-east-1:ACCOUNT_ID:secret:bridge-signing-seed \
  --publisher-public-key-hex REPLACE_ME
```

Smoke-only TLS escape hatch:

```bash
  --postgres-tls-accept-invalid-certs true
```

Use that flag only for development smoke stacks when the container image cannot validate the RDS CA chain. Production validation should provide the RDS CA bundle through `GBN_BRIDGE_POSTGRES_TLS_CA_PEM` or `GBN_BRIDGE_POSTGRES_TLS_CA_FILE` and keep invalid certificate acceptance disabled.

### 5) Run AWS smoke validation

```bash
cd prototype/gbn-bridge-proto
infra/scripts/smoke-conduit-full.sh \
  --region us-east-1 \
  --stack-name gbn-conduit-full-dev
```

Expected result:

- stack outputs print successfully
- authority service `desired=1`, `running=1`
- receiver service `desired=1`, `running=1`
- bridge service `desired=1`, `running=1`
- command exits non-zero if any service is below desired running count

### 6) Collect Phase 10 AWS evidence

```bash
cd prototype/gbn-bridge-proto
infra/scripts/mobile-validation-full.sh \
  --mode aws \
  --region us-east-1 \
  --stack-name gbn-conduit-full-dev \
  --artifact-dir /tmp/veritas-proto006-phase10-aws-artifacts \
  --window-minutes 60 \
  --mobile-context "minimal-aws-smoke"
```

For final mobile validation, pass a real chain ID and require it in all service logs:

```bash
cd prototype/gbn-bridge-proto
infra/scripts/mobile-validation-full.sh \
  --mode aws \
  --region us-east-1 \
  --stack-name gbn-conduit-full-dev \
  --artifact-dir /tmp/veritas-proto006-phase10-mobile-artifacts \
  --window-minutes 60 \
  --chain-id REPLACE_WITH_LIVE_CHAIN_ID \
  --mobile-context "carrier=REPLACE_ME;network=REPLACE_ME" \
  --require-chain-id
```

### 7) Tear down when finished

Only delete Conduit full-implementation stacks with this script:

```bash
cd prototype/gbn-bridge-proto
infra/scripts/teardown-conduit-full.sh \
  --region us-east-1 \
  --stack-name gbn-conduit-full-dev
```

---

## Vision & Mission

Lattice validated the first Veritas baseline with onion-style relay behavior. Conduit is the next architecture track: a bridge-mode system where a real Publisher authority coordinates signed bridge catalogs, bootstrap distribution, bridge control sessions, receiver traffic, ACKs, and observable distributed traces.

For validation, the infrastructure goal is specific:

- prove the Publisher is a real service boundary, not an in-process simulation
- prove ExitBridges can register, renew, receive commands, and forward data through deployed services
- prove the receiver path exists as a separate deployed service
- prove durable Publisher state survives service restarts through Postgres
- prove a single `chain_id` can be followed across authority, bridge, receiver, and validation artifacts
- prove the system can be exercised from AWS and, before final sign-off, from a real mobile network path

The infrastructure is not just deployment plumbing. It is how Conduit proves that the architecture is real.

### Design Principles

| Principle | What It Means In Conduit Infrastructure |
|---|---|
| V1 preservation | No Conduit infra task should edit or depend on `prototype/gbn-proto/**` |
| Service boundaries first | Authority, receiver, and bridge run as separate deployed services |
| Minimal before scaled | Validate one authority, one receiver, and one bridge before increasing bridge count |
| Evidence over assumptions | Every validation run should produce artifacts, logs, stack identity, and trace records |
| `chain_id` continuity | Every correlated bootstrap, upload, ACK, and progress path must preserve `chain_id` |
| Safe teardown | Conduit teardown scripts only delete `gbn-conduit-full-*` stacks |
| Production honesty | Smoke-only shortcuts must be labeled and removed before production validation |

---

## How It Works

The Conduit full stack deploys three service roles and one durable state layer.

```text
Creator / HostCreator
        |
        | bootstrap, refresh, upload requests
        v
+----------------------+
| Publisher Authority  |
| - bridge catalog     |
| - leases             |
| - bootstrap sessions |
| - control commands   |
| - progress records   |
+----------+-----------+
           |
           | control websocket / command polling
           v
+----------------------+
| ExitBridge           |
| - registers lease    |
| - renews heartbeat   |
| - receives fanout    |
| - forwards payloads  |
+----------+-----------+
           |
           | receiver path
           v
+----------------------+
| Publisher Receiver   |
| - receiver endpoint  |
| - forwards to auth   |
| - preserves chain_id |
+----------+-----------+
           |
           v
+----------------------+
| Postgres             |
| - durable authority  |
| - bootstrap state    |
| - bridge leases      |
| - progress / ACKs    |
+----------------------+
```

### First-Time Creator Bootstrap Target

The V2 architecture requires this production-shaped flow:

1. A `NewCreator` pairs with a `HostCreator`.
2. The `HostCreator` uses an existing bridge path to request network entry from the Publisher.
3. The Publisher creates a signed bootstrap payload containing the new creator entry and a seed bridge set.
4. The Publisher selects an active `ExitBridgeB` and instructs it to start punching toward the `NewCreator`.
5. The `NewCreator` receives the seed bridge details through the existing path.
6. `NewCreator` and `ExitBridgeB` establish a tunnel and ACK progress.
7. The seed bridge returns the signed bridge catalog.
8. The Publisher fans out commands to additional bridges.
9. Every progress event preserves the same `chain_id`.

Phase 10 minimal AWS smoke does not prove the full mobile version of this path. It proves the deployed services are alive, connected, and producing traceable logs. The full mobile validation run must still exercise the real network path.

### Returning Creator Refresh Target

For returning creators, the expected flow is:

```text
Creator
  -> load cached signed bridge descriptors
  -> verify Publisher signatures
  -> select a direct bridge
  -> connect
  -> request fresh bridge catalog
Publisher
  -> return updated bridge list
Creator
  -> store signed entries
  -> start UDP punch probes
ExitBridges
  -> punch back
Creator + ExitBridges
  -> ACK working tunnels
  -> report progress to Publisher
```

Validation should record whether the refresh completes, how long it takes, which bridge entries were used, and whether the same `chain_id` appears in the relevant authority, bridge, receiver, and artifact logs.

---

## Conduit Flow Packet Path

Conduit replaces the Lattice relay onion path with a Publisher-coordinated bridge path.

Current deployed service path:

```text
ExitBridge
  -> PublisherAuthority: register lease
  -> PublisherAuthority: renew heartbeat
  -> PublisherAuthority: receive control commands
  -> PublisherAuthority: report bootstrap / fanout progress
  -> PublisherReceiver: forward receiver-bound data
PublisherReceiver
  -> PublisherAuthority: proxy receiver event / ACK path
PublisherAuthority
  -> Postgres: persist bridge, catalog, bootstrap, progress, ACK state
```

Current AWS service discovery:

| Service | Internal name |
|---|---|
| Publisher Authority | `publisher-authority.conduit-<env>.internal:<authority-port>` |
| Publisher Receiver | `publisher-receiver.conduit-<env>.internal:<receiver-port>` |
| Bridge control URL | `ws://publisher-authority.conduit-<env>.internal:<authority-port>/v1/bridge/control` |

Current ports:

| Port | Default | Purpose |
|---|---:|---|
| Authority HTTP | `8080` | authority API and bridge control |
| Receiver HTTP | `8081` | receiver-facing service |
| UDP punch | `443` | signed bridge punch/tunnel port |

---

## ChainID Trace Design

`chain_id` is the root distributed trace identifier carried forward from the V1 implementation. Do not replace it with a competing field name.

Validation expectations:

- Creator-originated bootstrap or upload flow originates or carries one root `chain_id`.
- Authority logs include the `chain_id` for correlated bootstrap, catalog, progress, receiver, and ACK events.
- Bridge logs include the `chain_id` for applied commands and reported progress.
- Receiver logs include the `chain_id` when proxying or acknowledging receiver-path traffic.
- Test artifacts include the `chain_id`, stack identity, service status, and raw log extracts.

Trace collection command:

```bash
cd prototype/gbn-bridge-proto
infra/scripts/collect-conduit-traces.sh \
  --region us-east-1 \
  --stack-name gbn-conduit-full-dev \
  --window-minutes 60 \
  --chain-id REPLACE_WITH_LIVE_CHAIN_ID \
  --artifact-dir /tmp/veritas-proto006-chain-trace \
  --require-chain-id
```

The `--require-chain-id` flag should be used for final evidence. It fails if any required service has no matching events.

---

## Repository Layout

| Path | Purpose |
|---|---|
| `../Cargo.toml` | Conduit Rust workspace |
| `../Dockerfile.bridge` | builds the V2 ExitBridge deployment binary |
| `../Dockerfile.bridge-publisher` | legacy prototype publisher image from the earlier simulation track |
| `../Dockerfile.publisher-authority` | builds the real Conduit publisher-authority service image |
| `../Dockerfile.publisher-receiver` | builds the real Conduit publisher-receiver service image |
| `../docker-compose.bridge-smoke.yml` | earlier BusyBox smoke-only placeholder topology |
| `../docker-compose.conduit-e2e.yml` | local authority / receiver / bridge / Postgres topology |
| `../docs/mobile-test-matrix.md` | Conduit mobile and AWS validation matrix |
| `cloudformation/phase2-bridge-stack.yaml` | earlier isolated V2 bridge prototype stack |
| `cloudformation/conduit-full-stack.yaml` | full Conduit authority / receiver / bridge / Postgres stack |
| `cloudformation/parameters.json` | example parameter file with placeholders |
| `scripts/` | build, deploy, smoke, validation, trace, and teardown scripts |

---

## Technical Stack

| Layer | Current Tooling |
|---|---|
| Language | Rust |
| Containers | Docker |
| Local topology | Docker Compose and Rust e2e harness |
| Compute | AWS ECS/Fargate |
| Service discovery | AWS Cloud Map private DNS |
| Database | AWS RDS Postgres |
| Secrets | AWS Secrets Manager |
| Logs | AWS CloudWatch Logs |
| Image registry | AWS ECR |
| Deployment | AWS CloudFormation |
| Validation scripts | Bash, AWS CLI, Python helper snippets |

---

## Full-Implementation Phases

The current infrastructure is part of GBN-PROTO-006.

| Phase | Status | Infra Relevance |
|---|---|---|
| Phase 0 | complete | simulation baseline and gap inventory |
| Phase 1 | complete | real publisher authority API |
| Phase 2 | complete | durable Postgres-backed storage |
| Phase 3 | complete | bridge control sessions |
| Phase 4 | complete | network clients replacing in-process clients |
| Phase 5 | complete | bootstrap distribution and fanout |
| Phase 6 | complete | receiver and ACK path |
| Phase 7 | complete | distributed `chain_id` propagation |
| Phase 8 | complete | real deployment images and AWS control plane |
| Phase 9 | complete | distributed e2e harness and fault injection |
| Phase 10 | in validation | live AWS/mobile validation |
| Phase 11 | pending | decision gate |

Phase 10 is the current infra focus. It is not enough to say the stack deployed; the stack must produce evidence.

---

## Security Model Summary

Conduit validation should preserve these boundaries:

- Publisher signing keys are injected through Secrets Manager, not committed to the repo.
- Bridge signing seed is injected through Secrets Manager, not committed to the repo.
- Postgres password is generated/stored by the stack in Secrets Manager.
- Development TLS certificate bypass is smoke-only.
- V1 Lattice assets remain untouched.
- AWS stack names are constrained by scripts to prevent accidental deletion of unrelated infrastructure.
- Trace artifacts may contain operational metadata and should be treated as sensitive until reviewed.

Important limitations:

- Minimal AWS smoke is not a substitute for real mobile-carrier validation.
- One-bridge smoke does not prove multi-bridge fanout or churn behavior.
- `--postgres-tls-accept-invalid-certs true` is not production-safe.
- CloudWatch `chain_id` evidence proves observability, not cryptographic correctness by itself.

Before sharing artifacts, scan for secrets:

```bash
python ../../../tools/scan_secrets.py ../../../ --fail-on-findings
```

---

## Documentation Index

| Document | Purpose |
|---|---|
| [Root README](../../../README.md) | release-facing project overview |
| [GBN-ARCH-000 V2](../../../docs/architecture/GBN-ARCH-000-System-Architecture-V2.md) | Conduit system architecture |
| [GBN-ARCH-001 V2](../../../docs/architecture/GBN-ARCH-001-Media-Creation-Network-V2.md) | V2 MCN flow and publisher responsibilities |
| [GBN-PROTO-006 Execution Plan](../../../docs/prototyping/Conduit/Full-Implementation-Plan/GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md) | full implementation phase plan |
| [Phase 10 Plan](../../../docs/prototyping/Conduit/Full-Implementation-Plan/GBN-PROTO-006-Execution-Phase10-Live-AWS-And-Mobile-Validation.md) | live AWS/mobile validation plan |
| [Full Implementation Test Report](../../../docs/prototyping/Conduit/Full-Implementation-Plan/GBN-PROTO-006-Conduit-Full-Implementation-Test-Report.md) | canonical validation evidence report |
| [Mobile Test Matrix](../docs/mobile-test-matrix.md) | validation scenarios and thresholds |

---

## AWS Test Setup And Scripts

### Naming Rules

| Surface | Convention | Example |
|---|---|---|
| Environment variables | `GBN_BRIDGE_` | `GBN_BRIDGE_PUBLISHER_URL` |
| Container images | `gbn-conduit-full-` | `gbn-conduit-full-authority` |
| CloudFormation stacks | `gbn-conduit-full-` | `gbn-conduit-full-dev` |
| Metrics namespace | `GBN/BridgeProto` | `GBN/BridgeProto` |
| Artifact directories | explicit `/tmp/veritas-*` path | `/tmp/veritas-proto006-phase10-aws-artifacts` |

### Important Scripts

| Script | Purpose |
|---|---|
| `scripts/build-and-push-conduit-full.sh` | builds and pushes authority, receiver, and bridge images |
| `scripts/deploy-conduit-full.sh` | deploys the full Conduit CloudFormation stack |
| `scripts/smoke-conduit-full.sh` | verifies stack outputs and ECS running counts |
| `scripts/mobile-validation-full.sh` | runs local or AWS Phase 10 validation workflow |
| `scripts/collect-conduit-traces.sh` | collects CloudFormation, ECS, and CloudWatch `chain_id` evidence |
| `scripts/teardown-conduit-full.sh` | deletes only `gbn-conduit-full-*` stacks |
| `scripts/run-conduit-e2e.sh` | runs the distributed local e2e harness |
| `scripts/status-snapshot.sh` | legacy prototype stack status helper |
| `scripts/build-and-push.sh` | legacy prototype image build helper |
| `scripts/deploy-bridge-test.sh` | legacy prototype stack deploy helper |
| `scripts/teardown-bridge-test.sh` | deletes only legacy `gbn-bridge-phase2-*` stacks |

### What The Full Stack Creates

`cloudformation/conduit-full-stack.yaml` creates:

- ECS cluster
- Fargate service for `publisher-authority`
- Fargate service for `publisher-receiver`
- Fargate service for `exit-bridge`
- Cloud Map private DNS namespace
- RDS Postgres instance
- generated database credentials secret
- task execution role
- service task role
- security groups
- CloudWatch log groups
- service outputs used by validation scripts

### Required Deployment Inputs

| Input | Why It Is Required |
|---|---|
| VPC ID | network boundary for ECS and RDS |
| service subnet IDs | ECS task placement |
| database subnet IDs | RDS subnet group |
| authority image URI | deployed publisher authority binary |
| receiver image URI | deployed receiver binary |
| bridge image URI | deployed exit bridge binary |
| publisher signing key secret ARN | signs authority-owned catalogs and responses |
| bridge signing seed secret ARN | signs or derives bridge identity material |
| publisher public key hex | lets bridges and creators verify authority material |

### Current Runtime Environment Variables

Publisher authority:

- `GBN_BRIDGE_PUBLISHER_BIND_ADDR`
- `GBN_BRIDGE_POSTGRES_HOST`
- `GBN_BRIDGE_POSTGRES_PORT`
- `GBN_BRIDGE_POSTGRES_DATABASE`
- `GBN_BRIDGE_POSTGRES_USER`
- `GBN_BRIDGE_POSTGRES_SCHEMA`
- `GBN_BRIDGE_POSTGRES_SSLMODE`
- `GBN_BRIDGE_POSTGRES_TLS_ACCEPT_INVALID_CERTS`
- `GBN_BRIDGE_POSTGRES_PASSWORD`
- `GBN_BRIDGE_PUBLISHER_SIGNING_MODE`
- `GBN_BRIDGE_PUBLISHER_SIGNING_KEY_HEX`

Publisher receiver:

- `GBN_BRIDGE_RECEIVER_BIND_ADDR`
- `GBN_BRIDGE_AUTHORITY_URL`

Exit bridge:

- `GBN_BRIDGE_NODE_ID`
- `GBN_BRIDGE_INGRESS_HOST`
- `GBN_BRIDGE_AUTHORITY_URL`
- `GBN_BRIDGE_RECEIVER_URL`
- `GBN_BRIDGE_CONTROL_URL`
- `GBN_BRIDGE_PUBLISHER_PUBLIC_KEY_HEX`
- `GBN_BRIDGE_REACHABILITY_CLASS`
- `GBN_BRIDGE_PUNCH_PORT`
- `GBN_BRIDGE_CONTROL_KEEPALIVE_INTERVAL_MS`
- `GBN_BRIDGE_POLL_INTERVAL_MS`
- `GBN_BRIDGE_BRIDGE_SIGNING_SEED_HEX`

---

## Typical Validation Workflow

Use this sequence for Phase 10 validation.

### 1) Confirm local and AWS preflight

```bash
aws sts get-caller-identity
docker ps
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
```

### 2) Run local e2e

```bash
cd prototype/gbn-bridge-proto
infra/scripts/mobile-validation-full.sh \
  --mode local \
  --target-dir /tmp/veritas-proto006-phase10-local-target \
  --artifact-dir /tmp/veritas-proto006-phase10-local-artifacts
```

### 3) Build images

```bash
cd prototype/gbn-bridge-proto
infra/scripts/build-and-push-conduit-full.sh \
  --region us-east-1 \
  --tag proto006-phase10-validation
```

### 4) Deploy minimal smoke topology

```bash
cd prototype/gbn-bridge-proto
infra/scripts/deploy-conduit-full.sh \
  --region us-east-1 \
  --stack-name gbn-conduit-full-dev \
  --environment dev \
  --desired-bridge-count 1 \
  --vpc-id vpc-REPLACE_ME \
  --service-subnet-ids subnet-REPLACE_ME_A,subnet-REPLACE_ME_B \
  --database-subnet-ids subnet-REPLACE_ME_C,subnet-REPLACE_ME_D \
  --authority-image ACCOUNT_ID.dkr.ecr.us-east-1.amazonaws.com/gbn-conduit-full-authority:proto006-phase10-validation \
  --receiver-image ACCOUNT_ID.dkr.ecr.us-east-1.amazonaws.com/gbn-conduit-full-receiver:proto006-phase10-validation \
  --bridge-image ACCOUNT_ID.dkr.ecr.us-east-1.amazonaws.com/gbn-conduit-full-bridge:proto006-phase10-validation \
  --publisher-signing-key-secret-arn arn:aws:secretsmanager:us-east-1:ACCOUNT_ID:secret:publisher-signing \
  --bridge-signing-seed-secret-arn arn:aws:secretsmanager:us-east-1:ACCOUNT_ID:secret:bridge-signing-seed \
  --publisher-public-key-hex REPLACE_ME
```

### 5) Run smoke

```bash
cd prototype/gbn-bridge-proto
infra/scripts/smoke-conduit-full.sh \
  --region us-east-1 \
  --stack-name gbn-conduit-full-dev
```

### 6) Run AWS evidence collection

```bash
cd prototype/gbn-bridge-proto
infra/scripts/mobile-validation-full.sh \
  --mode aws \
  --region us-east-1 \
  --stack-name gbn-conduit-full-dev \
  --artifact-dir /tmp/veritas-proto006-phase10-aws-artifacts \
  --window-minutes 60 \
  --mobile-context "minimal-aws-smoke"
```

### 7) Run final mobile-chain trace capture

```bash
cd prototype/gbn-bridge-proto
infra/scripts/collect-conduit-traces.sh \
  --region us-east-1 \
  --stack-name gbn-conduit-full-dev \
  --window-minutes 60 \
  --chain-id REPLACE_WITH_LIVE_CHAIN_ID \
  --artifact-dir /tmp/veritas-proto006-phase10-chain-artifacts \
  --require-chain-id
```

### 8) Update the test report

Record:

- stack name and region
- image tag or digest set
- exact bridge count
- mobile carrier / network path
- validation artifact directory
- observed bootstrap timing
- observed upload / ACK timing
- observed failover / churn timing
- chain-specific trace result
- anomalies and blockers

Use [GBN-PROTO-006-Conduit-Full-Implementation-Test-Report.md](../../../docs/prototyping/Conduit/Full-Implementation-Plan/GBN-PROTO-006-Conduit-Full-Implementation-Test-Report.md).

### 9) Confirm V1 preservation

From the repo root:

```bash
git diff --name-only -- \
  prototype/gbn-proto \
  docs/prototyping/Lattice \
  docs/architecture/GBN-PROTO-004-Phase2-Serverless-Scale-Onion-Plan.md \
  docs/prototyping/Lattice/GBN-PROTO-004-Phase2-Serverless-Scale-Onion-Plan.md
```

Expected result: no output.

### 10) Tear down if the stack is no longer needed

```bash
cd prototype/gbn-bridge-proto
infra/scripts/teardown-conduit-full.sh \
  --region us-east-1 \
  --stack-name gbn-conduit-full-dev
```

---

## Validation Checklist

Minimum smoke sign-off:

- AWS identity is valid in WSL.
- Docker can build and push all three Conduit images.
- CloudFormation stack reaches `CREATE_COMPLETE` or `UPDATE_COMPLETE`.
- ECS authority service is `desired=1`, `running=1`.
- ECS receiver service is `desired=1`, `running=1`.
- ECS bridge service is `desired=1`, `running=1`.
- Authority logs show service startup.
- Bridge logs show lease registration or renewal.
- Receiver logs are available and queryable.
- Smoke artifact directory is preserved.

Full Phase 10 sign-off:

- All minimum smoke checks pass.
- Real mobile network path is documented.
- Bootstrap succeeds from the mobile path.
- Upload / ACK path succeeds from the mobile path.
- Failover or churn scenario is executed and timed.
- Batch-window behavior is measured.
- A specific `chain_id` appears in authority, receiver, bridge, and validation artifacts.
- The Phase 10 test report is updated with evidence and anomalies.

Do not mark Phase 10 complete based only on a stack deployment.

---

## Troubleshooting

### CloudFormation stack rolls back

Pull the first failing event before changing anything:

```bash
aws cloudformation describe-stack-events \
  --region us-east-1 \
  --stack-name gbn-conduit-full-dev \
  --query 'StackEvents[0:20].[Timestamp,LogicalResourceId,ResourceStatus,ResourceStatusReason]' \
  --output table
```

Common causes:

- ECS service did not stabilize.
- image URI is wrong or image is missing.
- secret ARN is wrong.
- service discovery registry is invalid.
- task cannot connect to RDS.
- task exits because required env vars are missing.

### ECS service is below desired count

Check service and task state:

```bash
aws ecs describe-services \
  --region us-east-1 \
  --cluster CLUSTER_NAME \
  --services SERVICE_NAME \
  --output json
```

Check stopped task reasons:

```bash
aws ecs list-tasks \
  --region us-east-1 \
  --cluster CLUSTER_NAME \
  --service-name SERVICE_NAME \
  --desired-status STOPPED
```

### RDS TLS fails in a smoke stack

If logs show certificate trust errors, either:

- provide the RDS CA bundle with `GBN_BRIDGE_POSTGRES_TLS_CA_PEM` or `GBN_BRIDGE_POSTGRES_TLS_CA_FILE`
- or, for smoke-only development validation, redeploy with `--postgres-tls-accept-invalid-certs true`

Do not carry the invalid-certificate setting into production validation.

### ECS metadata parsing fails

The bridge uses the ECS metadata endpoint to discover its task network identity. If metadata parsing fails, check bridge logs and confirm the task has the expected ECS metadata URI environment variable.

### WSL appears unresponsive

This host may run heavy WSL workloads. Prefer longer command timeouts before assuming Docker or AWS tooling is unavailable.

---

## V1 Preservation

Do not modify or call V1 deployment files from this directory. In particular, Conduit deployment work must not edit:

- `prototype/gbn-proto/infra/cloudformation/**`
- `prototype/gbn-proto/infra/scripts/**`
- `prototype/gbn-proto/Dockerfile.relay`
- `prototype/gbn-proto/Dockerfile.publisher`
- `docs/prototyping/Lattice/**`
- frozen V1 architecture docs

V1 regression commands, when required:

```bash
cd prototype/gbn-proto
cargo check --workspace
cargo test -p mcn-router-sim
```

---

## Contributing

For Conduit infrastructure changes:

- keep changes scoped to `prototype/gbn-bridge-proto/**` and the Conduit planning docs
- do not edit root `README.md` during implementation phases
- include validation commands and artifact paths in the commit message or follow-up notes
- prefer minimal smoke deployments before scaled test deployments
- label smoke-only deviations explicitly
- preserve raw artifacts when a failure occurs

---

## License

See the repository root [LICENSE](../../../LICENSE).
