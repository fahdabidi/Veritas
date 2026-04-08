# GBN Phase 1 Prototype — Execution Plan

This document tracks the step-by-step execution roadmap to upgrade the `gbn-proto` workspace into a functioning Zero-Trust DHT-based routing network, culminating in an end-to-end AWS integration test.

## Status trackers:
- `[ ]` Pending
- `[/]` In Progress
- `[x]` Completed

---

## Step 1: Workspace Dependencies & Protocol Schemas
- `[x]` Add `snow` and `libp2p` (with `kad` feature) to the workspace `Cargo.toml`.
- `[x]` In `crates/gbn-protocol/`, define `RelayDescriptor` structs wrapping Ed25519 Public Keys, IPs, and Signatures.
- `[x]` In `crates/gbn-protocol/`, define the core Telescopic Wire formats: `RelayExtend` (carry Noise handshake), `RelayData` (carry encrypted chunks), `RelayHeartbeat`.

## Step 2: Snow Protocol Wrappers (Crypto)
- `[x]` In `crates/mcn-crypto/`, implement `noise.rs` wrappers around the `snow` crate.
- `[x]` Define a `ClientInitiator` workflow and a `RelayResponder` workflow to establish `Noise_XX` handshakes.

## Step 3: Swarm and DHT Foundation (Router Sim)
- `[x]` In `crates/mcn-router-sim/`, implement the libp2p `Swarm` logic.
- `[x]` Ensure the router can bootstrap off a designated seed IP and successfully publish its `RelayDescriptor` into the Kademlia DHT.
- `[x]` Allow the Creator client to passively sync DHT buckets to discover listening relays.

## Step 4: The Onion Router Engine
- `[x]` In `crates/mcn-router-sim/`, replace the basic TCP reading loop with a state-machine that unwraps envelopes.
- `[x]` Handle `RelayExtend` logic: if a router receives an extension request, it dials the next hop, completes the inner handshake, and links the connections in-memory.

## Step 5: Circuit Manager & Dynamic Fallback
- `[x]` Implement the Creator's `CircuitManager`. Ensure it establishes multi-hop nested handshakes (`Guard -> Middle -> Exit`).
- `[x]` Implement the continuous `Heartbeat` PING interval.
- `[x]` Write the Circuit Failure fallback: if a heartbeat times out, dynamically query the DHT for a new route and re-queue un-ACKed chunks to it.

## Step 6: Local Security Integration Tests
- `[x]` Construct `S1.6`: Telescopic Sinkhole simulation (validate Guard is mathematically prevented from dropping middle packet).
- `[x]` Construct `S1.7`: DHT Spoofing simulation (validate Circuit Manager rejects invalid signed IPs).
- `[x]` Construct `S1.9`: Heartbeat Rebuild test (kill standard process manually and ensure video chunks reliably arrive at Publisher).

## Step 7: End-to-End AWS Deployment Validation
- `[x]` Revise `infra/scripts` to compile with the new async libp2p binaries.
- `[x]` Launch the EC2 CloudFormation Stack.
- `[x]` Execute the full 500MB video transmission across the physical cloud instances.
- `[x]` Trigger EC2 Spot Instance termination mid-transfer (`run-tests.sh` explicitly calling AWS CLI terminate on a Relay) to validate production-grade route recovery.
- `[x]` Verify Publisher perfectly reconstructs the hashed original file.
