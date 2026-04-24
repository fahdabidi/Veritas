# GBN-PROTO-005 - Decision Record

**Document ID:** GBN-PROTO-005-DR  
**Status:** Decision Recorded - Conduit remains experimental  
**Last Updated:** 2026-04-23  
**Phase 0 Baseline Release:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)  
**Evidence Source:** [GBN-PROTO-005 Phase 11 Test Results](GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign-Test.md)

---

## 1. Decision

The final GBN-PROTO-005 decision is:

**Conduit (V2) remains experimental.**

Conduit is not promoted to:

- the default mobile transport path
- the default creator upload path
- a replacement for Lattice

Lattice remains:

- the baseline implementation
- the release-facing transport architecture
- the historical and operational reference point

---

## 2. Why This Decision Was Made

The repository now contains a full Conduit prototype implementation across:

- V2 workspace isolation
- protocol wire model
- publisher authority plane
- ExitBridge runtime
- creator bootstrap flow
- bridge-mode data path
- weak discovery
- reachability classification
- V2 integration harness
- V2 AWS deployment assets
- V2 mobile-validation tooling

The local harness and local validation results are strong enough to prove that
the design is implementable as a prototype.

They are not strong enough to prove that Conduit should replace or outrank
Lattice, because the repo still lacks:

- live AWS deployment evidence
- live mobile-network measurements
- real NAT/carrier UDP punch success-rate data
- live batch onboarding latency measurements
- final extended V1 AWS regression after V2 infra merge

That evidence gap is exactly where the project risk sits, so the safe decision
is to keep Conduit experimental.

---

## 3. Assumption Review

| ID | Assumption | Current Result | Decision |
|---|---|---|---|
| A1 | returning creators can reconnect from cached bridge state | supported in local harness | not enough alone to promote |
| A2 | publisher-signed bridge state can replace unauthenticated transport trust | supported locally | accepted for prototype |
| A3 | enough bridges can expose creator-ingress reachability | modeled locally, not measured live | unresolved |
| A4 | coordinated UDP punching on `443` works on a meaningful share of mobile networks | local control flow works, no live carrier data | unresolved |
| A5 | first-time creators can reach Publisher through HostCreator bootstrap | supported in local harness | still needs live validation |
| A6 | Publisher can coordinate authority and batching without becoming a bottleneck | local batch logic works, no live scale data | partially supported |
| A7 | 1-hop encrypted bridge mode is acceptable for this mobile prototype | accepted as a weaker-anonymity prototype tradeoff | accepted for experimental mode only |
| A8 | short-lived leases contain stale bridge state | supported locally | accepted for prototype |
| A9 | failover and reuse can recover from churn quickly enough | supported locally | still needs live timing evidence |
| A10 | V2 can be isolated beside V1 without destabilizing V1 | proven | accepted |
| A11 | bridge descriptors carry enough reachability metadata without reopening schema churn | proven for current prototype scope | accepted |
| A12 | Publisher batch windows can scale onboarding acceptably | logic exists and rollover works locally | live latency still unresolved |

---

## 4. Exit Criteria Review

| Exit Criterion | Current State |
|---|---|
| first-time creator reaches Publisher through HostCreator bootstrap | proven locally |
| seed ExitBridge establishes working UDP tunnel | proven locally |
| new creator receives signed bootstrap bridge/DHT set | proven locally |
| returning creator receives fresh signed bridge update and punches to listed bridges | proven locally |
| encrypted payload reaches Publisher through bridge mode | proven locally |
| bridge reuse and failover work without V1 onion routing | proven locally |
| V1 remains untouched and runnable | proven |
| real-network mobile reachability and onboarding behavior | not yet proven live |

The prototype therefore clears the implementation and local-harness bar, but it
does not yet clear the live-validation bar needed for promotion.

---

## 5. Coexistence Decision

The accepted coexistence rule is:

- **Lattice (V1):** baseline and default
- **Conduit (V2):** experimental parallel transport

Conduit may continue under:

- a separate workspace
- a separate infra footprint
- a separate metrics/test footprint
- V2-only docs and decision records

Conduit must not be presented as the active default transport until a later
approved decision record changes that state.

---

## 6. Ownership And Migration Rules

1. Do not rewrite V1 docs to imply Conduit replaced Lattice historically.
2. Do not migrate release-facing defaults to Conduit yet.
3. Keep V1 and V2 deployment assets separate.
4. Continue using Lattice as the preserved regression reference point.
5. Only consider promotion after live AWS/mobile validation and a new decision review.

---

## 7. Unresolved Risks

- first-contact bootstrap viability under real carrier NAT
- coordinated UDP punch success rates under live network conditions
- bridge failover timing under real churn
- batch onboarding latency under live AWS load
- weak-discovery trust remains intentionally limited, but production abuse resistance is not yet tested beyond prototype scope
- Conduit still has weaker path anonymity than Lattice

---

## 8. Next Gate To Reopen This Decision

This decision should be revisited only after:

1. the committed Phase 10 AWS stack is deployed and validated
2. Phase 11 live AWS/mobile runs are completed and recorded
3. extended V1 AWS regression passes
4. updated evidence shows Conduit is operationally viable under real conditions

Until then, the project should treat Conduit as:

**implemented, locally validated, and still experimental**
