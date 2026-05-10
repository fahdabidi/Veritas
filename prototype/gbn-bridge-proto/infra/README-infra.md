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
- Pass 1 full implementation plan: [GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md](../../../docs/prototyping/Conduit/Full-Implementation-Plan/GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md)
- Pass 2 V2-to-V1 parity plan: [GBN-PROTO-007-Conduit-V2-V1-Parity-Execution-Plan.md](../../../docs/prototyping/Conduit/Full-Implementation-Plan-Pass2/GBN-PROTO-007-Conduit-V2-V1-Parity-Execution-Plan.md)
- Pass 2 local Kubernetes plan: [GBN-PROTO-008-Local-Kubernetes-Test-Infrastructure-Execution-Plan.md](../../../docs/prototyping/Conduit/Full-Implementation-Plan-Pass2/GBN-PROTO-008-Local-Kubernetes-Test-Infrastructure-Execution-Plan.md)
- Pass 3 architecture-correct bootstrap plan: [GBN-PROTO-012-Conduit-Architecture-Correct-Bootstrap-Execution-Plan.md](../../../docs/prototyping/Conduit/Full-Implementation-Plan-Pass3/GBN-PROTO-012-Conduit-Architecture-Correct-Bootstrap-Execution-Plan.md)
- Smoke 2 discovery report: [GBN-PROTO-012-Smoke-2-Discovery-20260510-002230-7843.md](../../../docs/prototyping/Conduit/Full-Implementation-Plan-Pass3/Test-Reports/GBN-PROTO-012-Smoke-2-Discovery-20260510-002230-7843.md)
- AWS Phase 10 validation plan: [GBN-PROTO-006-Execution-Phase10-Live-AWS-And-Mobile-Validation.md](../../../docs/prototyping/Conduit/Full-Implementation-Plan/GBN-PROTO-006-Execution-Phase10-Live-AWS-And-Mobile-Validation.md)
- AWS test report: [GBN-PROTO-006-Conduit-Full-Implementation-Test-Report.md](../../../docs/prototyping/Conduit/Full-Implementation-Plan/GBN-PROTO-006-Conduit-Full-Implementation-Test-Report.md)

> "Truth needs infrastructure" becomes operational here: real service boundaries, durable authority state, deployment images, AWS smoke validation, and end-to-end `chain_id` evidence.

---

## Project Status

This directory contains the V2-only deployment assets for the Conduit track. These files are intentionally isolated from the frozen V1 Lattice workspace under `prototype/gbn-proto/`.

Current state:

- Pass 2 is complete locally: admin HTTP endpoints, admin command injection, metrics surfaces, the creator capability library, and interactive operator parity are implemented.
- Pass 2 local Kubernetes is complete: k3d bring-up, Postgres, Conduit manifests, Prometheus/Grafana/Loki/Tempo, Prometheus metrics, and a kubectl-based operator script are implemented.
- Pass 3 is complete locally: the architecture-correct first-time creator bootup path, Publisher-seeded bridge DHT, HostCreator/NewCreator seed commands, local creator DHT, local-DHT `SendDummy`, upload session build, per-chunk encryption, multi-lane progressive fanout, and Smoke 1 through Smoke 4 scripts are implemented.
- The local k3d topology now has 14 Conduit actor pods: `publisher-authority`, `publisher-receiver`, ten `exit-bridge-*` pods, `creator-host`, and `creator-new`, plus Postgres and optional observability pods.
- `k8s-up.sh` builds versioned local image tags, imports them into k3d, patches deployments to the exact run tag, restarts workloads, and waits for validated readiness so repeated smoke runs do not accidentally use stale images.
- Smoke 2 has a saved detailed run report with real Publisher DHT entries, NewCreator local DHT state, API gate results, bootstrap session state, and ChainID pod-log evidence.
- AWS ECS/Fargate remains the production-shaped deployment surface, using Cloud Map service discovery, RDS Postgres, Secrets Manager, and CloudWatch Logs.
- Phase 10 minimal AWS smoke validation previously passed against `gbn-conduit-full-dev` in `us-east-1`; full mobile-carrier validation is still pending before production readiness.
- Root `README.md` remains release-facing and should not be edited for Conduit implementation work until the V2 release is ready.

Current saved evidence:

| Evidence | Value |
|---|---|
| Pass 3 Smoke 2 report | [GBN-PROTO-012-Smoke-2-Discovery-20260510-002230-7843.md](../../../docs/prototyping/Conduit/Full-Implementation-Plan-Pass3/Test-Reports/GBN-PROTO-012-Smoke-2-Discovery-20260510-002230-7843.md) |
| Smoke 2 ChainID | `smoke-2-b927fae092c0455d980beb21fc8e158e` |
| Smoke 2 bootstrap session | `bootstrap-000010` |
| Smoke 2 result | Publisher DHT `10/10`, NewCreator local DHT `10/10 active`, bootstrap session `completed`, ChainID pod-log evidence present |
| Prior AWS stack | `gbn-conduit-full-dev` |
| Prior AWS region | `us-east-1` |
| Prior AWS image tag | `proto006-phase10-fix2` |

Remaining validation gap:

- Run and archive detailed reports for Smoke 1, Smoke 3, and Smoke 4 using the same evidence standard as Smoke 2.
- Run a real mobile-network path against a deployed `gbn-conduit-full-*` stack.
- Capture explicit AWS `chain_id` evidence with `collect-conduit-traces.sh --chain-id <id> --require-chain-id`.
- Record AWS/mobile bootstrap, upload/ACK, failover/churn, and batch-window observations in the Phase 10 test report.

---

## Quick Start

### Prerequisites

Run local Kubernetes and smoke commands from WSL2 Ubuntu on this host. AWS credentials are only required for the ECS/Fargate deployment path.

