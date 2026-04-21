# GBN-ARCH-001-V2 — Media Creation Network: Conduit Bridge Architecture

**Document ID:** GBN-ARCH-001-V2  
**Architecture Codename:** Conduit  
**Version:** 0.1 (Draft)  
**Status:** In Review  
**Last Updated:** 2026-04-21  
**Parent Architecture:** [GBN-ARCH-000-V2 (Conduit)](GBN-ARCH-000-System-Architecture-V2.md)  
**Related Prototype:** [GBN-PROTO-005](../prototyping/GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign.md)

---

## 1. Overview

The **Conduit** (V2) Media Creation Network redesign introduces a distributed bridge model for creator uploads in environments where creators and relay nodes cannot be treated as stable inbound-reachable overlay nodes.

The core architectural principle becomes:

**A returning creator only needs one reachable, publisher-authorized bridge to refresh its transport view and upload encrypted payloads. A first-time creator needs a one-time host-assisted path to the Publisher so the Publisher can seed the initial bridge set.**

This architecture does not replace the **Lattice** (V1) onion design. It defines a parallel **Bridge Mode** transport.

After the Publisher returns a bridge update, the creator and the listed ExitBridges must immediately attempt coordinated UDP hole punching so they can talk to each other directly. The default UDP port is `443` unless a bridge-specific override is signed into the bridge entry.

---

## 2. Component Diagram

```text
+--------------------------------------------------------------+
| Creator Client                                               |
|                                                              |
|  - Publisher trust root                                      |
|  - Cached Bridge Catalog                                     |
|  - Bridge selector                                            |
|  - Session uploader                                           |
+-------------------------------+------------------------------+
                                |
                                v
+--------------------------------------------------------------+
| ExitBridge                                                    |
|                                                              |
|  - Creator ingress endpoint                                  |
|  - Publisher registration client                             |
|  - Lease manager                                              |
|  - Opaque payload forwarder                                  |
+-------------------------------+------------------------------+
                                |
                                v
+--------------------------------------------------------------+
| Publisher Authority + Receiver                               |
|                                                              |
|  - Bridge registration validator                             |
|  - Bridge lease signer                                        |
|  - Bridge catalog issuer                                      |
|  - Payload receiver + ACK producer                           |
+--------------------------------------------------------------+
```

---

### 2.1 Bootup-Only Host Creator Role

A `HostCreator` is an ordinary creator that already has a working path to the Publisher. It is used only for first-contact onboarding of a `NewCreator`.

The `HostCreator`:

- pairs locally with the `NewCreator`
- relays the encrypted join request to the Publisher through an existing bridge path
- relays the onion response from the Publisher back to the `NewCreator`

The `HostCreator` is not a trust authority and does not choose the new bridge set.

---

## 3. Core Flows

### 3.1 Bridge Registration

```text
ExitBridge
  -> outbound register to Publisher
  -> receives signed lease + preferred UDP punch port (default 443)
  -> begins heartbeat
```

### 3.2 Returning Creator Refresh And Fanout

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
  -> store signed bridge entries in local DHT / discovery cache
  -> immediately send UDP punch probes to listed bridges
Listed ExitBridges
  -> immediately send UDP punch probes back to Creator
Creator + ExitBridges
  -> ACK working tunnels
  -> report progress to Publisher
