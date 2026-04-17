# GBN Phase 2 Serverless Scale Test — Results Report

## Overview
This report documents the results of the Phase 2 N=100 serverless scale test run on AWS Fargate (Task GBN-PROTO-004). The goal was to verify the telescopic onion circuit routing and end-to-end data delivery (from a Creator, through 3 hops, to a Publisher) under an ongoing Chaos Engine churn simulating node failure and network jitter.

## Pre-Requisites and Deployment Status
- **Gossip Base:** PlumTree gossip mesh stabilized correctly (`gossipbw` sustained).
- **Scale:** Successfully scaled out 100 ECS tasks and passed Stabilization Gate 1 and Gate 2.
- **Chaos Engine:** Triggered and correctly churned the network at regular intervals. 

## Metrics Summary (N=100)

| Metric | Target | Actual | Status |
|---|---|---|---|
| **Circuit Build Success Rate** | >80% | `null` | **FAIL** (Not logged / No metrics) |
| **Path Diversity** | 100% disjoint | `0` | **FAIL** (Not logged / No metrics) |
| **Chunks Reassembled at Publisher** | ≥ 1 complete session | `0` | **FAIL** |
| **Chunks Received** | Track accumulation | `0` | **FAIL** |
| **Gossip Continuous** | Non-zero BW | Yes (`84-340` bytes/sec) | **PASS** |

## Outcome: FAIL
The Phase 2 automation gate computed a **FAIL** because `phase2_pass` was `false` and `chunks_reassembled_sum` was `0`.

The metrics `circuit`, `circuit_count`, `chunks_reassembled`, `chunks_received`, and `path_diversity` returned empty values from CloudWatch. This implies that the Creator task either:
1. Failed to build circuits (potentially due to a crash or failed Cloud Map discovery of `FreeSubnet` exits).
2. Was unable to push the related CloudWatch metrics due to an IAM role block or error.
3. The upload never triggered.

## Next Steps for Debugging
Before concluding Phase 2, the following must be investigated:
1. Retrieve the `creator` ECS CloudWatch Log stream (`/aws/ecs/gbn-proto-phase1-scale-n100/gbn`) to see if the `build_circuits_speculative()` function timed out or panicked.
2. Verify if the Publisher IP was successfully pushed and retrieved from Cloud Map.
3. Fix the underlying bug and run the sequence again.
