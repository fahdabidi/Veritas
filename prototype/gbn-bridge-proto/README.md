# Veritas Conduit Workspace

This workspace contains the V2 Conduit bridge-mode prototype. It exists beside the
frozen V1 Lattice workspace and must evolve without modifying `prototype/gbn-proto/`.

## Scope

Phase 1 creates only the isolated Rust workspace boundary:

- standalone Cargo workspace
- compile-only crate stubs
- V2-local infra placeholder paths
- reserved naming rules for future implementation

Real protocol schemas, runtime behavior, publisher authority logic, and deployment
assets begin in later phases.

## Isolation Rule

The Conduit workspace must remain a sibling of the preserved Lattice workspace:

```text
prototype/
|- gbn-proto/          # Lattice (V1), protected baseline
`- gbn-bridge-proto/   # Conduit (V2), new bridge-mode workspace
```

Do not add Conduit crates to the V1 workspace manifest. Do not place Conduit code
under `prototype/gbn-proto/`.

## Workspace Layout

```text
prototype/gbn-bridge-proto/
|- Cargo.toml
|- .gitignore
|- README.md
|- crates/
|  |- gbn-bridge-protocol/
|  |- gbn-bridge-runtime/
|  |- gbn-bridge-publisher/
|  `- gbn-bridge-cli/
|- infra/
|  |- README-infra.md
|  |- scripts/
|  `- cloudformation/
`- tests/
```

## Phase 1 Commands

From `prototype/gbn-bridge-proto/`:

```bash
cargo fmt --check
cargo check --workspace
cargo test --workspace
```

## Reserved Naming Rules

| Surface | Convention | Example |
|---|---|---|
| Environment variables | `GBN_BRIDGE_` | `GBN_BRIDGE_PUBLISHER_URL` |
| Container images | `gbn-bridge-proto-` | `gbn-bridge-proto-publisher` |
| CloudFormation stacks | `gbn-bridge-phase2-` | `gbn-bridge-phase2-dev` |
| Metrics namespace | `GBN/BridgeProto` | `GBN/BridgeProto` |
| Crate names | `gbn-bridge-*` | `gbn-bridge-runtime` |

## Dependency Policy

Phase 1 intentionally keeps the workspace lean:

- no V1 path dependencies by default
- no copied V1 dependency block
- no shared helper crates yet
- only the minimum code needed to compile placeholder crates