```

### 3.3 First-Time Creator Bootup Via Host Creator

The one-time first-contact boot flow is:

1. `NewCreator` pairs with a `HostCreator`, triggering a request to join the network.
2. `HostCreator`, using an already-working `ExitBridgeA`, sends an encrypted join request to the Publisher saying a `NewCreator` wants to join.
3. The Publisher creates a bootstrap payload containing:
   - the `NewCreator` DHT entry, including IP address, identity ID, public key, and bootstrap transport metadata
   - DHT entries for 9 active `ExitBridge` nodes
4. The Publisher picks a different active `ExitBridgeB` and sends it the bootstrap payload.
5. `ExitBridgeB` ACKs the message and immediately starts punching a UDP hole toward the `NewCreator` on the selected port, defaulting to `443`.
6. The Publisher sends an onion response back through the established path `Publisher -> ExitBridgeA -> HostCreator -> NewCreator`. That response contains the IP address, public key, and UDP port for `ExitBridgeB`, plus the Publisher public key.
7. `NewCreator` and `ExitBridgeB` start tunneling toward each other on the selected UDP port. When each side receives a packet from the other, it ACKs the tunnel and both sides notify the Publisher that progress is being made.
8. `NewCreator` requests from `ExitBridgeB` the 9 bridge entries that the Publisher seeded.
9. `ExitBridgeB` returns the signed payload of 9 `ExitBridge` entries. `NewCreator` stores them in its local DHT / discovery state and confirms receipt.
10. `ExitBridgeB` informs the Publisher that the seed tunnel is up. The Publisher then triggers the remaining 9 `ExitBridge` nodes to start punching toward the `NewCreator`.
11. Simultaneously, `ExitBridgeB` instructs the `NewCreator` to begin tunneling toward those 9 `ExitBridge` nodes. The Publisher may optionally attach additional bridge-entry payloads for those bridges to pass back.
12. For every successful bridge tunnel, the `NewCreator` marks that DHT entry active in its local table.

### 3.4 Upload Session And Progressive Fanout

```text
NewCreator
  -> split payload into 10 chunks
  -> begin sending chunks as each of the 10 bridge tunnels becomes active
  -> reuse already-active bridges if fewer than 10 bridges become active before timeout
