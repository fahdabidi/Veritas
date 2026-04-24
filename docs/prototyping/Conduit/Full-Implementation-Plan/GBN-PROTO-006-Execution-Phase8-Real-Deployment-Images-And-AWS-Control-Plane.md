# GBN-PROTO-006 - Execution Phase 8 Detailed Plan: Real Deployment Images And AWS Control Plane

**Status:** Ready to start after Phase 7 distributed `chain_id` trace propagation is implemented and validated  
**Primary Goal:** replace the current placeholder deployment images and partial Phase 10 prototype AWS scaffolding with a real deployable Conduit topology that runs separate Publisher Authority, Publisher Receiver, and bridge services over durable storage and explicit service wiring, while preserving the service and trace boundaries introduced in Phases 1 through 7  
**Source Plan:** [GBN-PROTO-006 Execution Plan](GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md)  
**Protected V1 Baseline:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)  
**Phase 7 Detailed Plan:** [GBN-PROTO-006-Execution-Phase7-Distributed-ChainID-Trace-Propagation](GBN-PROTO-006-Execution-Phase7-Distributed-ChainID-Trace-Propagation.md)  
**Starting Conduit Baseline:** `2b6d5c5d24e269e96e3fdc820f3f90669607414a`

---

## 1. Current Repo Findings

These findings should drive Phase 8 instead of being rediscovered during implementation:

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | Phase 8 should record the mainline commit used to begin the deployment cutover |
| Current HEAD commit | `2b6d5c5d24e269e96e3fdc820f3f90669607414a` | current committed Conduit baseline still deploys only the prototype topology |
| Current bridge image | [`Dockerfile.bridge`](../../../prototype/gbn-bridge-proto/Dockerfile.bridge) builds only the `exit-bridge` CLI binary | bridge deployment exists, but only for the prototype runtime surface |
| Current publisher image | [`Dockerfile.bridge-publisher`](../../../prototype/gbn-bridge-proto/Dockerfile.bridge-publisher) builds one monolithic `bridge-publisher` binary | there are still no separate authority and receiver deployment images |
| Current local compose topology | [`docker-compose.bridge-smoke.yml`](../../../prototype/gbn-bridge-proto/docker-compose.bridge-smoke.yml) is a BusyBox placeholder stack | proves the current compose surface is smoke-only and not a real distributed topology |
| Current AWS stack | [`phase2-bridge-stack.yaml`](../../../prototype/gbn-bridge-proto/infra/cloudformation/phase2-bridge-stack.yaml) deploys one publisher ECS service and one bridge ECS service family | the current stack still reflects the earlier prototype phase, not a full Conduit topology |
| Current persistent store wiring | the current Phase 10 stack has no database, no migration step, and no secrets-backed DSN/config injection | a production-capable control plane cannot run without durable storage wiring |
| Current service discovery model | the current stack injects one `PublisherEndpoint` string and uses public subnets with public IP assignment | full Conduit needs explicit internal service wiring for authority, receiver, and bridges |
| Current infra guidance | [`README-infra.md`](../../../prototype/gbn-bridge-proto/infra/README-infra.md) explicitly says the current binaries are "prototype entrypoints" and "do not yet provide a production network service" | the repo already documents the current deployment surface as insufficient |
| Current infra scripts | existing scripts are still `build-and-push.sh`, `deploy-bridge-test.sh`, `bootstrap-smoke.sh`, `status-snapshot.sh`, and `teardown-bridge-test.sh` for the Phase 10 prototype stack | these should inform, but not define, the full Conduit deployment surface |
| Current `chain_id` deployment visibility | current images, compose stack, and CloudFormation outputs do not expose a canonical trace-aware service topology | Phase 8 must keep `chain_id` visible in service logs and validation surfaces once the real services are deployed |

---

## 2. Review Summary

Phase 8 is where Conduit must stop shipping "prototype entrypoints" and start shipping actual deployable service images and topology definitions. If this phase is weak, all prior control-plane and receiver work may exist in code, but the system will still not be deployable as a real distributed implementation.