- Rust toolchain
- Docker with BuildKit support
- `python3`
- `bash`
- `kubectl`, `k3d`, and `helm` for the local Kubernetes path
- AWS CLI v2 authenticated to the target account for the AWS path
- AWS-only: existing VPC and at least two service subnets
- AWS-only: existing database subnets for RDS
- AWS-only: Secrets Manager values for publisher and bridge signing material

Bootstrap missing local Kubernetes tooling from WSL:

```bash
cd prototype/gbn-bridge-proto
infra/scripts/bootstrap-k8s.sh
```

Confirm Docker is responsive:

```bash
docker ps
```

Confirm the active AWS identity only when using the AWS path:

```bash
aws sts get-caller-identity
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

### 3) Bring up the current local Kubernetes topology

```bash
cd prototype/gbn-bridge-proto
infra/scripts/k8s-up.sh
```

This creates the k3d cluster, builds versioned local images, imports the exact run tag,
applies the Conduit manifests, restarts all deployments, waits for stable workloads,
and runs the default local smoke checks.

### 4) Install local observability

```bash
cd prototype/gbn-bridge-proto
infra/scripts/k8s-observability-up.sh
```

Grafana is exposed at `http://localhost:30030` with local-only credentials
`admin/admin`.

### 5) Run the Pass 3 local acceptance suite

```bash
cd prototype/gbn-bridge-proto
infra/scripts/k8s-pass3-acceptance.sh --require-observability
```

For functional-only runs when the observability namespace is intentionally absent:

```bash
cd prototype/gbn-bridge-proto
infra/scripts/k8s-pass3-acceptance.sh --no-require-observability
```

The acceptance runner executes, in order:

1. `k8s-smoke-tracing-v3.sh`
2. `k8s-smoke-discovery-v3.sh`
3. `k8s-smoke-route-v3.sh`
4. `k8s-smoke-upload-v3.sh`

### 6) Build and push Conduit images for AWS

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
- `gbn-conduit-full-creator`

### 7) Deploy an AWS smoke stack

Use `DesiredBridgeCount=10` for Pass 3 architecture-correct validation so the Publisher
can seed the full 10-entry ExitBridge DHT set. Use `DesiredBridgeCount=1` only for
legacy minimal ECS smoke checks that do not exercise Pass 3 bootstrapping.

```bash
cd prototype/gbn-bridge-proto
infra/scripts/deploy-conduit-full.sh \
  --region us-east-1 \
  --stack-name gbn-conduit-full-dev \
  --environment dev \
  --desired-bridge-count 10 \
  --vpc-id vpc-REPLACE_ME \
  --service-subnet-ids subnet-REPLACE_ME_A,subnet-REPLACE_ME_B \
  --database-subnet-ids subnet-REPLACE_ME_C,subnet-REPLACE_ME_D \
  --authority-image ACCOUNT_ID.dkr.ecr.us-east-1.amazonaws.com/gbn-conduit-full-authority:proto006-phase10-fix2 \
  --receiver-image ACCOUNT_ID.dkr.ecr.us-east-1.amazonaws.com/gbn-conduit-full-receiver:proto006-phase10-fix2 \
  --bridge-image ACCOUNT_ID.dkr.ecr.us-east-1.amazonaws.com/gbn-conduit-full-bridge:proto006-phase10-fix2 \
  --creator-image ACCOUNT_ID.dkr.ecr.us-east-1.amazonaws.com/gbn-conduit-full-creator:proto006-phase10-fix2 \
  --publisher-signing-key-secret-arn arn:aws:secretsmanager:us-east-1:ACCOUNT_ID:secret:publisher-signing \
  --bridge-signing-seed-secret-arn arn:aws:secretsmanager:us-east-1:ACCOUNT_ID:secret:bridge-signing-seed \
  --publisher-public-key-hex REPLACE_ME
```

Smoke-only TLS escape hatch:

```bash
  --postgres-tls-accept-invalid-certs true
```

Use that flag only for development smoke stacks when the container image cannot validate the RDS CA chain. Production validation should provide the RDS CA bundle through `GBN_BRIDGE_POSTGRES_TLS_CA_PEM` or `GBN_BRIDGE_POSTGRES_TLS_CA_FILE` and keep invalid certificate acceptance disabled.

### 8) Run AWS smoke validation

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
- bridge service reaches configured desired count
- `creator-host` service `desired=1`, `running=1`
- `creator-new` service `desired=1`, `running=1`
- command exits non-zero if any service is below desired running count

### 9) Collect Phase 10 AWS evidence

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

### 10) Tear down when finished

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
| Minimal before scaled | Legacy AWS smoke can still use one bridge, but Pass 3 bootstrap/upload validation requires ten bridges |
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
| HostCreator | `creator-host.conduit-<env>.internal:9090` |
| NewCreator | `creator-new.conduit-<env>.internal:9090` |
| Bridge control URL | `ws://publisher-authority.conduit-<env>.internal:<authority-port>/v1/bridge/control` |

Current ports:

| Port | Default | Purpose |
|---|---:|---|
| Authority HTTP | `8080` | authority API and bridge control |
| Receiver HTTP | `8081` | receiver-facing service |
| Admin HTTP | `9090` | localhost-only admin/operator surface |
| Metrics HTTP | `9100` | local Prometheus scrape surface for bridge and creator pods |
| UDP punch | `443` | signed bridge punch/tunnel port |

---

## ChainID Trace Design

`chain_id` is the root distributed trace identifier carried forward from the V1 implementation. Do not replace it with a competing field name.

Validation expectations:

- Creator-originated bootstrap or upload flow originates or carries one root `chain_id`.
- Creator logs include the `chain_id` for seed, local DHT, route, encryption, and upload events.
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

For local Kubernetes, use `CollectTraces` from `k8s-control-interactive.sh` or rely on
the smoke scripts' artifact directories. Pass 3 smoke reports should preserve pod-log
ChainID evidence even when the optional observability backend is not installed.

