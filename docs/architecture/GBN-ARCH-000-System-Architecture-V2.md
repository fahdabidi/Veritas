# GBN-ARCH-000-V2 — Veritas: Top-Level System Architecture (Conduit)

**Document ID:** GBN-ARCH-000-V2  
**Architecture Codename:** Conduit  
**Version:** 0.1 (Draft)  
**Status:** In Review  
**Last Updated:** 2026-04-21  
**Related:** [GBN-ARCH-000 (Lattice)](GBN-ARCH-000-System-Architecture.md), [GBN-ARCH-001-V2](GBN-ARCH-001-Media-Creation-Network-V2.md), [GBN-PROTO-005](../prototyping/GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign.md)

---

## Table of Contents

1. [Architecture Philosophy](#1-architecture-philosophy)
2. [System Context Diagram](#2-system-context-diagram)
3. [Component Architecture Overview](#3-component-architecture-overview)
4. [Data Flow Architecture](#4-data-flow-architecture)
5. [Network Topology](#5-network-topology)
6. [Identity and Authority Architecture](#6-identity-and-authority-architecture)
7. [Deployment Model](#7-deployment-model)
8. [Security Architecture](#8-security-architecture)
9. [Scalability Architecture](#9-scalability-architecture)
10. [Architecture Decisions Log](#10-architecture-decisions-log)

---

## 1. Architecture Philosophy

### 1.1 Why Conduit Exists

**Lattice** (V1) assumes that creators and relay nodes can build onion paths across dialable overlay nodes. **Conduit** (V2) introduces a second transport architecture for mobile and carrier-constrained environments where unsolicited inbound reachability cannot be assumed.

Conduit does not replace Lattice. It adds a new transport mode:

- **Lattice — Onion Mode (V1)**
  - stronger path anonymity
  - weaker real-world mobile reachability
- **Conduit — Bridge Mode (V2)**
  - stronger real-world mobile reachability
  - weaker path anonymity

### 1.2 Core Architectural Principles

| Principle | Rationale |
|---|---|
| **Publisher-Authorized Transport** | The Publisher is the authority over which bridge nodes are valid |
| **Reachability Is First-Class** | IP visibility alone is not treated as usable connectivity |
| **Weak Discovery, Strong Authority** | DHT/discovery suggests candidates; signed bridge catalogs authorize use |
| **Coordinated UDP Hole Punching** | Creator/bridge reachability is actively repaired after catalog refresh rather than assumed |
| **Mode Separation** | Bridge Mode and Onion Mode must coexist without destabilizing each other |
| **Incremental Migration** | Conduit is added beside Lattice, not implemented by mutating Lattice runtime paths |

### 1.3 The Five Planes

```text
+------------------------------------------------------------+
| CONTENT PLANE                                              |
| Encrypted media chunks, manifests, payload metadata        |
+------------------------------------------------------------+
| TRANSPORT PLANE                                            |
| Creator <-> ExitBridge UDP tunnel repair and payload relay |
+------------------------------------------------------------+
| BOOTSTRAP PLANE                                            |
| Host-assisted first-contact path for new creators          |
+------------------------------------------------------------+
| AUTHORITY PLANE                                            |
| Publisher-signed bridge leases, catalogs, and punch orders |
+------------------------------------------------------------+
| DISCOVERY PLANE                                            |
| Weak DHT/gossip plus Publisher-seeded bootstrap hints      |
+------------------------------------------------------------+
```

---

## 2. System Context Diagram

```text
+-----------------------------------------------------------------------+
| External World                                                        |
|                                                                       |
|  +---------------------+      +------------------------------------+  |
|  | Creator Device      |      | Mobile Carrier NAT /              |  |
|  | hostile jurisdiction|      | Inbound Default-Deny              |  |
|  +---------------------+      +------------------------------------+  |
|             ^                               |                          |
|             | one-time local pairing        | blocks unsolicited       |
|             |                               | inbound to Creator and   |
|  +--------------------+                     | ExitBridge nodes         |
|  | Host Creator Node  |                     v                          |
|  | bootstrap sponsor  |                                              |
|  +--------------------+                                              |
|                                                                       |
|  +--------------------+                                              |
|  | Viewer             |                                              |
|  +--------------------+                                              |
+-----------------------------------------------------------------------+

                 | bootstrap relay / outbound bridge attach
                 v
+-----------------------------------------------------------------------+
| Global Broadcast Network V2                                           |
|                                                                       |
|  +-------------------------+      opaque payload relay / punch orders  |
|  | Distributed ExitBridge  | --------------------------------------->  |
|  | Layer                   |                                           |
|  +-------------------------+                                           |
|                 |                                                      |
|                 v                                                      |
|  +-------------------------+        publish media        +-----------+ |
|  | Publisher Authority     | --------------------------> | GDS /     | |
|  | + Receiver              |                             | Storage   | |
|  +-------------------------+                             +-----------+ |
|                                                                ^      |
|                                                                |      |
|                                            catalog / playback integration
|                                                                |      |
|                                                           +----------+|
|                                                           | Content  ||
|                                                           | Provider ||
|                                                           | Layer    ||
|                                                           +----------+|
+-----------------------------------------------------------------------+
```

---

## 3. Component Architecture Overview

### 3.1 Component Responsibilities

| Component | Responsibility |
|---|---|
| `CreatorClient` | Loads publisher trust, stores publisher-seeded bridge entries, punches direct UDP tunnels, uploads encrypted payloads |
| `ExitBridge` | Receives publisher instructions, punches UDP reachability probes toward creators, forwards opaque payloads to Publisher |
| `PublisherAuthority` | Registers bridges, signs leases, publishes bridge catalogs, coordinates creator/bootstrap hole punching |
| `PublisherReceiver` | Receives bridge-forwarded payloads and produces ACKs |
| `WeakDiscovery` | Provides non-authoritative bridge candidate hints |
| `GDS / VCP` | Remains downstream of Publisher as in the broader GBN architecture |

### 3.2 Node Type Taxonomy

| Node Type | Primary Role |
|---|---|
| **Creator Device** | Upload origin, bridge consumer |
| **Host Creator Node** | Existing creator that provides one-time bootstrap sponsorship for a new creator |
| **ExitBridge Node** | Distributed meet-me / relay node |
| **Publisher Authority Node** | Trust root and payload receiver |
| **Discovery Node** | Optional weak-discovery helper |

---

## 4. Data Flow Architecture

### 4.1 Returning Creator Refresh Flow

```text
Creator boot
  -> load cached bridge catalog
  -> verify Publisher signatures
  -> filter usable direct bridges
  -> connect to one bridge
  -> request fresh signed catalog
Publisher
  -> return updated bridge set
Creator
  -> refresh local cache / local DHT discovery table
```

### 4.2 Immediate Direct Tunnel Establishment

```text
After Publisher bridge-set update
  -> Creator immediately sends UDP punch probes to listed ExitBridges
  -> listed ExitBridges immediately send UDP punch probes back to Creator
  -> default UDP port is 443 unless a bridge-specific override is signed by Publisher
  -> both sides ACK successful bidirectional reachability
  -> both sides report tunnel progress back to Publisher
```

### 4.3 First-Time Creator Boot Flow

```text
NewCreator pairs with HostCreator
  -> HostCreator relays encrypted join request through ExitBridgeA to Publisher
  -> Publisher creates bootstrap payload with:
     - NewCreator DHT entry
     - 9 active ExitBridge entries
  -> Publisher selects ExitBridgeB and sends it the bootstrap payload
  -> ExitBridgeB starts punching a UDP hole toward NewCreator
  -> Publisher returns onion response through ExitBridgeA -> HostCreator -> NewCreator
  -> response carries ExitBridgeB endpoint, selected UDP port, and Publisher public key
  -> NewCreator and ExitBridgeB exchange UDP probes and ACKs
  -> NewCreator requests the 9-bridge set from ExitBridgeB
  -> ExitBridgeB returns the signed bridge payload
  -> Publisher and ExitBridgeB fan out punch requests to the remaining bridges
  -> NewCreator starts dialing those bridges in parallel
```

### 4.4 Upload Flow

```text
Creator
  -> BridgeOpen
  -> BridgeData
ExitBridge
  -> forward opaque payload
Publisher
  -> verify / store / ACK
ExitBridge
  -> BridgeAck
Creator
```

### 4.5 Recovery Flow

```text
Bridge failure
  -> mark bridge suspect
  -> select next cached usable bridge
  -> reconnect
  -> request fresh catalog
  -> resume or restart upload session
  -> if fewer than 10 live bridges are available before timeout, reuse already-live bridges for remaining chunks
```

---

## 5. Network Topology

### 5.1 Topological Shift From Lattice

Lattice (V1):

```text
Creator -> Guard -> Middle -> Exit -> Publisher
```

Conduit (V2):

```text
Creator -> ExitBridge -> Publisher
```

### 5.2 Topology Implication

The new transport assumes:

- creators do not need full overlay path construction
- returning creators only need one reachable publisher-authorized ingress bridge to refresh state
- first-time creators need one host-assisted bootstrap path to the Publisher
- bridge catalogs replace relay-set path construction as the immediate routing input
- creator-to-bridge direct tunnels are actively opened by coordinated UDP punching instead of passive inbound dialing

### 5.3 First-Boot Topology

```text
NewCreator -> HostCreator -> ExitBridgeA -> Publisher
Publisher -> ExitBridgeB -> NewCreator
NewCreator <-> ExitBridgeB
NewCreator <-> ExitBridge[1..9]
```

---

## 6. Identity And Authority Architecture

### 6.1 Publisher Authority

The Publisher becomes authoritative for:

- bridge authorization
- bridge lease issuance
- bridge catalog signing
- bridge rotation and revocation

### 6.2 Bridge Descriptor Trust

V2 introduces a signed bridge descriptor:

```text
BridgeDescriptor
  bridge_id
  identity_pub
  ingress_endpoints[]
  udp_punch_port
  reachability_class
  lease_expiry_ms
  capabilities[]
  publisher_sig
```

### 6.3 Discovery Boundary

- DHT / weak discovery can suggest bridge candidates
- only publisher-signed descriptors make a bridge transport-eligible
- Publisher-seeded DHT entries may populate a creator-local discovery table during bootstrap
- storage in the DHT is not by itself authorization; Publisher signature and lease state remain authoritative

### 6.4 Publisher-Orchestrated Bootstrap Authority

The Publisher is authoritative for first-contact bootstrap:

- only the Publisher may issue the initial 9-bridge bootstrap set for a new creator
- a HostCreator is a transport sponsor, not a trust authority
- ExitBridges begin bootstrap punching only after Publisher instruction or an already-established relationship
- the Publisher chooses the default UDP punch port, currently `443`, unless a signed bridge-specific override is present

---

## 7. Deployment Model

### 7.1 Coexistence Rule

Conduit must be deployed beside Lattice.

Recommended structure:

```text
prototype/gbn-proto/         # Lattice (V1) onion mode
prototype/gbn-bridge-proto/  # Conduit (V2) bridge mode
```

### 7.2 Deployment Isolation

| Isolation Concern | Conduit Rule |
|---|---|
| Image names | New image names for bridge-mode components |
| Stack names | Separate CloudFormation stack names |
| Metrics | Separate namespace or dimension set |
| Environment variables | `GBN_BRIDGE_*` prefix |
| Scripts | Separate deploy/build/test scripts |

---

## 8. Security Architecture

### 8.1 Security Properties Preserved

- payload confidentiality from ExitBridge
- publisher control over transport-authorized exits
- no reliance on one central meet-me node

### 8.2 Security Properties Reduced

- bridge sees creator transport endpoint
- timing correlation is easier than in Lattice (V1) onion mode
- single-hop transport reveals more adjacency information than multi-hop onion

### 8.3 Security Positioning

Conduit is a **mobile viability architecture**, not a full anonymity replacement for Lattice (V1).

---

## 9. Scalability Architecture

Conduit scales by:

- allowing many small bridge nodes rather than one central broker
- pushing authority decisions to signed catalogs
- reducing creator path-construction complexity
- replacing DHT-based exit trust with lease-based bridge trust

Primary scaling concerns:

- publisher authority throughput for catalog issuance
- bridge lease churn
- creator cache freshness under rapid bridge turnover

### 9.1 Batched New-Creator Bootstrap

To scale first-contact onboarding, the Publisher may batch bootstrap fanout:

- collect up to 10 new-creator join requests inside a 0.5 second window
- assign the same 9 active ExitBridges to that batch
- send the batched punch/bootstrap instructions to those bridges together
- place the 11th join request into the next batch window immediately
- allow bridges to return progress updates so the Publisher can detect stalled batches and reassign bridges

---

## 10. Architecture Decisions Log

| Decision | Outcome |
|---|---|
| Keep Lattice (V1) onion implementation untouched | Accepted |
| Introduce Conduit (V2) as additive transport mode | Accepted |
| Make Publisher the bridge authority | Accepted |
| Treat DHT as weak discovery only | Accepted |
| Model mobile reachability explicitly | Accepted |
| Do not assume all phone-hosted bridges are directly dialable | Accepted |
| Perform immediate creator/bridge UDP hole punching after Publisher bridge refresh | Accepted |
| Use one-time HostCreator-assisted bootstrap for first-contact creator onboarding | Accepted |
| Batch new-creator bootstrap fanout in short Publisher windows | Accepted |
