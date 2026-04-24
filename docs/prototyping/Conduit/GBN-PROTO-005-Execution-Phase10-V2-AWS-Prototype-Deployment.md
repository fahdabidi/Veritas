# GBN-PROTO-005 - Execution Phase 10 Detailed Plan: V2 AWS Prototype Deployment

**Status:** Implemented locally from the committed Phase 9 test-harness baseline; live AWS deployment validation and extended V1 AWS regression remain pending in an AWS/Docker-capable environment
**Primary Goal:** add V2-only AWS prototype deployment assets for Conduit publisher and ExitBridge services without mutating frozen V1 Lattice infrastructure
**Source Plan:** [GBN-PROTO-005 Execution Plan](GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign-Execution-Plan.md)
**Phase 0 Baseline Release:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)
**Protocol Baseline:** [GBN-ARCH-002-Bridge-Protocol-V2](../architecture/GBN-ARCH-002-Bridge-Protocol-V2.md)

---

## 1. Current Repo Findings

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | Phase 10 builds on the committed Conduit test-harness baseline |
| Current HEAD commit | `317d2512899e2b6611269a070c0bb9d8d9105f32` | this is the committed Phase 9 baseline used to begin AWS prototype work |
| Phase 0 baseline release | `veritas-lattice-0.1.0-baseline` published | V1 Lattice remains the infrastructure no-touch reference |
| V2 infra state before Phase 10 | only placeholder infra docs plus the Phase 9 local smoke runner existed | Phase 10 must create real V2 deployment assets from scratch |
| Current runtime boundary | Conduit runtime behavior is still in-process and test-harness driven | AWS assets can deploy prototype processes, but full network bootstrap validation remains pending until network listeners replace placeholders |
| Current Docker environment | Docker / `docker-compose` are unavailable in the local validation environment | image build and compose/AWS smoke validation must be rerun in a Docker-capable environment |
| Current protected V1 path drift | none | V1 deployment assets remain untouched |

---

## 2. Scope Lock

### In Scope

- V2-only Dockerfiles for Publisher Authority and ExitBridge processes
- V2-only CloudFormation stack for ECS/Fargate prototype services
- V2-only build, deploy, status, smoke, interactive-control, and teardown scripts
- deployment entrypoints that validate `GBN_BRIDGE_*` environment wiring and keep ECS tasks alive
- deployment docs that explain current AWS prototype limits

### Out Of Scope

- modifying V1 CloudFormation, V1 Dockerfiles, or V1 scripts
- changing root `README.md`
- claiming production AWS bootstrap success before a real network listener exists
- mobile-network validation
- changing the V2 protocol or runtime semantics beyond deployment entrypoint configuration

---

## 3. Decisions Locked In Phase 10

### 3.1 V2 Naming

| Surface | Locked Value |
|---|---|
| stack prefix | `gbn-bridge-phase2-` |
| publisher image repo | `gbn-bridge-proto-publisher` |
| bridge image repo | `gbn-bridge-proto-exit-bridge` |
| environment prefix | `GBN_BRIDGE_` |
| default UDP punch port | `443` |
| default batch window | `500` ms |

### 3.2 Deployment Model

Phase 10 uses an isolated ECS/Fargate CloudFormation stack. The template creates:

- one V2 publisher service
- one scalable V2 ExitBridge service
- V2-only log groups
- V2-only security groups
- V2-only task definitions
- a V2-only task execution role

### 3.3 Runtime Honesty Boundary

The deployment binaries are prototype entrypoints. They validate configuration and keep tasks alive under ECS, but they do not yet expose a production network protocol service. The AWS smoke script therefore validates stack and task-definition wiring, not a full live first-contact bootstrap path.

---

## 4. Files Created Or Modified

Created:

- `prototype/gbn-bridge-proto/Dockerfile.bridge`
- `prototype/gbn-bridge-proto/Dockerfile.bridge-publisher`
- `prototype/gbn-bridge-proto/infra/cloudformation/phase2-bridge-stack.yaml`
- `prototype/gbn-bridge-proto/infra/cloudformation/parameters.json`
- `prototype/gbn-bridge-proto/infra/scripts/build-and-push.sh`
- `prototype/gbn-bridge-proto/infra/scripts/bootstrap-smoke.sh`
- `prototype/gbn-bridge-proto/infra/scripts/deploy-bridge-test.sh`
- `prototype/gbn-bridge-proto/infra/scripts/status-snapshot.sh`
- `prototype/gbn-bridge-proto/infra/scripts/teardown-bridge-test.sh`
- `prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/lib.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/bin/bridge-publisher.rs`

Modified:

- `prototype/gbn-bridge-proto/infra/README-infra.md`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/main.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/bin/exit-bridge.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/bin/creator-client.rs`
- `prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/bin/host-creator.rs`
- master execution plan status and Phase 10 reference

---

## 5. Validation Commands

Local validation:

```bash
cargo fmt --all --check --manifest-path prototype/gbn-bridge-proto/Cargo.toml
cargo check --workspace --manifest-path prototype/gbn-bridge-proto/Cargo.toml
```

Temp-target V2 test fallback:

```powershell
$target = Join-Path $env:LOCALAPPDATA 'Temp\veritas-bridge-target-phase10'
New-Item -ItemType Directory -Path $target -Force | Out-Null
$env:CARGO_INCREMENTAL='0'
cargo test --workspace --manifest-path prototype/gbn-bridge-proto/Cargo.toml --target-dir $target
```

Script parse / help validation:

```bash
bash prototype/gbn-bridge-proto/infra/scripts/build-and-push.sh --help
bash prototype/gbn-bridge-proto/infra/scripts/deploy-bridge-test.sh --help
bash prototype/gbn-bridge-proto/infra/scripts/bootstrap-smoke.sh --help
bash prototype/gbn-bridge-proto/infra/scripts/status-snapshot.sh --help
bash prototype/gbn-bridge-proto/infra/scripts/teardown-bridge-test.sh --help
```

V1 preservation:

```bash
git diff --name-only -- \
  prototype/gbn-proto \
  docs/prototyping/GBN-PROTO-004-Phase2-Serverless-Scale-Onion-Plan.md \
  docs/prototyping/GBN-PROTO-004-Phase2-Serverless-Scale-Test.md \
  docs/architecture/GBN-ARCH-000-System-Architecture.md \
  docs/architecture/GBN-ARCH-001-Media-Creation-Network.md
```

```bash
cd prototype/gbn-proto
cargo check --workspace
cargo test -p mcn-router-sim
```

AWS validation still required before full Phase 10 sign-off:

```bash
prototype/gbn-bridge-proto/infra/scripts/build-and-push.sh --region <region> --tag phase10
prototype/gbn-bridge-proto/infra/scripts/deploy-bridge-test.sh \
  --region <region> \
  --stack-name gbn-bridge-phase2-<env> \
  --environment <env> \
  --vpc-id <vpc-id> \
  --subnet-ids <subnet-a>,<subnet-b> \
  --publisher-image <publisher-image-uri> \
  --bridge-image <bridge-image-uri>
prototype/gbn-bridge-proto/infra/scripts/bootstrap-smoke.sh --region <region> --stack-name gbn-bridge-phase2-<env>
prototype/gbn-proto/infra/scripts/run-tests.sh <v1-stack-name> <region>
```

Executed local validation result for the current implementation:

- `cargo fmt --all --check --manifest-path prototype/gbn-bridge-proto/Cargo.toml` passed
- `cargo check --workspace --manifest-path prototype/gbn-bridge-proto/Cargo.toml` hit the known OneDrive `target/` write denial, then passed with `--target-dir %LOCALAPPDATA%\Temp\veritas-bridge-target-phase10`
- `cargo test --workspace --manifest-path prototype/gbn-bridge-proto/Cargo.toml --target-dir %LOCALAPPDATA%\Temp\veritas-bridge-target-phase10` passed
- script help checks passed for `build-and-push.sh`, `deploy-bridge-test.sh`, `bootstrap-smoke.sh`, `status-snapshot.sh`, and `teardown-bridge-test.sh`
- `bash -n` syntax checks passed for all new Phase 10 shell scripts, including `relay-control-interactive-v2.sh`
- `prototype/gbn-bridge-proto/infra/cloudformation/parameters.json` parsed as valid JSON
- protected V1 diff remained empty
- `cd prototype/gbn-proto && cargo check --workspace` passed
- `cd prototype/gbn-proto && cargo test -p mcn-router-sim` passed

---

## 6. Acceptance Criteria

Phase 10 implementation is locally complete when:

- all V2-only deployment files exist
- Dockerfiles reference only V2 workspace binaries
- CloudFormation creates only `gbn-bridge-phase2-*` / `gbn-bridge-proto-*` resources
- scripts refuse unsafe non-V2 stack names where destructive action is possible
- scripts use `GBN_BRIDGE_*` environment variables
- V2 workspace validation passes
- protected V1 diff remains empty
- minimum V1 regression suite passes

Full Phase 10 sign-off additionally requires:

- image build and push succeeds in Docker/ECR
- V2 CloudFormation deployment succeeds in AWS
- `bootstrap-smoke.sh` passes against the deployed stack
- extended V1 AWS regression suite passes

---

## 7. Risks And Blockers

| Risk | Current State | Mitigation |
|---|---|---|
| runtime entrypoints are not full network services | true for Phase 10 | document smoke boundary and avoid claiming full live bootstrap |
| Docker unavailable locally | true in current environment | rerun image and smoke validation in Docker-capable environment |
| AWS deployment not executed locally | true in current environment | keep scripts deterministic and require live AWS validation before sign-off |
| accidental V1 infra drift | no drift observed | path-scoped diff remains a hard gate |

Current blocker:

- full AWS Phase 10 sign-off needs Docker, AWS credentials, VPC/subnet inputs, and a live deployment run