---

## Repository Layout

| Path | Purpose |
|---|---|
| `../Cargo.toml` | Conduit Rust workspace |
| `../Dockerfile.bridge` | builds the V2 ExitBridge deployment binary |
| `../Dockerfile.bridge-publisher` | legacy prototype publisher image from the earlier simulation track |
| `../Dockerfile.publisher-authority` | builds the real Conduit publisher-authority service image |
| `../Dockerfile.publisher-receiver` | builds the real Conduit publisher-receiver service image |
| `../Dockerfile.creator-runner` | builds the Pass 3 creator-runner image used by `creator-host` and `creator-new` |
| `../docker-compose.bridge-smoke.yml` | earlier BusyBox smoke-only placeholder topology |
| `../docker-compose.conduit-e2e.yml` | local authority / receiver / bridge / Postgres topology |
| `../docs/mobile-test-matrix.md` | Conduit mobile and AWS validation matrix |
| `cloudformation/phase2-bridge-stack.yaml` | earlier isolated V2 bridge prototype stack |
| `cloudformation/conduit-full-stack.yaml` | full Conduit authority / receiver / bridge / Postgres stack |
| `cloudformation/parameters.json` | example parameter file with placeholders |
| `k8s/conduit/` | local Kubernetes Conduit manifests for Publisher, ten bridges, two creators, and Postgres |
| `k8s/observability/` | local Prometheus, Grafana, Loki, Promtail, and Tempo values/dashboards |
| `scripts/` | build, deploy, smoke, validation, trace, operator, and teardown scripts |

---

## Technical Stack

| Layer | Current Tooling |
|---|---|
| Language | Rust |
| Containers | Docker |
| Local topology | k3d, Kubernetes manifests, Docker Compose, and Rust e2e harness |
| Local observability | Prometheus, Grafana, Loki, Promtail, Tempo |
| AWS compute | AWS ECS/Fargate |
| AWS service discovery | AWS Cloud Map private DNS |
| Database | local Kubernetes Postgres or AWS RDS Postgres |
| Secrets | Kubernetes Secrets locally, AWS Secrets Manager in AWS |
| Logs | pod logs and Loki locally, CloudWatch Logs in AWS |
| Image registry | local k3d image import locally, AWS ECR in AWS |
| Deployment | Kustomize/kubectl locally, AWS CloudFormation in AWS |
| Validation scripts | Bash, kubectl, AWS CLI, Python helper snippets |

---

## Implementation Tracks

Conduit infrastructure has three implementation passes in this repository.

| Track | Status | Infra Relevance |
|---|---|---|
| GBN-PROTO-006 Pass 1 | implemented; AWS/mobile acceptance still pending | real Publisher authority, receiver, bridge control, durable Postgres, AWS images/control plane, distributed e2e harness |
| GBN-PROTO-007 Pass 2 | complete locally | V1 operator parity for V2: read-only admin endpoints, command injection, metrics, creator library, `relay-control-interactive-v2.sh` |
| GBN-PROTO-008 Pass 2 local k8s | complete locally | k3d cluster, Kubernetes manifests, local Postgres, Prometheus/Grafana/Loki/Tempo, kubectl operator panel |
| GBN-PROTO-012 Pass 3 | complete locally | architecture-correct HostCreator/NewCreator bootup, Publisher-seeded 10-bridge DHT, local creator DHT, local-DHT routing, upload session build, per-chunk encryption, multi-lane progressive fanout, Smoke 1-4 scripts |

Pass 3 phases:

| Phase | Title | Status |
|---|---|---|
| 0 | Creator Pod Deployment And Cluster Topology | complete |
| 1 | Creator Local State And DHT Metadata Model | complete |
| 2 | SeedHostCreator Admin API And Operator Command | complete |
| 3 | SeedNewCreator API And First-Contact Join Path | complete |
| 4 | Bootstrap Payload Delivery, Local DHT Population, And Punch Fanout | complete |
| 5 | Onboarded-Creator SendDummy And Local-DHT Single-Lane Envelope Demo | complete |
| 6 | Operator Scripts And Acceptance Gate | complete |
| 7 | Smoke 1 - Tracing Suite Implementation | complete |
| 8 | Smoke 2 - Discovery / Bootup Suite Implementation | complete |
| 9 | Smoke 3 - Route And Encryption Boundary Suite Implementation | complete |
| 10 | Upload Session Build And Per-Chunk Encryption Pipeline | complete |
| 11 | Multi-Lane Progressive Fanout | complete |
| 12 | Smoke 4 - Full Upload Pipeline Suite Implementation | complete |