The main gaps the detailed Phase 8 plan must close are:

| Gap | Why It Matters | Resolution For Phase 8 |
|---|---|---|
| one monolithic publisher image still exists | the full implementation requires separate authority and receiver service surfaces | add dedicated `Dockerfile.publisher-authority` and `Dockerfile.publisher-receiver` |
| current compose stack is placeholder-only | there is no local deployable topology that reflects the full Conduit service graph | add `docker-compose.conduit-e2e.yml` with real service images and wiring |
| current AWS stack is still Phase 10 prototype scaffolding | it lacks separate services, durable DB, secrets, and real service discovery | add `conduit-full-stack.yaml` and new deploy/smoke/teardown scripts |
| no storage/secret deployment wiring exists | Phases 1-7 depend on real storage and signing/config material in production | deploy Postgres and secrets/config injection as part of the control plane |
| current scripts target the old prototype stack only | the operational entrypoint surface is still prototype-scoped | add full-stack scripts and keep the old prototype scripts intact |
| no explicit deployment rule exists for `chain_id` visibility | later validation phases will struggle to correlate live runs | ensure service logs and validation artifacts preserve `chain_id` in the deployed topology |

Phase 8 should make the deployment topology real, but it should not yet claim live validation success. That remains Phase 10.

---

## 3. Scope Lock

### In Scope

- add separate publisher-authority and publisher-receiver deployment images
- keep the bridge image, but upgrade it to run the real network service path
- add `docker-compose.conduit-e2e.yml` for local distributed deployment
- add a new full-stack CloudFormation template for Conduit
- wire in durable storage, secrets/config, service discovery, and logging/metrics plumbing
- add new deploy, smoke, and teardown scripts for the full stack
- update V2-only infra documentation
- preserve `chain_id` visibility in service logs and validation outputs

### Out Of Scope

- live AWS/mobile measurement capture
- final distributed fault-injection harness rollout
- modifying `prototype/gbn-proto/**`
- modifying the main repo `README.md`

---

## 4. Preflight Gates

Phase 8 should not begin code edits until all of these are checked:

1. Confirm the Phase 0 inventory deliverables exist.
2. Confirm Phases 1 through 7 are implemented and validated so the service boundaries, receiver path, and `chain_id` model already exist in code.
3. Confirm protected V1 paths are clean in the local worktree.
4. Confirm the deployment split will include distinct authority and receiver services.
5. Confirm durable storage and secret/config injection are part of the stack, not manual afterthoughts.
6. Confirm the old Phase 10 prototype stack will remain isolated and not be silently overwritten in-place.
7. Confirm `README.md` remains out of scope.

If any gate fails, Phase 8 should stop.

Current blocker:

- Phases 1 through 7 are not yet implemented in this full-implementation track, so Phase 8 remains planning-ready only

---

## 5. Deployment Decisions To Lock In Phase 8

### 5.1 Image Split Rule

Phase 8 should produce at least these deployment images:

- `publisher-authority`
- `publisher-receiver`
- `bridge`

Creator and host-creator images should be added only if a distributed e2e or live validation path actually needs containerized entrypoints. Do not create unnecessary images for code paths that remain test-only.

### 5.2 Local Compose Rule

`docker-compose.conduit-e2e.yml` should be a real local topology, not a placeholder:

- authority service
- receiver service
- one or more bridge services
- local durable store
- any required init/migration step
- explicit environment / secret wiring

Do not carry forward the BusyBox placeholder pattern from `docker-compose.bridge-smoke.yml` into the full implementation path.

### 5.3 CloudFormation Topology Rule

The full AWS template should include at minimum:

- authority service
- receiver service
- bridge service(s)
- durable database
- secrets/config storage
- service discovery or stable internal endpoints
- log groups / metrics wiring

Public exposure should be deliberate and minimal. Internal service traffic should not rely on broad public addressing just because the prototype stack did.

### 5.4 Secrets And Config Rule