Creator -> ExitBridge -> Publisher
Publisher -> ExitBridge -> Creator ACK
```

---

## 4. Bridge Descriptor Model

### 4.1 Required Fields

| Field | Meaning |
|---|---|
| `bridge_id` | Stable bridge identity |
| `identity_pub` | Public key of the bridge |
| `ingress_endpoints[]` | Creator-visible ingress candidates |
| `udp_punch_port` | Preferred UDP hole-punch port, default `443` unless overridden |
| `reachability_class` | `direct`, `brokered`, or `relay_only` |
| `lease_expiry_ms` | Lease expiry assigned by Publisher |
| `capabilities[]` | Supported session / routing capabilities |
| `publisher_sig` | Publisher signature authorizing use |

### 4.2 Reachability Semantics

| Class | Meaning |
|---|---|
| `direct` | Creator may attempt direct bridge attach |
| `brokered` | Requires additional rendezvous assistance |
| `relay_only` | Not eligible as creator ingress |

### 4.3 Publisher-Seeded DHT Entry Semantics

During bootstrap, the Publisher may distribute DHT-style entries for both creators and bridges. These entries are discovery and transport hints that the creator stores locally, but they are still governed by Publisher authority.

Each bootstrap entry should carry at least:

| Field | Meaning |
|---|---|
| `node_id` / `iid` | Stable identity for the creator or bridge |
| `ip_addr` | Current observed transport endpoint |
| `pub_key` | Public key used for transport identity |
| `udp_punch_port` | Default punch port for that entry |
| `entry_expiry_ms` | Validity window for bootstrap reuse |
| `publisher_sig` | Publisher signature binding the entry contents |

Storing an entry in the DHT does not, by itself, make it authoritative. Transport use still requires a valid Publisher signature and non-expired lease state.

---

## 5. Protocol Outline

### 5.1 Registration And Authority Messages

```text
BridgeRegister
BridgeLease
BridgeHeartbeat
BridgeRevoke
```

### 5.2 Creator Discovery, Refresh, And Bootstrap Messages

```text
BridgeCatalogRequest
BridgeCatalogResponse
BridgeRefreshHint
CreatorJoinRequest
CreatorBootstrapResponse
BridgeSetRequest
BridgeSetResponse
```

### 5.3 Reachability Repair Messages

```text
BridgePunchStart
BridgePunchProbe
BridgePunchAck
BootstrapProgress
BridgeBatchAssign
```

### 5.4 Data Path Messages

```text
BridgeOpen
BridgeData
BridgeAck
BridgeClose
```

---

## 6. Security Model

### 6.1 Trust Boundaries

- Creator trusts Publisher key material out-of-band.
- Creator trusts bridges only through publisher-signed descriptors.
- ExitBridge is allowed to forward opaque payloads but not decrypt creator content.
- Weak discovery cannot elevate a node into a trusted bridge.
- HostCreator may relay bootstrap traffic but cannot authorize bridges or rewrite the Publisher-selected bridge set.
- Bootstrap DHT entries are valid only while the Publisher signature and expiry remain valid.

### 6.2 Comparison To Lattice (V1)

| Property | Lattice — Onion Mode (V1) | Conduit — Bridge Mode (V2) |
|---|---|---|
| Path anonymity | Stronger | Weaker |
| Mobile reachability | Weaker | Stronger |
| Path construction complexity | Higher | Lower |
| Dependency on publisher authority | Lower for routing | Higher for bridge trust |

---

## 7. Failure And Recovery Model

### 7.1 Bridge Failure

If a bridge fails:

1. creator marks bridge suspect
2. creator retries another cached valid bridge
3. creator requests fresh catalog after reconnect
4. creator resumes or restarts upload session
5. creator immediately resumes UDP punching toward any newly assigned bridges

### 7.2 Lease Expiry

The creator must not start new sessions on expired bridge descriptors.

### 7.3 Insufficient Fanout

If the creator does not establish all 10 desired bridge tunnels before timeout:

1. keep the already-working bridge tunnels alive
2. reuse those active bridges for remaining chunk transmission
3. continue background punching toward additional bridges if they are still valid

### 7.4 Catalog Staleness

The creator should maintain:

- a current active catalog
- a previous fallback catalog
- a small bootstrapping seed set

---

## 8. Deployment Model

### 8.1 Implementation Boundary

This architecture is intended to be implemented in a new workspace:

```text
prototype/gbn-bridge-proto/
```

while preserving the existing Lattice (V1) workspace:

```text
prototype/gbn-proto/
```

### 8.2 New Conduit (V2) Components

| Crate | Responsibility |
|---|---|
| `gbn-bridge-protocol` | Wire schemas and bridge descriptor types |
| `gbn-bridge-runtime` | Shared runtime, retries, telemetry, lease handling |
| `gbn-bridge-publisher` | Publisher authority and payload receiver |
| `gbn-bridge-cli` | Creator client and test harness |

---

## 9. Testing Strategy

### 9.1 Minimum Validation Targets

- creator bootstraps from cached signed bridge catalog
- returning creator receives an updated bridge list and immediately hole-punches to it
- first-time creator bootstraps through a HostCreator and reaches the Publisher
- first-time creator receives a seed bridge plus 9 additional bridge entries
- creator establishes bidirectional UDP tunnel ACKs on default port `443` unless overridden
- bridge successfully registers and renews lease
- creator uploads through one bridge
- publisher returns ACK through same bridge session
- creator fails over to a second bridge after first-bridge loss
- creator reuses active bridges when fewer than 10 tunnels are available before timeout

### 9.2 Security Validation Targets

- unsigned bridges are rejected
- expired bridges are rejected
- weak discovery cannot override publisher authority
- bridge cannot read creator plaintext payload
- HostCreator cannot forge the Publisher-provided bridge set
- bootstrap DHT entries without valid Publisher signatures are rejected

---

## 10. Migration Guidance

Conduit should be introduced as:

- a new transport mode
- a separate workspace
- a separate infra footprint
- a separate metrics stream

Only after M3/M4 validation should the project decide whether Conduit remains:

- a parallel mobile transport mode
- or the preferred creator upload mode for later phases