The active evidence gap is no longer "does the local topology exist"; it is archiving repeatable reports for every smoke gate and then repeating the same traceability standard against AWS/mobile.

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
- Legacy one-bridge AWS smoke does not prove multi-bridge fanout or churn behavior; use the Pass 3 10-bridge topology for that.
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
| [GBN-PROTO-007 Pass 2 Plan](../../../docs/prototyping/Conduit/Full-Implementation-Plan-Pass2/GBN-PROTO-007-Conduit-V2-V1-Parity-Execution-Plan.md) | V2-to-V1 operator parity execution plan |
| [GBN-PROTO-008 Local k8s Plan](../../../docs/prototyping/Conduit/Full-Implementation-Plan-Pass2/GBN-PROTO-008-Local-Kubernetes-Test-Infrastructure-Execution-Plan.md) | local Kubernetes infrastructure and observability plan |
| [GBN-PROTO-012 Pass 3 Plan](../../../docs/prototyping/Conduit/Full-Implementation-Plan-Pass3/GBN-PROTO-012-Conduit-Architecture-Correct-Bootstrap-Execution-Plan.md) | architecture-correct creator bootup and upload pipeline plan |
| [GBN-PROTO-012 Smoke 1](../../../docs/prototyping/Conduit/Full-Implementation-Plan-Pass3/GBN-PROTO-012-Smoke-1-Tracing.md) | distributed tracing/logging smoke plan |
| [GBN-PROTO-012 Smoke 2](../../../docs/prototyping/Conduit/Full-Implementation-Plan-Pass3/GBN-PROTO-012-Smoke-2-Discovery.md) | discovery and first-time bootup smoke plan |
| [GBN-PROTO-012 Smoke 3](../../../docs/prototyping/Conduit/Full-Implementation-Plan-Pass3/GBN-PROTO-012-Smoke-3-Route.md) | local-DHT route and encryption boundary smoke plan |
| [GBN-PROTO-012 Smoke 4](../../../docs/prototyping/Conduit/Full-Implementation-Plan-Pass3/GBN-PROTO-012-Smoke-4-Full-Upload.md) | full upload pipeline smoke plan |
| [Smoke 2 Report](../../../docs/prototyping/Conduit/Full-Implementation-Plan-Pass3/Test-Reports/GBN-PROTO-012-Smoke-2-Discovery-20260510-002230-7843.md) | saved local-k8s discovery run with DHT, API, bootstrap, and ChainID evidence |
| [Phase 10 Plan](../../../docs/prototyping/Conduit/Full-Implementation-Plan/GBN-PROTO-006-Execution-Phase10-Live-AWS-And-Mobile-Validation.md) | live AWS/mobile validation plan |
| [Full Implementation Test Report](../../../docs/prototyping/Conduit/Full-Implementation-Plan/GBN-PROTO-006-Conduit-Full-Implementation-Test-Report.md) | canonical validation evidence report |
| [Mobile Test Matrix](../docs/mobile-test-matrix.md) | validation scenarios and thresholds |

---

## Local Kubernetes Test Environment

The local Kubernetes track mirrors the AWS full stack without creating any AWS resources.
It runs in k3d with the Pass 3 architecture-correct topology in the `veritas` namespace:

- `postgres` StatefulSet with a 1 Gi `local-path` PVC
- `publisher-authority` Deployment
- `publisher-receiver` Deployment
- `exit-bridge` Deployment with ten replicas (`exit-bridge-0` through `exit-bridge-9`)
- `creator-host` Deployment
- `creator-new` Deployment

The local manifests live under [`infra/k8s/conduit`](k8s/conduit). They keep the admin
listener bound to `127.0.0.1:9090` inside each pod, matching the AWS isolation rule.
Validation and operator tools reach admin routes with `kubectl exec`.

The deployment validates the V2 architecture model from `GBN-ARCH-001-V2` section 3.3:

1. `creator-host` is seeded with Publisher metadata and one ExitBridgeA entry.
2. The Publisher initializes and owns a signed 10-entry ExitBridge DHT view.
3. `creator-new` is seeded with HostCreator metadata.
4. `creator-new` starts first-contact bootup through HostCreator and ExitBridgeA.
5. The Publisher selects a distinct seed bridge, returns signed bootstrap entries, and records bootstrap progress.
6. `creator-new` stores the returned bridge entries in its own local DHT and marks active tunnels.
7. `SendDummy` and `SendUpload` route from `creator-new` local DHT state rather than from a direct Publisher catalog shortcut.

### Local Prerequisites

Run these from WSL2 Ubuntu with Docker Desktop WSL integration enabled:

```bash
cd prototype/gbn-bridge-proto
infra/scripts/bootstrap-k8s.sh
```

The bootstrap script installs `k3d`, `kubectl`, and `helm` if missing. It is intentionally
Linux/WSL-only.

### Bring Up Local Conduit

```bash
cd prototype/gbn-bridge-proto
infra/scripts/k8s-up.sh
```

The script creates or reuses the `veritas` k3d cluster, builds local images, gives each
build a unique run tag, imports that exact tag into k3d, patches deployments to the same
tag, restarts workloads, waits for all workloads, and runs default local smoke validation.
This avoids the old stale `:dev` image failure mode where k3d/containerd could keep an
older image after a rebuild.

Useful overrides:

| Variable | Default | Purpose |
|---|---|---|
| `VERITAS_K3D_CLUSTER` | `veritas` | k3d cluster name |
| `VERITAS_K8S_NAMESPACE` | `veritas` | Conduit namespace |
| `VERITAS_K3D_AGENTS` | `2` | number of k3d agent nodes |
| `VERITAS_K8S_RUN_SMOKE` | `1` | run local smoke validation after deploy |
| `VERITAS_K8S_RUN_CARGO_PERSISTENCE` | `1` | run Postgres-backed publisher persistence tests through a port-forward |
| `VERITAS_K8S_POSTGRES_LOCAL_PORT` | `15432` | localhost port used for Postgres test port-forward |

The dev overlay generates a local-only Postgres password at
`infra/k8s/conduit/overlays/dev/password.txt`; the file is gitignored.

### Pass 3 Smoke Tests

The current local test suite is implemented as four explicit smoke gates:

| Smoke | Script | What it validates | Report status |
|---|---|---|---|
| Smoke 1 - Tracing | `infra/scripts/k8s-smoke-tracing-v3.sh` | Every actor pod emits ChainID logs/spans and Prometheus samples. | Placeholder: save latest detailed report after next run. |
| Smoke 2 - Discovery / Bootup | `infra/scripts/k8s-smoke-discovery-v3.sh` | Publisher DHT seed, HostCreator/NewCreator bootup, NewCreator local DHT population, bootstrap session state, ChainID evidence. | Saved: [GBN-PROTO-012-Smoke-2-Discovery-20260510-002230-7843.md](../../../docs/prototyping/Conduit/Full-Implementation-Plan-Pass3/Test-Reports/GBN-PROTO-012-Smoke-2-Discovery-20260510-002230-7843.md). |
| Smoke 3 - Route / Encryption Boundary | `infra/scripts/k8s-smoke-route-v3.sh` | Complete NewCreator local-DHT preflight, Publisher/creator/ExitBridge DHT evidence, `SendDummy` route source `local_dht`, Publisher decrypt/hash validation, bridge ciphertext-only boundary, ChainID evidence, failover. | Placeholder: archive next `report.md` as `docs/prototyping/Conduit/Full-Implementation-Plan-Pass3/Test-Reports/GBN-PROTO-012-Smoke-3-Route-<run-id>.md`. |
| Smoke 4 - Full Upload | `infra/scripts/k8s-smoke-upload-v3.sh` | Build upload session, Publisher/creator/ExitBridge DHT evidence, sanitize/chunk/encrypt, 10/10 ExitBridge normal fanout, receiver reconstruction, ChainID evidence across every lane, failover, persistence. | Placeholder: archive next `report.md` as `docs/prototyping/Conduit/Full-Implementation-Plan-Pass3/Test-Reports/GBN-PROTO-012-Smoke-4-Upload-<run-id>.md`. |

Run all four gates in dependency order:

```bash
cd prototype/gbn-bridge-proto
infra/scripts/k8s-pass3-acceptance.sh --require-observability
```

If the observability namespace is intentionally absent, use:

```bash
cd prototype/gbn-bridge-proto
infra/scripts/k8s-pass3-acceptance.sh --no-require-observability
```

Functional smoke reports should include three evidence classes:

- DHT dumps from the relevant node admin API (`DumpPublisherDht`, `DumpNodeDht`, or `DumpLocalDht`). Smoke 3 and Smoke 4 require Publisher DHT, NewCreator local DHT, Publisher per-bridge DHT entries, and ExitBridge metadata/local-DHT admin responses to agree on the same expected bridge set before packet transfer begins.
- ChainID evidence from pod logs and, when observability is required, Tempo/Loki. Smoke 3 requires creator, Publisher, and the selected ExitBridge; Smoke 4 requires creator, Publisher, and every ExitBridge lane that carried a chunk.
- API completion entries proving each stage reached its expected terminal state, including Publisher decrypt/hash validation for Smoke 3 and Publisher receiver reconstruction/content-hash validation for Smoke 4.

### Validate Local Conduit

To rerun the older GBN-PROTO-007 parity smoke checks against the local topology:

```bash
cd prototype/gbn-bridge-proto
infra/scripts/k8s-smoke.sh --send-dummy
```

This checks namespace and rollout status, Postgres readiness, public health endpoints,
localhost admin metrics on Conduit pods, bridge registration, frame persistence by
`chain_id`, and recent pod logs containing each generated `chain_id`. Treat the Pass 3
smoke scripts as the authoritative tests for creator onboarding, local-DHT routing,
encryption boundaries, and upload behavior.

To run the host-side publisher persistence test that previously failed with local
Postgres `ConnectionRefused`, use the Kubernetes Postgres port-forward runner:

```bash
cd prototype/gbn-bridge-proto
infra/scripts/k8s-test-publisher-postgres.sh
```

The script opens `kubectl port-forward svc/postgres 15432:5432`, exports the
`GBN_BRIDGE_POSTGRES_*` and `GBN_BRIDGE_TEST_POSTGRES_URL` variables from the Kubernetes Secret, and runs
`cargo test -p gbn-bridge-publisher --test persistence_flow`. Pass cargo arguments after
the script name to run a broader suite against the same database.

### Local Observability

After `k8s-up.sh` succeeds, install the local observability stack:

```bash
cd prototype/gbn-bridge-proto
infra/scripts/k8s-observability-up.sh
```

This installs Prometheus, Grafana, Loki, Promtail, and Tempo into the `observability`
namespace using Helm values under [`infra/k8s/observability`](k8s/observability).

Grafana is exposed at:

```text
http://localhost:30030
```

Default local credentials are `admin/admin`. Do not reuse those credentials outside this
local-only k3d stack.

The `Conduit V2 Overview` dashboard is pre-provisioned under the `Conduit` folder.
GBN-PROTO-008 Phase 3 exposes `/metrics` on authority port `8080`, receiver port `8081`,
and bridge metrics port `9100`. Conduit pods also read `GBN_BRIDGE_OTLP_ENDPOINT` from
the local config map and emit chain-aware spans to Tempo when the observability stack is
installed. Use the dashboard `chain_id` textbox to filter Loki and Tempo panels once
`SendDummy` produces a chain ID.

Useful observability commands:

```bash
kubectl -n observability get pods,svc
kubectl -n observability port-forward svc/kube-prom-prometheus 9090:9090
kubectl -n observability port-forward svc/tempo 3200:3200
```

Remove observability without deleting the Conduit cluster:

```bash
cd prototype/gbn-bridge-proto
infra/scripts/k8s-observability-down.sh
```

### Local Kubernetes Operator Panel

After `k8s-up.sh` and `k8s-observability-up.sh` have completed, drive the running
local cluster from one menu-driven script:

```bash
bash prototype/gbn-bridge-proto/infra/scripts/k8s-control-interactive.sh
```

Override defaults with `VERITAS_K8S_NAMESPACE`, `VERITAS_OBS_NAMESPACE`,
`VERITAS_GRAFANA_URL`, and `VERITAS_K8S_ADMIN_PORT`. The script discovers all running
Conduit pods with a `veritas-role` label and presents Authority, Receiver, and Bridge
pods, plus the creator pods, in one numbered list. Every admin call goes through `kubectl exec -- curl
http://127.0.0.1:9090/...`; no public admin ingress is required.

Menu items:

- `Status`, `DescribePod`, `TailLogs`, `ExecShell`, and `ShowCatalog` are diagnostics.
- `DumpBridges`, `DumpFrames`, and `AdminMetrics` call the Phase 1 admin endpoints.
- `InitializePublisherDht` seeds/rebuilds the Publisher-owned 10-bridge DHT from active ExitBridge registrations.
- `DumpPublisherDht` dumps the Publisher DHT table.
- `DumpNodeDht` dumps DHT/discovery state for a selected node, including ExitBridge and Creator nodes.
- `DumpLocalDht` dumps the selected creator's local discovery table.
- `SeedHostCreator` prepares `creator-host` with Publisher and ExitBridgeA metadata.
- `SeedNewCreator` starts first-contact bootup on `creator-new` through HostCreator.
- `ResetCreatorState` clears creator-local state for repeatable bootstrap tests.
- `LiveMetrics` prints Grafana and Prometheus access URLs for the Phase 2 stack.
- `SendDummy` requires an onboarded NewCreator with Publisher encryption metadata in local DHT and sends a Publisher-encrypted dummy frame through a local-DHT-selected bridge.
- `BuildUploadSession` builds sanitized, chunked, Publisher-encrypted upload session state.
- `SendUpload` dispatches a built upload session through multi-lane progressive fanout; Smoke 4 requires the normal run to use all 10 local ExitBridges.
- `CollectTraces` collects ChainID-scoped pod-log and observability evidence.
- `TriggerCommand` queues a bridge control command through the authority admin API.
- `CheckImages`, `SmokeValidation`, `Refresh`, and `Teardown` support local iteration.

### Tear Down Local Conduit

```bash
cd prototype/gbn-bridge-proto
infra/scripts/k8s-down.sh
```

Set `VERITAS_K8S_ASSUME_YES=1` for non-interactive cleanup.

---

## AWS Test Setup And Scripts

### Naming Rules

| Surface | Convention | Example |
|---|---|---|
| Environment variables | `GBN_BRIDGE_` | `GBN_BRIDGE_PUBLISHER_URL` |
| Container images | `gbn-conduit-full-` | `gbn-conduit-full-authority` |
| CloudFormation stacks | `gbn-conduit-full-` | `gbn-conduit-full-dev` |
| Metrics namespace | `Veritas/Conduit` | `Veritas/Conduit` |
| Artifact directories | explicit `/tmp/veritas-*` path | `/tmp/veritas-proto006-phase10-aws-artifacts` |

### Important Scripts

| Script | Purpose |
|---|---|
| `scripts/build-and-push-conduit-full.sh` | builds and pushes authority, receiver, and bridge images |
| `scripts/deploy-conduit-full.sh` | deploys the full Conduit CloudFormation stack |
| `scripts/smoke-conduit-full.sh` | verifies stack outputs and ECS running counts |
| `scripts/mobile-validation-full.sh` | runs local or AWS Phase 10 validation workflow |
| `scripts/collect-conduit-traces.sh` | collects CloudFormation, ECS, and CloudWatch `chain_id` evidence |
| `scripts/relay-control-interactive-v2.sh` | interactive ECS-only operator control panel |
| `scripts/_seed_actions.sh` | shared Pass 3 operator action library used by AWS and local control scripts |
| `scripts/bootstrap-k8s.sh` | installs local k3d, kubectl, and helm tooling |
| `scripts/k8s-up.sh` | creates local k3d Conduit topology, builds/imports exact versioned images, restarts workloads, and runs local smoke validation |
| `scripts/k8s-smoke.sh` | validates local Postgres, health/admin endpoints, bridge registration, and baseline parity checks |
| `scripts/k8s-pass3-acceptance.sh` | runs Smoke 1 through Smoke 4 in dependency order |
| `scripts/k8s-smoke-tracing-v3.sh` | Smoke 1: validates ChainID logging/tracing/metrics for every actor pod |
| `scripts/k8s-smoke-discovery-v3.sh` | Smoke 2: validates Publisher DHT seeding and HostCreator/NewCreator bootup |
| `scripts/k8s-smoke-route-v3.sh` | Smoke 3: validates local-DHT SendDummy, encryption boundary, and failover |
| `scripts/k8s-smoke-upload-v3.sh` | Smoke 4: validates upload session build, per-chunk encryption, 10/10 ExitBridge normal fanout, reconstruction, failover, and persistence |
| `scripts/k8s-test-publisher-postgres.sh` | port-forwards local k8s Postgres and runs publisher persistence tests |
| `scripts/k8s-observability-up.sh` | installs Prometheus, Grafana, Loki, Promtail, and Tempo locally |
| `scripts/k8s-observability-down.sh` | removes the local observability namespace and Helm releases |
| `scripts/k8s-control-interactive.sh` | interactive kubectl-only local operator control panel |
| `scripts/k8s-down.sh` | deletes the local k3d Conduit cluster |
| `scripts/teardown-conduit-full.sh` | deletes only `gbn-conduit-full-*` stacks |
| `scripts/run-conduit-e2e.sh` | runs the distributed local e2e harness |
| `scripts/status-snapshot.sh` | legacy prototype stack status helper |
| `scripts/build-and-push.sh` | legacy prototype image build helper |
| `scripts/deploy-bridge-test.sh` | legacy prototype stack deploy helper |
| `scripts/teardown-bridge-test.sh` | deletes only legacy `gbn-bridge-phase2-*` stacks |

### Interactive Control Panel

The interactive operator panel is at
[`infra/scripts/relay-control-interactive-v2.sh`](scripts/relay-control-interactive-v2.sh).
Run it with:

```bash
bash prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh
```

Override stack name or region with `GBN_BRIDGE_STACK_NAME` and
`GBN_BRIDGE_AWS_REGION`. `LiveMetrics` derives the CloudWatch `Stack` dimension from
the stack's `EnvironmentName` parameter; override it with
`GBN_BRIDGE_METRICS_STACK_DIMENSION` if needed.