Phase 8 should make configuration production-shaped:

- DB credentials or DSNs from secrets, not hard-coded env defaults
- signing keys or wrapped key material from a secret source appropriate to the stack
- environment wiring for service endpoints that matches the split service topology

### 5.5 Migration Rule

If the full Conduit stack depends on Phase 2 durable storage, Phase 8 must define how schema creation and migration happen in deployment:

- init task
- bootstrap migration step
- or explicit migration job / script

Do not assume the database is already prepared manually.

### 5.6 ChainID Deployment Rule

Phase 8 must preserve `chain_id` visibility in the deployed topology:

- service logs should preserve structured `chain_id` output
- smoke scripts should not discard trace-bearing logs
- stack outputs and validation guidance must leave a path to collect trace evidence later

### 5.7 Stack Isolation Rule

The full implementation stack should not overwrite the old prototype stack naming and assumptions without intent. A separate full-stack naming convention is safer than silently reusing `phase2-bridge-*` artifacts.

---

## 6. Module And Asset Ownership To Lock In Phase 8

Phase 8 should keep responsibilities split like this:

| Asset | Responsibility |
|---|---|
| `Dockerfile.publisher-authority` | build and run the real authority service |
| `Dockerfile.publisher-receiver` | build and run the real receiver service |
| `Dockerfile.bridge` | build and run the real bridge service path |
| `docker-compose.conduit-e2e.yml` | local full-topology deploy surface |
| `infra/cloudformation/conduit-full-stack.yaml` | real AWS topology for the full implementation |
| `infra/scripts/deploy-conduit-full.sh` | stack creation/update path |
| `infra/scripts/smoke-conduit-full.sh` | deployed topology smoke gate |
| `infra/scripts/teardown-conduit-full.sh` | safe teardown path |
| `infra/README-infra.md` | V2-only operational documentation for the full stack |

Do not let `phase2-bridge-stack.yaml` become the long-term full implementation file through incremental mutation. The repo needs a clear separation between prototype stack and full implementation stack.

---

## 7. Dependency And Implementation Policy

Phase 8 should keep the infra surface explicit and reproducible.

### Recommended Dependencies

- existing Rust workspace build surface for service binaries
- Docker multi-stage builds
- CloudFormation-native resources where practical
- AWS-managed log and secret services already consistent with the current repo approach

### Bias

- prefer explicit service split over monolithic container reuse
- prefer durable store wiring that matches the chosen Phase 2 persistence model
- prefer one clear local compose topology and one clear AWS full-stack topology
- keep `chain_id` visible in service logs and validation outputs

### Avoid In Phase 8

- mutating V1 infra paths
- continuing to rely on placeholder containers in the full stack
- publishing a stack that still depends on manual undocumented bootstrap steps
- blurring prototype and full-stack naming
- drifting into live evidence collection

---

## 8. Evidence Capture Requirements

Phase 8 should collect and preserve these exact data points:

| Evidence | Source | Must Appear In |
|---|---|---|
| starting branch | `git branch --show-current` | phase notes or commit message |
| starting commit SHA | `git rev-parse HEAD` | phase notes or commit message |
| Phase 1-7 prerequisite status | implementation and validation records | phase notes |
| pre-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |
| image split evidence | Dockerfiles and build outputs | phase notes |
| local compose topology evidence | compose file and config/render output | phase notes |
| AWS full-stack topology evidence | CloudFormation template and validation output | phase notes |
| storage / secrets wiring evidence | template resources and script inputs | phase notes |
| smoke-script evidence | script usage and output samples | phase notes |
| `chain_id` deployment visibility evidence | service log format or smoke output samples | phase notes |
| validation command set used | local command log | phase notes |
| post-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |

Do not sign off Phase 8 with only "images build." Record the actual service split, stateful dependencies, and topology wiring.

---

## 9. Recommended Execution Order

Implement Phase 8 in this order:

1. Capture the starting branch, commit SHA, and protected-path diff state.
2. Add the new deployment Dockerfiles for authority and receiver first.
3. Define the full local compose topology.
4. Define the full AWS stack template with DB, secrets, and service wiring.
5. Add deploy, smoke, and teardown scripts for the full stack.
6. Update V2 infra documentation.
7. Validate image builds, compose config, and CloudFormation structure.
8. Run the required V1 and V2 preservation checks.

This order locks the actual service surfaces before the deployment scripts and docs depend on them.

---

## 10. Validation Commands

Run these from the repo root unless noted otherwise:

Standard V2 checks:

```bash
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
```

Deployment-specific checks:

```bash
docker build -f prototype/gbn-bridge-proto/Dockerfile.publisher-authority prototype/gbn-bridge-proto
docker build -f prototype/gbn-bridge-proto/Dockerfile.publisher-receiver prototype/gbn-bridge-proto
docker build -f prototype/gbn-bridge-proto/Dockerfile.bridge prototype/gbn-bridge-proto
```

```bash
docker compose -f prototype/gbn-bridge-proto/docker-compose.conduit-e2e.yml config
```

```bash
bash -n prototype/gbn-bridge-proto/infra/scripts/deploy-conduit-full.sh
bash -n prototype/gbn-bridge-proto/infra/scripts/smoke-conduit-full.sh
bash -n prototype/gbn-bridge-proto/infra/scripts/teardown-conduit-full.sh
```

```bash
aws cloudformation validate-template \
  --template-body file://prototype/gbn-bridge-proto/infra/cloudformation/conduit-full-stack.yaml
```

Also run:

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

Recommended Phase 8-specific checks:

```bash
rg -n "prototype entrypoints|production network service" prototype/gbn-bridge-proto/infra/README-infra.md
```

```bash
git status --short
```

Expected outcome:

- real authority and receiver images exist
- local compose and AWS templates reflect the full service topology
- deployment scripts are syntactically valid
- the full stack has durable storage and secret/config wiring
- `chain_id` remains visible in service logs and smoke surfaces
- protected V1 paths show no drift
- minimum V1 regression suite remains green

---

## 11. Acceptance Criteria

Phase 8 is complete when:

- separate authority and receiver deployment images exist
- the bridge image runs the real service path
- a real local compose topology exists
- a real full-stack CloudFormation template exists
- deploy/smoke/teardown scripts exist for the full stack
- storage, secrets, and service wiring are defined explicitly
- `chain_id` remains visible in service logs and validation surfaces
- all required V1 and V2 validation commands have been run and recorded

Phase 8 is not complete if:

- the full implementation still relies on placeholder BusyBox compose services
- there is still only one monolithic publisher deployment image
- the stack has no durable database or secrets/config wiring
- deployment remains tied to the old Phase 10 prototype topology

---

## 12. Risks And Blockers

| Risk | Why It Matters | Mitigation |
|---|---|---|
| prototype stack assumptions leak into the full stack | deployment will remain shaped by earlier simulation-era shortcuts | create a new full-stack template and scripts instead of mutating everything in place |
| authority and receiver remain bundled | real service boundaries from earlier phases would be erased in deployment | make separate images a formal acceptance criterion |
| database and secrets are left manual | live deployment would not be reproducible | require explicit stack resources and script inputs for them |
| local compose remains placeholder-only | Phase 9 distributed harness would have no real local topology to target | require a real compose topology in this phase |
| `chain_id` disappears at the deployment/logging layer | later live validation evidence would be hard to correlate | make trace visibility part of the deliverables |

---

## 13. Sign-Off Recommendation

The correct Phase 8 sign-off is:

- Conduit now has real deployment images and a real AWS/local topology
- authority, receiver, and bridge services are deployed explicitly
- durable state and secret/config wiring are part of the stack
- the deployed topology preserves `chain_id` visibility

The correct Phase 8 sign-off is not:

- a renamed version of the old prototype stack
- a placeholder compose topology
- a monolithic deployment that erases the intended service boundaries

