# GBN-PROTO-005 — Prototyping Plan: Phase 2 — Distributed Peer-to-Peer Onion Redesign

**Document ID:** GBN-PROTO-005  
**Phase:** 2 of 3  
**Status:** Decision Recorded - Conduit remains experimental
**Last Updated:** 2026-04-23
**Related Docs:** [GBN-ARCH-000-V2](../architecture/GBN-ARCH-000-System-Architecture-V2.md), [GBN-ARCH-001-V2](../architecture/GBN-ARCH-001-Media-Creation-Network-V2.md), [GBN-PROTO-004](GBN-PROTO-004-Phase2-Serverless-Scale-Onion-Plan.md), [GBN-PROTO-004-Test](GBN-PROTO-004-Phase2-Serverless-Scale-Test.md)

---

## 1. Phase Goal

**Prove that a mobile-compatible Media Creation Network can deliver encrypted creator payloads through a publisher-authorized, distributed bridge layer by combining host-assisted first-contact bootstrap, publisher-seeded bridge state, and immediate creator/bridge UDP hole punching.**

This phase is a deliberate redesign of the current Phase 2 onion transport assumptions. It keeps the existing `Guard -> Middle -> Exit -> Publisher` implementation intact as the V1 baseline, while introducing a parallel V2 architecture optimized for phones and carrier-constrained networks.

The V2 goal is not to replace the current onion prototype immediately. The goal is to validate that:

- creators inside a hostile geofence can reach at least one publisher-authorized bridge node
- first-time creators can reach the Publisher through a one-time bootstrap path via an already-connected host creator
- bridge nodes can act as distributed meet-me points without becoming a single central dependency
- publisher-signed bridge catalogs and publisher-seeded DHT entries can replace unauthenticated DHT state as the source of truth for usable exits
- creators and bridge nodes can actively repair direct reachability through coordinated UDP punching, defaulting to port `443`
- creators can progressively fan out toward 10 active bridges and begin upload before all 10 are fully established
- the transport can degrade gracefully under mobile churn, IP changes, and partial reachability

---

## 2. Problem Statement

The current Phase 2 prototype assumes the creator can dial a chosen first hop and that candidate exit relays behave like ordinary inbound-reachable overlay nodes. This assumption does not hold on many consumer mobile networks.

### 2.1 Operational Constraint

Most consumer cellular networks apply default-deny filtering to unsolicited inbound traffic. As a result:

- a phone-hosted relay cannot be assumed to accept direct inbound TCP/UDP
- a creator cannot rely on DHT IP addresses alone as proof of usable connectivity
- a phone being "online" and "in the DHT" does not imply that it can serve as an ingress hop
- two mobile nodes that have never communicated cannot be assumed to reach each other directly without coordinated hole punching
- a brand-new creator cannot be assumed to already possess a trustworthy active bridge set

### 2.2 Design Consequence

The network must move from:

- **routing-oriented DHT selection**

to:

- **publisher-authorized bridge selection**

The new default transport path becomes:

```text
Creator -> ExitBridge -> Publisher
```

where the ExitBridge acts as a distributed meet-me node and the Publisher remains the authority over which bridges are valid.

### 2.3 Bootstrap Consequence

Because a new creator does not yet have a trusted active bridge set, first-contact bootstrap must be Publisher-coordinated.

The initial join path becomes:

```text
NewCreator -> HostCreator -> ExitBridgeA -> Publisher
Publisher -> ExitBridgeB -> NewCreator
NewCreator <-> ExitBridgeB
```

After that seed tunnel is live, the Publisher can fan out additional `ExitBridge` entries and coordinated punch instructions.

### 2.4 Reachability Consequence

Receiving a signed bridge list is not enough. After each Publisher bridge update:

- the creator must immediately send UDP punch probes toward the listed bridges
- the listed bridges must immediately send UDP punch probes back toward the creator
- both sides must ACK successful bidirectional reachability
- both sides must report tunnel progress back to the Publisher

The default UDP punch port is `443` unless a bridge-specific override is signed into the bridge entry.

---

## 3. Prototype Scope

This prototype explores a new transport mode called **Bridge Mode** or **Distributed Meet-Me Mode**.

### 3.1 In Scope

