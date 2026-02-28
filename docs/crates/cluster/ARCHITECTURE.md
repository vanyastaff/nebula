# Architecture

## Problem Statement

- business problem:
  - platform must execute workflows reliably across multiple nodes with high availability.
- technical problem:
  - coordinate scheduling, failover, and control-plane decisions without split-brain or inconsistent state.

## Current Architecture

- module map:
  - no `crates/cluster` implementation yet.
  - distributed concerns currently implied across runtime/storage/docs.
- data/control flow:
  - single-node oriented flows exist; cluster control-plane contract is not yet codified.
- known bottlenecks:
  - missing centralized cluster ownership blocks safe scale-out path.

## Target Architecture

- target module map:
  - `membership`: node registration, health, liveness, and leader view
  - `consensus`: control-plane state replication and leader election
  - `scheduler`: placement and rescheduling policies
  - `failover`: node failure detection and recovery orchestration
  - `autoscale`: scale-out/in decision engine
  - `api`: cluster control commands and status interfaces
- public contract boundaries:
  - runtime/worker call into scheduler/placement APIs.
  - storage/event layers provide durable state and signaling.
- internal invariants:
  - one authoritative leader view for control-plane mutations.
  - no workflow placement without membership validation.
  - failover and rebalancing are idempotent and auditable.

## Design Reasoning

- key trade-off 1:
  - consensus-backed safety increases robustness but adds latency/operational complexity.
- key trade-off 2:
  - pluggable scheduling strategies improve flexibility but complicate predictability.
- rejected alternatives:
  - best-effort gossip-only coordination for critical scheduling decisions.

## Comparative Analysis

Sources: n8n, Node-RED, Activepieces, Temporal, Prefect, Airflow.

- Adopt:
  - Temporal/Airflow style robust control-plane and worker coordination principles.
  - explicit scheduling/failover contracts with observability.
- Reject:
  - ad-hoc decentralized scheduling without strong consistency for critical ownership state.
- Defer:
  - multi-region active-active control plane in first release.

## Breaking Changes (if any)

- change:
  - migration from single-node runtime assumptions to cluster-aware scheduling contracts.
- impact:
  - runtime/execution/worker APIs will need cluster context and placement metadata.
- mitigation:
  - adapter layer and staged rollout with compatibility mode.

## Open Questions

- Q1: which consensus backend/library should be adopted for first implementation?
- Q2: what minimum scheduling guarantees are required in v1 (fairness, affinity, stickiness)?
