# GBN-PROTO-005 V1 Regression Suite

This document defines the minimum and extended V1 regression gates that protect the frozen Veritas onion-mode baseline during GBN-PROTO-005 work.

## 1. Minimum Required Local Suite

Run these commands from `prototype/gbn-proto/`:

```bash
cargo check --workspace
cargo test -p mcn-router-sim
```

Pass criteria:

- `cargo check --workspace` completes successfully
- `cargo test -p mcn-router-sim` completes successfully
- no protected V1 path drift is introduced while running the suite

## 2. Extended Local Suite

Run this command from `prototype/gbn-proto/` when a phase broadens V2 integration risk or touches shared Rust dependencies:

```bash
cargo test --workspace
```

Pass criteria:

- the full V1 workspace test suite completes successfully
- failures are treated as phase blockers until understood and resolved

## 3. Extended AWS Suite

Use the existing V1 deployment path when a phase could affect deployability, runtime behavior, or any release packaging that depends on the V1 AWS smoke path.

Reference entry points:

- `prototype/gbn-proto/infra/scripts/deploy-smoke-n5.sh`
- `prototype/gbn-proto/infra/scripts/deploy-scale-test.sh`
- `prototype/gbn-proto/infra/scripts/relay-control-interactive.sh`
- `prototype/gbn-proto/infra/scripts/teardown-scale-test.sh`

Required evidence:

- smoke or scale topology deploy succeeds
- the V1 creator can execute `SendDummy` successfully through the established onion path
- teardown succeeds cleanly after the validation run

## 4. When Each Suite Is Mandatory

- Minimum local suite:
  run at the end of every GBN-PROTO-005 implementation phase
- Extended local suite:
  run whenever a phase changes shared crates, shared Cargo dependencies, or any V1-adjacent validation surface
- Extended AWS suite:
  run whenever a phase affects release automation, deployment packaging, runtime orchestration, or any infrastructure-adjacent workflow that could undermine V1 smoke validation

## 5. Failure Handling

- Any failure in the minimum local suite blocks phase sign-off.
- Any failure in an extended suite blocks sign-off for phases that require that suite.
- Protected V1 paths must remain unchanged unless an explicit, separate V1 maintenance decision is approved.
- Later phases must stop and resolve the regression before proceeding.