- publisher-authorized bridge registration and lease issuance
- returning-creator bootstrap from cached and refreshed bridge catalogs
- first-time creator bootstrap through a one-time host-creator path
- publisher-seeded DHT entry distribution for new creators and active bridges
- coordinated creator/bridge UDP hole punching after bridge refresh and bootstrap
- creator payload delivery through a single bridge hop to publisher, with progressive fanout toward 10 active bridges
- reachability classification of bridge nodes
- reuse of already-live bridges when the full 10-bridge fanout is not ready before timeout
- publisher batching of new-creator onboarding fanout
- bridge rotation, expiry, refresh, and failover
- keeping V1 onion routing untouched in the repository and in deployment flows

### 3.2 Out of Scope

- replacing the V1 onion path in existing `gbn-proto`
- deleting or mutating the current DHT validation logic
- proving full Tor-like anonymity equivalence to the current multi-hop design
- implementing full WebRTC/ICE/TURN internals in the first milestone
- turning HostCreator into a permanent trust authority
- building a general-purpose arbitrary peer mesh between all creators and bridges

---

## 4. Assumptions To Validate

| ID | Assumption | Risk if Wrong |
|---|---|---|
| A1 | A returning creator can maintain enough cached bridge descriptors to reconnect after app restarts or transient disconnects | If not, the system depends too heavily on a live bootstrap seed |
| A2 | Publisher-signed bridge catalogs and Publisher-seeded DHT entries are sufficient to replace unauthenticated DHT state as the source of truth for usable exits | If not, transport trust and discovery remain entangled |
| A3 | A subset of bridge nodes can expose creator-reachable ingress despite mobile and carrier restrictions | If not, the design still cannot bootstrap on real phones |
| A4 | Coordinated UDP hole punching on a default port of `443` is sufficient to establish usable direct creator/bridge tunnels on a meaningful fraction of mobile networks | If not, a stronger rendezvous stack is required earlier |
| A5 | A first-time creator can reliably reach the Publisher through a one-time HostCreator-assisted bootstrap path | If not, first-contact onboarding remains a blocking weakness |
| A6 | The Publisher can act as authority without becoming a single operational bottleneck for bridge refresh and bootstrap fanout | If not, catalog issuance or bootstrap orchestration must be delegated or mirrored |
| A7 | A 1-hop encrypted bridge transport preserves acceptable confidentiality for Phase 2 mobile delivery | If not, the design needs additional layered routing even in bridge mode |
| A8 | Short-lived bridge leases are enough to contain stale IPs and dead mobile nodes | If not, creators will burn time on unusable bridges |
| A9 | Progressive bridge rotation and retry logic can recover from mobile node churn fast enough for real creator uploads | If not, upload reliability collapses under realistic conditions |
| A10 | A new bridge-mode workspace can be added beside `gbn-proto` without destabilizing current deploy/test paths | If not, repo isolation must move to a separate repository earlier |
| A11 | Bridge descriptors can carry enough reachability metadata, including preferred UDP punch port, to guide creator choice without leaking excessive targeting information | If not, either privacy or usability will suffer |
| A12 | The Publisher can batch new-creator bootstrap fanout in short windows without creating unacceptable onboarding latency | If not, Publisher scalability becomes the new bottleneck |

---

## 5. Prototype Components

### 5.1 Repository Strategy

The V2 design must not alter the existing Phase 2 onion implementation.

Recommended repository layout:

```text
prototype/
├── gbn-proto/                  # Existing V1 onion implementation (frozen baseline)
└── gbn-bridge-proto/           # New V2 bridge-mode workspace
    ├── Cargo.toml
    ├── crates/
    │   ├── gbn-bridge-protocol/
    │   ├── gbn-bridge-runtime/
    │   ├── gbn-bridge-publisher/
    │   └── gbn-bridge-cli/
    ├── infra/
    │   ├── cloudformation/
    │   └── scripts/
    └── tests/
```

Rules for coexistence:

- no edits to V1 runtime code paths as part of V2 experimentation
- no shared deployment stack names
- no shared image names
- all new environment variables use `GBN_BRIDGE_*`
- all new metrics use separate namespaces or dimension labels

### 5.2 New Logical Components

#### `gbn-bridge-protocol`
- message schemas for bridge registration, lease, catalog refresh, creator join/bootstrap, UDP punch coordination, session open, payload, and ACK
- bridge descriptor types
- publisher-seeded DHT/bootstrap entry types
- publisher signature verification types

#### `gbn-bridge-runtime`
- shared creator/bridge transport runtime
- transport wrappers
- UDP punch state and tunnel-establishment loops
- first-time creator bootstrap handling
- local DHT / discovery table updates from publisher-seeded entries
- metadata tracing
- lease handling
- retry, refresh, and progressive fanout scheduling

