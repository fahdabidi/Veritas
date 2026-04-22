# Veritas Conduit Infra Placeholders

This directory is reserved for future Conduit-specific deployment assets. Phase 1
does not introduce real AWS templates, deployment scripts, or environment wiring.

## Reserved Naming Rules

| Surface | Convention | Example |
|---|---|---|
| Environment variables | `GBN_BRIDGE_` | `GBN_BRIDGE_PUBLISHER_URL` |
| Container images | `gbn-bridge-proto-` | `gbn-bridge-proto-exit-bridge` |
| CloudFormation stacks | `gbn-bridge-phase2-` | `gbn-bridge-phase2-dev` |
| Metrics namespace | `GBN/BridgeProto` | `GBN/BridgeProto` |

## Phase Boundary

Real deployment assets begin in later phases after:

- the wire model is locked
- the publisher authority plane exists
- the bridge and creator runtimes are no longer placeholders

Until then, keep this directory limited to naming guidance and reserved paths.
