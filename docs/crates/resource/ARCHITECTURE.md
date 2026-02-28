# Architecture

## Problem Statement

- business problem:
  - workflow nodes need stable access to expensive clients (DB, HTTP, queue, SDK) without recreating them per action.
  - multi-tenant execution must enforce isolation while keeping high throughput.
- technical problem:
  - centralize lifecycle, scope checks, back-pressure, health, and observability with low runtime overhead.

## Current Architecture

- module map:
  - `manager`: registry/orchestration, dependency graph, shutdown, hot-reload
  - `pool`: bounded concurrency (`Semaphore`), idle queue, recycle/cleanup, latency stats
  - `scope`: containment model + compatibility strategies
  - `health`, `quarantine`, `autoscale`: runtime safety controls
  - `hooks`, `events`, `metrics`: observability and extension points
  - `reference`: `ResourceProvider` and `ResourceRef` abstraction layer
- data/control flow:
  1. caller builds `Context` with scope and cancellation token.
  2. `Manager::acquire` validates quarantine, health state, and scope compatibility.
  3. acquire hooks run; pool acquires or creates instance; events emitted.
  4. guard returned; on drop, release hooks and recycle/cleanup path executes.
- known bottlenecks:
  - contention around hot resources under strict `max_size`
  - expensive `create()` paths during spikes
  - full pool replacement on `reload_config`

## Target Architecture

- target module map:
  - keep current modules, add clearer policy layer for acquire modes and reload classes.
- public contract boundaries:
  - `Manager`, `ManagerBuilder`, `Resource`, `Config`, `ResourceProvider`, `PoolConfig`, `Scope` are stable integration contracts.
  - hooks/events are extensibility contracts and must be versioned explicitly.
- internal invariants:
  - no acquire bypasses scope + quarantine + health checks.
  - no dropped guard leaks permits or instances.
  - failed register/reload cannot leave dependency graph in dirty state.

## Design Reasoning

- key trade-off 1:
  - string resource IDs keep dynamic runtime flexibility; typed wrappers reduce mismatch risk.
- key trade-off 2:
  - centralized manager gives uniform policy enforcement but adds a hot-path coordination layer.
- rejected alternatives:
  - per-node ad-hoc pools in action crates were rejected due to duplicated policy and weak isolation guarantees.

## Comparative Analysis

Sources: n8n, Node-RED, Activepieces, Temporal, Prefect, Airflow.

- Adopt:
  - n8n/Activepieces style centralized credential+resource access with explicit runtime contracts.
  - Temporal style explicit failure classification and operational visibility.
  - Airflow/Prefect style operator-level observability hooks (adapted as `hooks` + `events`).
- Reject:
  - Node-RED style broad mutable global context for connection objects; too risky for strict tenant isolation.
  - implicit auto-magic retries in resource layer without policy visibility.
- Defer:
  - distributed global resource scheduler across workers (valuable later, not required for single-node contract stability).
  - live zero-drop reconfiguration for all resource classes.

## Breaking Changes (if any)

- change:
  - future major may introduce typed resource keys as primary API, with string IDs as compatibility layer.
- impact:
  - runtime/action crates using raw IDs may require adapter migration.
- mitigation:
  - dual API window (`ResourceKey<T>` + existing string paths) with compile-time lint warnings.

## Open Questions

- Q1: should `reload_config` support classified in-place updates for non-destructive fields?
- Q2: should back-pressure policy be configured per resource class or per caller context?
