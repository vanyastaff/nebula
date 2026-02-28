# Architecture

## Problem Statement

- business problem:
  - workflow execution allocates aggressively and needs predictable memory behavior under burst traffic.
- technical problem:
  - provide low-overhead allocation/reuse primitives with safe defaults, observability hooks, and bounded-pressure controls.

## Current Architecture

- module map:
  - `allocator`: low-level allocation strategies and global manager
  - `arena`: scoped fast allocation and bulk reset semantics
  - `pool`: reusable object pools with health, ttl, and thread-safe variants
  - `cache`: compute, concurrent, partitioned, and multi-level caches
  - `budget`: memory budgets and hierarchy-aware reservation model
  - `stats` and `monitoring`: usage metrics and system-pressure reactions
  - `core`, `error`, `utils`: shared contracts and safety helpers
- data/control flow:
  1. caller selects feature-backed primitives based on workload.
  2. operations return `MemoryResult<T>` with typed failure reasons.
  3. optional stats/monitoring collect pressure and allocator metrics.
  4. callers adapt behavior (throttle, fallback, cleanup) based on signals.
- known bottlenecks:
  - lock contention in shared pool/cache paths under high concurrency
  - mis-sized pools or budgets causing avoidable `PoolExhausted`/`BudgetExceeded`

## Target Architecture

- target module map:
  - preserve current split, improve policy composition across allocator/pool/cache/budget.
- public contract boundaries:
  - stable: `MemoryError`, `MemoryResult`, `prelude`, key config types, core alloc/pool/cache/budget APIs.
  - evolving: advanced monitoring/profiling/async ergonomics.
- internal invariants:
  - unsafe internals remain encapsulated behind safe public contracts.
  - retryability classification must stay consistent with runtime/resilience expectations.
  - feature off-paths must compile and behave deterministically.

## Design Reasoning

- key trade-off 1:
  - one crate with multiple paradigms reduces integration friction but increases API complexity.
- key trade-off 2:
  - aggressive feature-gating lowers baseline overhead but expands test matrix burden.
- rejected alternatives:
  - single global allocator strategy for all workflows was rejected as too rigid for mixed workloads.

## Comparative Analysis

Sources: n8n, Node-RED, Activepieces, Temporal, Prefect, Airflow.

- Adopt:
  - Temporal/Prefect style explicit operational metrics and pressure-aware behavior.
  - n8n/Activepieces-style practical caching/reuse for execution-heavy paths.
- Reject:
  - Node-RED-like broad mutable global context for memory objects without strict boundaries.
  - hidden automatic policy rewrites under pressure without explicit caller visibility.
- Defer:
  - distributed cross-worker memory coordinator.
  - adaptive auto-tuning of all pool/cache knobs by default.

## Breaking Changes (if any)

- change:
  - likely future convergence toward unified runtime memory config.
- impact:
  - direct per-module configuration bootstrap paths may need migration.
- mitigation:
  - dual API window with explicit shims and migration checklist.

## Open Questions

- Q1: should adaptive pressure actions be centralized in one policy object?
- Q2: which feature set should be considered strict long-term stable baseline?
