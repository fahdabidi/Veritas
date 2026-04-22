# GBN-ARCH-002 - Bridge Protocol V2

**Document ID:** GBN-ARCH-002  
**Status:** Draft  
**Last Updated:** 2026-04-22  
**Related Docs:** [GBN-ARCH-000-V2](GBN-ARCH-000-System-Architecture-V2.md), [GBN-ARCH-001-V2](GBN-ARCH-001-Media-Creation-Network-V2.md), [GBN-PROTO-005 Execution Plan](../prototyping/GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign-Execution-Plan.md)

This document records the canonical Phase 2 Conduit wire model implemented in `gbn-bridge-protocol`.

---

## 1. Scope

Phase 2 defines the shared wire schema only. It does not define runtime loops, publisher service behavior, or deployment wiring.

The protocol layer owns:

- canonical bridge descriptors
- publisher-seeded bootstrap entries
- registration, lease, catalog, bootstrap, punch, and session message shapes
- protocol versioning
- replay-protection metadata
- publisher-signature verification contracts

---

## 2. Canonical M1 Bridge Descriptor

`BridgeDescriptor` is the authoritative transport object for bridge attach decisions.

Required fields:

- `bridge_id`
- `identity_pub`
- `ingress_endpoints[]`
- `udp_punch_port`
- `reachability_class`
- `lease_expiry_ms`
- `capabilities[]`
- `publisher_sig`

Deferred from M1:

- `network_type`
- `geo_tag`
- `observed_reliability_score`

---

## 3. Publisher-Seeded Bootstrap Entries

`BootstrapDhtEntry` is the signed bootstrap hint object distributed during creator onboarding and bridge-set refresh.

Required fields:

- `node_id`
- `ip_addr`
- `pub_key`
- `udp_punch_port`
- `entry_expiry_ms`
- `publisher_sig`

These entries are locally cacheable, but they are only transport-eligible while their Publisher signature remains valid and their expiry window has not elapsed.

---

## 4. Message Families

### 4.1 Lease And Authority

- `BridgeRegister`
- `BridgeLease`
- `BridgeHeartbeat`
- `BridgeRevoke`

### 4.2 Catalog And Bootstrap

- `BridgeCatalogRequest`
- `BridgeCatalogResponse`
- `BridgeRefreshHint`
- `CreatorJoinRequest`
- `CreatorBootstrapResponse`
- `BridgeSetRequest`
- `BridgeSetResponse`

### 4.3 Punch And Batch Control

- `BridgePunchStart`
- `BridgePunchProbe`
- `BridgePunchAck`
- `BootstrapProgress`
- `BridgeBatchAssign`

### 4.4 Bridge Session Data Path

- `BridgeOpen`
- `BridgeData`
- `BridgeAck`
- `BridgeClose`

---

## 5. Versioning And Replay Semantics

All message transport should be wrapped in `ProtocolEnvelope<T>`.

`ProtocolEnvelope` carries:

- `version`
- optional `ReplayProtection`
- a typed message body

The current M1 version is `1`. Unsupported versions must be rejected rather than silently accepted.

Replay-sensitive envelopes should carry:

- `message_id`
- `nonce`
- `sent_at_ms`

Consumers should reject envelopes that are too old for the accepted replay window or that appear to originate from the future.

---

## 6. Signature And Expiry Rules

The protocol layer treats these objects as Publisher-authoritative:

- `BridgeDescriptor`
- `BootstrapDhtEntry`
- `BridgeCatalogResponse`
- `CreatorBootstrapResponse`
- `BridgeSetResponse`
- `BridgeLease`
- `BridgeRevoke`
- `BridgePunchStart`
- `BridgeBatchAssign`

Each authoritative object must expose an unsigned payload and a verification path bound to the Publisher public key.

Expiry checks are required for:

- descriptors
- bootstrap entries
- catalog responses
- bootstrap responses
- bridge-set responses
- leases
- punch-start instructions

---

## 7. Reachability Classes

The M1 reachability model is:

- `direct`
- `brokered`
- `relay_only`

Only `direct` is creator-ingress eligible for first-contact bootstrap in the current prototype assumptions. The other classes remain representable so later phases can implement more nuanced policy without mutating the Phase 2 wire shape.

---

## 8. Module Ownership

The protocol crate keeps the message surface split by domain:

- `descriptor.rs`
- `bootstrap.rs`
- `catalog.rs`
- `lease.rs`
- `punch.rs`
- `session.rs`
- `signing.rs`
- `messages.rs`
- `error.rs`

`messages.rs` is limited to shared envelope glue and the top-level typed message enum.