#### `gbn-bridge-publisher`
- publisher authority service
- bridge registration and lease issuance
- catalog generation and signing
- creator bootstrap coordination
- bridge batch fanout coordination for new creators
- payload receive and ACK path

#### `gbn-bridge-cli`
- creator-mode prototype client
- host-creator bootstrap client flow
- bridge-mode local test harness
- observability and manual control hooks

---

## 6. V2 Role Model

### 6.1 CreatorClient

- mobile sender inside geofence
- trusts publisher key out-of-band
- stores cached bridge descriptors and publisher-seeded DHT/bootstrap entries
- immediately begins UDP punch attempts to newly assigned bridges
- progressively fans out toward 10 active bridges
- can reuse already-live bridges if the full bridge set is not ready before timeout
- as a returning creator, needs only one reachable bridge to refresh and upload

### 6.2 HostCreator

- already-connected creator node used only for first-contact bootstrap
- relays a new creator join request through an existing bridge path
- relays the Publisher onion response back to the new creator
- does not choose or authorize the new creator bridge set

### 6.3 ExitBridge

- publisher-authorized bridge node outside geofence
- accepts Publisher bootstrap instructions
- starts UDP hole punching toward creators when assigned
- can return publisher-seeded bridge entries to a new creator after the seed tunnel is live
- forwards opaque payloads to publisher
- may advertise weak discovery information, but is not trusted by DHT alone

### 6.4 PublisherAuthority

- signs bridge leases
- maintains live bridge registry
- returns creator bridge catalogs and punch instructions
- seeds new creators with signed bridge and DHT/bootstrap entries
- selects seed bridges for first-time bootstrap
- batches new-creator onboarding fanout when needed
- validates and stores creator payloads

### 6.5 Optional SeedDiscovery

- weak discovery helper
- may surface possible bridge candidates
- never authoritative for bridge trust

---

## 7. Reachability Model

The V2 system must treat reachability as a first-class property.

### 7.1 Bridge Reachability Classes

| Class | Meaning | Creator Bootstrap Eligible |
|---|---|---|
| `direct` | Creator can dial the bridge directly | Yes |
| `brokered` | Bridge is alive but requires external rendezvous/broker flow | Not for M0/M1 bootstrap |
| `relay_only` | Bridge may maintain publisher connectivity but is not creator-ingress capable | No |

### 7.2 Key Constraint

Being in the DHT is not enough. A bridge descriptor must contain:

- publisher authorization
- lease expiry
- preferred UDP punch port
- reachability class
- creator-ingress hints

### 7.3 Tunnel Establishment Rule

After the Publisher returns a bridge update:

- the creator must immediately start UDP punch probes toward the listed bridges
- the listed bridges must immediately start UDP punch probes back toward the creator
- the default UDP punch port is `443` unless a signed bridge-specific override is present
- both sides must ACK the first successful bidirectional exchange
- both sides should send progress updates back to the Publisher

### 7.4 Bootstrap Reachability Rule

A first-time creator is not assumed to be able to reach 10 bridges immediately.

The bootstrap sequence is:

- establish one Publisher-seeded seed bridge first
- receive the remaining bridge set through that seed bridge
- fan out toward the remaining bridges in parallel
- reuse already-live bridges if the full 10-bridge set is not ready before timeout

---

## 8. Bridge Descriptor

The current `RelayNode` structure is insufficient for bridge mode. A new signed descriptor is required.

```text
BridgeDescriptor
├── bridge_id
├── identity_pub
├── ingress_endpoints[]
├── reachability_class
├── network_type
├── geo_tag
├── capabilities[]
├── lease_expiry_ms
├── observed_reliability_score
└── publisher_sig
```

### 8.1 Design Principle

- DHT entry = weak discovery
- Bridge descriptor = transport trust object

The descriptor must also include a preferred `udp_punch_port`, defaulting to `443` unless a signed bridge-specific override is present.

### 8.2 Publisher-Seeded Bootstrap Entry

During first-time creator bootstrap, the Publisher may also distribute signed DHT/bootstrap entries for creators and bridges.

Those entries should contain at least:

```text
BootstrapDhtEntry
- node_id / iid
- ip_addr
- pub_key
- udp_punch_port
- entry_expiry_ms
- publisher_sig
```

These entries are discovery and transport hints stored by the creator locally. They are not authoritative without a valid Publisher signature and non-expired transport state.

---

## 9. Data Flow