The panel discovers all running ECS tasks for the selected stack and presents a numbered
menu. Admin actions use `aws ecs execute-command --interactive` against each task's
localhost admin port, `127.0.0.1:9090`; no public admin ingress is required.

Menu items:

- `Status`, `StackOutputs`, `TailLogs`, `ExecShell`, `ShowCatalog`: diagnostics.
- `DumpBridges`, `DumpFrames`, `AdminMetrics`: localhost admin endpoints.
- `LiveMetrics`: CloudWatch dashboard for namespace `Veritas/Conduit`.
- `InitializePublisherDht`, `DumpPublisherDht`, `DumpNodeDht`, `SeedHostCreator`, `SeedNewCreator`, `DumpLocalDht`, and `ResetCreatorState`: Pass 3 bootstrap and DHT workflows.
- `SendDummy`: run from an onboarded NewCreator and trace the returned `chain_id`.
- `BuildUploadSession` and `SendUpload`: build and send the full Pass 3 upload pipeline.
- `CollectTraces`: collect ChainID-scoped CloudWatch evidence.
- `TriggerCommand`: push a bridge control payload through the authority admin endpoint.
- `CheckImages`: compare each task's running image digest with ECR `latest`.
- `BootstrapSmoke`, `Refresh`, `Teardown`, `Exit`: operational workflows.

### What The Full Stack Creates

`cloudformation/conduit-full-stack.yaml` creates:

- ECS cluster
- Fargate service for `publisher-authority`
- Fargate service for `publisher-receiver`
- Fargate service for `exit-bridge`
- Fargate service for `creator-host`
- Fargate service for `creator-new`
- Cloud Map private DNS namespace
- RDS Postgres instance
- EFS file system and access points for creator-local state
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
| creator image URI | deployed creator-runner binary |
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

Creator:

- `GBN_BRIDGE_NODE_ID`
- `GBN_BRIDGE_ADMIN_BIND_ADDR`
- `GBN_CONDUIT_ACTOR`
- `GBN_BRIDGE_AUTHORITY_URL`
- `GBN_BRIDGE_PUBLISHER_PUBLIC_KEY_HEX`
- `GBN_BRIDGE_STATE_DIR`
- `GBN_BRIDGE_STACK_ENV`

---

## Typical Validation Workflow

Use the local Pass 3 sequence for active implementation validation. Use the AWS sequence
when validating deployed infrastructure or mobile-carrier behavior.

### Local Pass 3 sequence

```bash
cd prototype/gbn-bridge-proto
infra/scripts/k8s-up.sh
infra/scripts/k8s-observability-up.sh
infra/scripts/k8s-pass3-acceptance.sh --require-observability
```

After each smoke run, preserve the artifact directory printed by the script for local
debugging. Report-producing smoke scripts archive the final `report.md` into the
tracked Pass 3 test-report folder by default. Override the tracked report location
with `VERITAS_K8S_SMOKE_REPORT_ROOT` when needed.
The current saved detailed report is:

- Smoke 2: [GBN-PROTO-012-Smoke-2-Discovery-20260510-002230-7843.md](../../../docs/prototyping/Conduit/Full-Implementation-Plan-Pass3/Test-Reports/GBN-PROTO-012-Smoke-2-Discovery-20260510-002230-7843.md)

Report placeholders to fill after the remaining runs:

- Smoke 1: `docs/prototyping/Conduit/Full-Implementation-Plan-Pass3/Test-Reports/GBN-PROTO-012-Smoke-1-Tracing-<run-id>.md`
- Smoke 3: `docs/prototyping/Conduit/Full-Implementation-Plan-Pass3/Test-Reports/GBN-PROTO-012-Smoke-3-Route-<run-id>.md`
- Smoke 4: `docs/prototyping/Conduit/Full-Implementation-Plan-Pass3/Test-Reports/GBN-PROTO-012-Smoke-4-Upload-<run-id>.md`

Each report should include DHT state, API completion evidence, and ChainID log/span
evidence for every stage it claims as complete.

### AWS Phase 10 sequence

#### 1) Confirm local and AWS preflight

```bash
aws sts get-caller-identity
docker ps
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
```

#### 2) Run local e2e

```bash
cd prototype/gbn-bridge-proto
infra/scripts/mobile-validation-full.sh \
  --mode local \
  --target-dir /tmp/veritas-proto006-phase10-local-target \
  --artifact-dir /tmp/veritas-proto006-phase10-local-artifacts
```

#### 3) Build images

```bash
cd prototype/gbn-bridge-proto
infra/scripts/build-and-push-conduit-full.sh \
  --region us-east-1 \
  --tag proto006-phase10-validation
```

#### 4) Deploy Pass 3 smoke topology

```bash
cd prototype/gbn-bridge-proto
infra/scripts/deploy-conduit-full.sh \
  --region us-east-1 \
  --stack-name gbn-conduit-full-dev \
  --environment dev \
  --desired-bridge-count 10 \
  --vpc-id vpc-REPLACE_ME \
  --service-subnet-ids subnet-REPLACE_ME_A,subnet-REPLACE_ME_B \
  --database-subnet-ids subnet-REPLACE_ME_C,subnet-REPLACE_ME_D \
  --authority-image ACCOUNT_ID.dkr.ecr.us-east-1.amazonaws.com/gbn-conduit-full-authority:proto006-phase10-validation \
  --receiver-image ACCOUNT_ID.dkr.ecr.us-east-1.amazonaws.com/gbn-conduit-full-receiver:proto006-phase10-validation \
  --bridge-image ACCOUNT_ID.dkr.ecr.us-east-1.amazonaws.com/gbn-conduit-full-bridge:proto006-phase10-validation \
  --creator-image ACCOUNT_ID.dkr.ecr.us-east-1.amazonaws.com/gbn-conduit-full-creator:proto006-phase10-validation \
  --publisher-signing-key-secret-arn arn:aws:secretsmanager:us-east-1:ACCOUNT_ID:secret:publisher-signing \
  --bridge-signing-seed-secret-arn arn:aws:secretsmanager:us-east-1:ACCOUNT_ID:secret:bridge-signing-seed \
  --publisher-public-key-hex REPLACE_ME
```

