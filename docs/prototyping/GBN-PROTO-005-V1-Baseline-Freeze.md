# GBN-PROTO-005 V1 Baseline Freeze

This document freezes the Veritas V1 onion-mode baseline so all GBN-PROTO-005 V2 bridge-mode work has a traceable reference point and a protected no-touch scope.

## 1. Baseline Identification

baseline_code_sha: 217ec4ab022b4e6b087b920b1e771bdc9caa6a72
baseline_branch: main
release_tag: gbn-proto-005-phase0-v1-baseline

- Project: `Veritas`
- Baseline mode: `V1 onion mode`
- Release title: `GBN-PROTO-005 Phase 0 - V1 Baseline Freeze`
- Baseline code commit meaning: last approved V1 implementation commit before the Phase 0 release packaging docs

## 2. Protected V1 Paths

These paths are protected by the Phase 0 freeze and must not be changed by later V2 implementation work unless a separate approved V1 maintenance action is opened.

- `prototype/gbn-proto`
- `docs/prototyping/GBN-PROTO-004-Phase2-Serverless-Scale-Onion-Plan.md`
- `docs/prototyping/GBN-PROTO-004-Phase2-Serverless-Scale-Test.md`
- `docs/architecture/GBN-ARCH-000-System-Architecture.md`
- `docs/architecture/GBN-ARCH-001-Media-Creation-Network.md`

## 3. Protected V1 Behaviors

The freeze protects the current V1 onion-mode implementation and its surrounding release-critical behavior, including:

- creator to relay to publisher onion-path upload behavior under `prototype/gbn-proto/`
- V1 DHT, gossip, and direct-node validation behavior
- V1 chunk framing and onion protocol serialization
- V1 publisher receive path and V1 relay runtime behavior
- V1 scale and smoke deployment assets used to validate the onion-mode baseline

## 4. Required Regression Gates

The required regression gates for any later work are defined in:

- [GBN-PROTO-005-V1-Regression-Suite.md](GBN-PROTO-005-V1-Regression-Suite.md)

The minimum local gates that must continue to pass are:

- `cargo check --workspace`
- `cargo test -p mcn-router-sim`

## 5. Release Package

The Phase 0 release package is the combination of:

- this freeze manifest
- [GBN-PROTO-005-V1-Regression-Suite.md](GBN-PROTO-005-V1-Regression-Suite.md)
- `.github/workflows/release-phase0.yml`
- the annotated Git tag `gbn-proto-005-phase0-v1-baseline`
- the GitHub release published from that tag

## 6. Approval Gate

Phase 1 must not begin until the Phase 0 release workflow has completed successfully and this V1 baseline freeze has been published as a GitHub release.

Any change that touches a protected V1 path after this freeze must be handled as explicit V1 maintenance, not as incidental V2 implementation work.