### 9.1 Returning Creator Refresh And Fanout

```text
1. Creator loads publisher trust root
2. Creator loads cached bridge catalog
3. Creator filters for:
   - valid publisher signature
   - unexpired lease
   - reachability_class=direct
4. Creator attempts to connect to one bridge
5. On success, creator requests fresh catalog
6. Publisher returns updated signed bridge entries
7. Creator refreshes local catalog / local DHT discovery table
8. Creator immediately starts UDP punch probes toward the listed bridges
9. Listed bridges immediately start UDP punch probes back toward the creator
10. Working creator/bridge tunnels ACK each other and report progress to Publisher
```

### 9.2 Bridge Registration

```text
1. ExitBridge starts
2. ExitBridge performs outbound registration to Publisher
3. Publisher validates identity/capabilities
4. Publisher returns signed lease and preferred UDP punch port, default `443` unless overridden
5. ExitBridge begins periodic heartbeat
6. ExitBridge waits for creator fanout / bootstrap punch instructions
```

### 9.3 First-Time Creator Bootstrap

```text
1. NewCreator pairs with HostCreator
2. HostCreator, using ExitBridgeA, sends an encrypted join request to Publisher
3. Publisher creates a bootstrap payload containing:
   - NewCreator DHT/bootstrap entry
   - 9 active ExitBridge entries
4. Publisher selects ExitBridgeB and sends it the bootstrap payload
5. ExitBridgeB ACKs the message and starts punching a UDP hole toward NewCreator
6. Publisher returns an onion response through:
   Publisher -> ExitBridgeA -> HostCreator -> NewCreator
7. The response contains:
   - ExitBridgeB IP address
   - ExitBridgeB public key
   - selected UDP punch port, default `443`
   - Publisher public key
8. NewCreator and ExitBridgeB exchange UDP probes and ACKs
9. Both sides notify Publisher that the seed tunnel is working
10. NewCreator requests the 9 seeded bridge entries from ExitBridgeB
11. ExitBridgeB returns the signed bridge payload
12. NewCreator stores those entries locally and confirms receipt
13. Publisher triggers the remaining bridges to start punching toward NewCreator
14. ExitBridgeB instructs NewCreator to start dialing those bridges in parallel
15. NewCreator marks each successfully established tunnel active in its local DHT / discovery table
```

### 9.4 Data Upload And Progressive Fanout

```text
1. Creator splits payload into 10 chunks
2. Creator begins sending chunks as each bridge becomes active
3. Creator uses:
   - BridgeOpen
   - BridgeData
   - BridgeAck
4. ExitBridge forwards opaque payload to Publisher
5. Publisher verifies, stores, and ACKs
6. If fewer than 10 bridges are active before timeout, Creator reuses already-live bridges for remaining chunks
```

### 9.5 Refresh Path

The creator does not need a globally current DHT. It needs:

- one reachable bridge
- one fresh signed bridge catalog

The Publisher may return:

- full catalog slices
- differential updates
- bridge deprecations
- forced rotations

### 9.6 Batched New-Creator Fanout

To scale first-contact onboarding, the Publisher may:

- collect up to 10 new-creator join requests inside a 0.5 second batch window
- assign the same 9 active ExitBridges to that batch
- send the batched bootstrap / punch instructions to those bridges together
- place the 11th new creator into the next batch window immediately
- use bridge progress updates to detect stalled bootstrap attempts and reassign bridges if needed

---

## 10. Security Model

### 10.1 Preserved Properties

- payload confidentiality from the bridge
- publisher-controlled trust over usable bridge nodes
- distributed bridge layer rather than a single meet-me server
- HostCreator is only a transport sponsor and not a trust authority
- bootstrap DHT entries remain subordinate to Publisher signature and expiry validation

### 10.2 Reduced Properties Compared To V1 Onion

- bridge sees creator connection endpoint
- bridge and creator expose more transport metadata during UDP tunnel establishment
- transport timing is less distributed than 3-hop onion
- path unlinkability is weaker

### 10.3 Explicit Positioning

This design should be documented as:

- **V1 Onion Mode**: stronger anonymity, weaker mobile viability
- **V2 Bridge Mode**: stronger mobile viability, weaker path anonymity

---

## 11. Test Plan

### 11.1 Correctness Tests