#### 5) Run smoke

```bash
cd prototype/gbn-bridge-proto
infra/scripts/smoke-conduit-full.sh \
  --region us-east-1 \
  --stack-name gbn-conduit-full-dev
```

#### 6) Run AWS evidence collection

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

#### 7) Run final mobile-chain trace capture

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

#### 8) Update the test report

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

#### 9) Confirm V1 preservation

From the repo root:

```bash
git diff --name-only -- \
  prototype/gbn-proto \
  docs/prototyping/Lattice \
  docs/architecture/GBN-PROTO-004-Phase2-Serverless-Scale-Onion-Plan.md \
  docs/prototyping/Lattice/GBN-PROTO-004-Phase2-Serverless-Scale-Onion-Plan.md
```

Expected result: no output.

#### 10) Tear down if the stack is no longer needed

```bash
cd prototype/gbn-bridge-proto
infra/scripts/teardown-conduit-full.sh \
  --region us-east-1 \
  --stack-name gbn-conduit-full-dev
```

---

## Validation Checklist

Local Pass 3 smoke sign-off:

- `k8s-up.sh` completes with the exact image tag imported into k3d and deployed.
- `kubectl -n veritas get pods` shows `publisher-authority`, `publisher-receiver`, ten `exit-bridge-*` pods, `creator-host`, `creator-new`, and Postgres Ready.
- `k8s-smoke-tracing-v3.sh` passes and records ChainID logs/spans/metrics for every actor pod.
- `k8s-smoke-discovery-v3.sh` passes and validates:
  - Publisher DHT contains ten signed active ExitBridge entries.
  - `SeedHostCreator`, `InitializePublisherDht`, and `SeedNewCreator` APIs complete.
  - `creator-new` local DHT contains ten active bridge entries.
  - Publisher bootstrap session reaches `completed`.
  - ChainID pod-log evidence exists for the bootstrap stages.
- `k8s-smoke-route-v3.sh` passes and validates complete Publisher/NewCreator/ExitBridge DHT evidence, local-DHT `SendDummy`, Publisher decrypt/hash validation, ciphertext-only bridge forwarding, receiver persistence, ChainID logs across creator/Publisher/selected bridge, and forced failover.
- `k8s-smoke-upload-v3.sh` passes and validates complete Publisher/NewCreator/ExitBridge DHT evidence, upload session build, sanitization, per-chunk encryption, 10/10 ExitBridge normal fanout, receiver reconstruction, ChainID logs across creator/Publisher/every lane-carrying bridge, failover, and creator PVC persistence.
- A detailed report is saved for each smoke run. Smoke 2 is currently saved at [GBN-PROTO-012-Smoke-2-Discovery-20260510-002230-7843.md](../../../docs/prototyping/Conduit/Full-Implementation-Plan-Pass3/Test-Reports/GBN-PROTO-012-Smoke-2-Discovery-20260510-002230-7843.md); Smoke 1, Smoke 3, and Smoke 4 reports are placeholders until rerun.

AWS smoke sign-off:

- AWS identity is valid in WSL.
- Docker can build and push authority, receiver, bridge, and creator images.
- CloudFormation stack reaches `CREATE_COMPLETE` or `UPDATE_COMPLETE`.
- ECS authority service is `desired=1`, `running=1`.
- ECS receiver service is `desired=1`, `running=1`.
- ECS bridge service reaches the configured desired count.
- ECS `creator-host` and `creator-new` services are `desired=1`, `running=1`.
- Authority logs show service startup.
- Bridge logs show lease registration or renewal.
- Creator logs show admin listener startup and node metadata availability.
- Receiver logs are available and queryable.
- Smoke artifact directory is preserved.

Full Phase 10 sign-off:

- All minimum smoke checks pass.
- Real mobile network path is documented.
- Bootstrap succeeds from the mobile path.
- Upload / ACK path succeeds from the mobile path.
- Failover or churn scenario is executed and timed.
- Batch-window behavior is measured.
- A specific `chain_id` appears in authority, receiver, bridge, creator, and validation artifacts.
- The Phase 10 test report is updated with evidence and anomalies.

Do not mark Phase 10 complete based only on a stack deployment.

---

## Troubleshooting

### Local k3d cluster uses stale images

Run `infra/scripts/k8s-up.sh` instead of manually restarting deployments. The script
builds a unique image tag, imports it into k3d, patches deployments to that exact tag,
and waits for rollout readiness. If a pod still reports old behavior, collect:

```bash
kubectl -n veritas get pods -o wide
kubectl -n veritas describe pod POD_NAME
kubectl -n veritas logs POD_NAME --tail=200
```

### Local bootstrap succeeds but DHT assertions fail

Dump the three state surfaces before retrying:

```bash
bash prototype/gbn-bridge-proto/infra/scripts/k8s-control-interactive.sh
```

Use `DumpPublisherDht`, `DumpNodeDht`, and `DumpLocalDht`. A valid Smoke 2 run must
show the same ten bridge IDs in the Publisher DHT, NewCreator local DHT, active tunnel
set, and Publisher bootstrap session.

### Local ChainID evidence is missing

First decide whether observability is required for the run. With
`--no-require-observability`, the smoke scripts still require pod-log ChainID evidence.
With `--require-observability`, Loki and Tempo must also return the ChainID. Check:

```bash
kubectl -n observability get pods,svc
kubectl -n veritas logs deploy/creator-new --tail=200
kubectl -n veritas logs deploy/publisher-authority --tail=200
```

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