| Test ID | Test Name | Pass Criteria |
|---|---|---|
| T5.1 | Cached Returning-Creator Bootstrap | Returning creator reconnects using only cached signed descriptors |
| T5.2 | Bridge Refresh And Punch Fanout | Returning creator receives updated bridge entries and starts coordinated UDP punching to them |
| T5.3 | Lease Expiry Enforcement | Creator refuses expired bridges for new sessions |
| T5.4 | First-Time Creator Bootstrap | New creator reaches the Publisher through a HostCreator path, establishes a seed bridge, and receives 9 additional bridge entries |
| T5.5 | Tunnel ACK Establishment | Creator and ExitBridge confirm bidirectional UDP tunnel establishment on default port `443` unless overridden |
| T5.6 | Progressive Fanout Upload | Creator begins chunk upload as bridges come online and reuses active bridges if fewer than 10 are available before timeout |
| T5.7 | Bridge Failover | Creator switches to a second bridge after first bridge failure |
| T5.8 | Publisher Authorization Enforcement | Creator rejects unsigned or badly signed bridge descriptors and bootstrap entries |
| T5.9 | Payload Confidentiality | Bridge cannot decrypt creator payload body |

### 11.2 Operational Tests

| Test ID | Test Name | Pass Criteria |
|---|---|---|
| O5.1 | Dead Bridge Catalog Entry | Creator skips or retires dead bridge after failure budget |
| O5.2 | Bridge Churn | Catalog refresh keeps creator supplied with at least one valid bridge |
| O5.3 | Publisher Lease Rotation | Active bridges renew before expiry without creator-visible outage |
| O5.4 | Partial Mobile Churn | Upload succeeds while a subset of bridges churn or disappear |
| O5.5 | Batched Bootstrap Fanout | Publisher can batch up to 10 new creators into one 0.5 second fanout window without stalling onboarding |
| O5.6 | Insufficient Fanout Reuse | Creator successfully reuses already-live bridges when full 10-bridge fanout is delayed |

### 11.3 Security Tests

| Test ID | Test Name | What It Proves |
|---|---|---|
| S5.1 | Bridge Descriptor Forgery | Only publisher-signed bridge descriptors are trusted |
| S5.2 | Stale Lease Replay | Expired descriptors cannot be replayed into new sessions |
| S5.3 | Bridge Metadata Visibility | Bridge only sees forwarding metadata, not creator plaintext |
| S5.4 | Catalog Poisoning Resistance | Weak DHT discovery cannot override publisher authority |
| S5.5 | HostCreator Trust Boundary | HostCreator cannot forge or replace the Publisher-provided bridge set |
| S5.6 | Bootstrap Entry Forgery | Unsigned or tampered Publisher-seeded DHT/bootstrap entries are rejected |

---

## 12. Milestones

| Milestone | Goal |
|---|---|
| M0 | Finalize V2 documents and repo isolation plan |
| M1 | Bridge registration + lease issuance + preferred UDP punch port |
| M2 | First-time creator bootstrap through HostCreator, seed bridge establishment, and Publisher-seeded DHT entry delivery |
| M3 | Returning creator refresh, immediate UDP punch fanout, and bridge-set update handling |
| M4 | Progressive creator -> ExitBridge -> Publisher encrypted payload delivery with bridge reuse, expiry, rotation, and failover |
| M5 | Real-network validation of mobile reachability and batched new-creator onboarding |

---

## 13. Exit Criteria

This prototype phase is successful when:

- a first-time creator can reach the Publisher through a HostCreator-assisted bootstrap path
- a seed ExitBridge can establish a working UDP tunnel to the new creator
- the new creator can receive and store a signed bridge/DHT bootstrap set from the Publisher
- a returning creator can receive a fresh signed bridge update and immediately hole-punch to the listed bridges
- encrypted payloads reach the Publisher through bridge mode using progressively activated bridges
- bridge reuse and failover work without depending on V1 onion routing
- the V1 implementation remains untouched and runnable

## 14. Decision Outcome

The prototype implementation and local harness work are complete enough to
support a final project decision.

That decision is:

- **Conduit remains experimental**
- **Lattice remains the baseline and release-facing transport mode**

Why this is the current result:

- the repo now proves the Conduit control plane, bootstrap path, reachability
  policy, bridge-mode data path, weak discovery boundary, batching behavior, and
  local validation harness
- the repo does not yet prove live AWS/mobile bootstrap viability, real NAT
  punch success rates, live batch onboarding latency, or extended V1 AWS
  regression after the V2 infra merge

So the Phase 2 redesign assumptions are best read as:

- locally supported for prototype implementation and harness validation
- still incomplete for promotion to the default or release-facing transport path
