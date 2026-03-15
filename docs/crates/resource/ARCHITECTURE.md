# Architecture

## Problem Statement

- business problem:
  - workflow nodes need stable access to expensive clients (DB, HTTP, queue, SDK) without recreating them per action.
  - multi-tenant execution must enforce isolation while keeping high throughput.
- technical problem:
  - centralize lifecycle, scope checks, back-pressure, health, and observability with low runtime overhead.

## Current Architecture

### Module Map

| Module | Feature flag | Key exported types | Role |
|--------|-------------|-------------------|------|
| `resource` | always | `Resource` trait, `Config` trait | Core resource definition |
| `manager` | always | `Manager`, `ManagerBuilder` | Central registry; dependency ordering; shutdown; hot-reload |
| `manager_guard` | always | (internal) | Internal RAII guard logic for manager-held locks |
| `manager_pool` | always | (internal) | Internal pool management per resource type |
| `pool` | always | `Pool`, `PoolConfig`, `PoolStrategy` | Bounded concurrency (Semaphore); idle queue; recycle/cleanup; `Gate`/`GateGuard` and `CounterGuard` as internal RAII safety primitives |
| `poison` | always | `Poison<T>`, `PoisonGuard`, `PoisonError` | Arm/disarm guard that permanently marks a value unusable if dropped while armed; protects pool state across async critical sections |
| `scope` | always | `Scope`, `Strategy` | Containment model; tenant/workflow/execution/action hierarchy |
| `context` | always | `Context` | Execution context: scope + workflow ID + exec ID + cancellation |
| `guard` | always | `ResourceGuard` | RAII acquire guard; returns instance to pool on drop |
| `lifecycle` | always | lifecycle hook traits | Create/destroy/recycle lifecycle callbacks |
| `health` | always | `HealthState`, `HealthConfig` | Health checks; degraded/unhealthy state transitions |
| `quarantine` | always | `QuarantineState`, `QuarantineConfig` | Automatic quarantine after repeated failures |
| `autoscale` | always | `AutoscaleConfig` | Optional pool size scaling based on utilization |
| `hooks` | always | `HookRegistry`, acquire/release hooks | Pre/post acquire and release callbacks |
| `events` | always | `EventBus`, `ResourceEvent` | Broadcast lifecycle events via Tokio broadcast channel |
| `metrics` | always | `ResourceMetrics`, `PoolStats` | Utilization counters; acquire latency stats |
| `metadata` | always | `ResourceMetadata` | Display name, description, icon, tags — used by API/desktop UI |
| `reference` | always | `ResourceProvider`, `ResourceRef` | Decoupled typed/dynamic acquire; TypeId-based resource key |
| `error` | always | `Error`, `Result` | Error taxonomy |
| `credentials` | `credentials` | credential-backed resource binding | Integration with `nebula-credential` |
| `dependency_graph` | always | (internal) | Topological ordering for startup/shutdown |

### Data and Control Flow

```
startup:
  Manager::register(resource, config, pool_config)?
    → Config::validate() → fail-fast on invalid config
    → dependency_graph records ordering

acquire:
  Manager::acquire(id, &ctx)  or  Manager::acquire_typed(Resource, &ctx)
    ├─→ scope check (ctx.scope compatible with registration scope?)
    ├─→ quarantine check → Error::Quarantined if active
    ├─→ health check → Error::Unavailable if Unhealthy
    ├─→ run AcquireHooks
    ├─→ Pool::acquire (Semaphore; timeout if max_size reached)
    │       → Resource::create(&config, &ctx) on cache miss
    │       → Error::PoolExhausted / Error::Timeout on back-pressure
    └─→ emit ResourceEvent::Acquired → return ResourceGuard

guard.drop():
  ├─→ Resource::recycle(instance) → pool idle queue if successful
  │   else Resource::cleanup(instance) → permanent removal
  ├─→ run ReleaseHooks
  └─→ emit ResourceEvent::Released; Semaphore permit released

shutdown:
  Manager::shutdown()
    → dependency_graph reverse order
    → gate.close().await → drains in-flight maintenance tasks
    → drain pools → Resource::cleanup per instance
    → emit ResourceEvent::CleanedUp per resource
```

### Known Bottlenecks

- contention around hot resources under strict `max_size`
- expensive `create()` paths during traffic spikes
- full pool replacement on `reload_config` (in-place non-destructive reload not yet supported: see Open Question Q1)

## Pool Safety Hardening Checklist

This checklist tracks cancel-safety and observability patterns implemented in `nebula-resource`.

| Area | Pattern | Status in `nebula-resource` | Next hardening step |
|------|---------|-----------------------------|---------------------|
| Cancel safety in critical sections | Arm/disarm poison guard around mutable shared state | Implemented (`Poison<T>`, `PoisonGuard`, `PoisonError` in `nebula-resource::poison`; `PoolState` wrapped in `Mutex<Poison<PoolState>>>`; drop-without-disarm permanently marks pool as poisoned with timestamp and diagnostic context) | — |
| Failure storm protection | Circuit breaker around expensive fallible operations | Implemented for pool `create` and `recycle` with open/close signaling | Add breaker saturation metrics dashboard examples |
| Shutdown correctness | Gate-style "no new work + wait in-flight" model | Implemented (`Gate`/`GateGuard` from `nebula-resilience::gate`; maintenance task holds `GateGuard`; `shutdown()` calls `gate.close().await` before semaphore close) | — |
| Timeout envelopes | Bound long-running create/recycle operations | Implemented (`create_timeout`, `recycle_timeout`) | Add per-resource timeout profiles in cookbook |
| Observability by RAII | Guard-based counters/timers for wait/run windows | Implemented (`CounterGuard` RAII in `acquire_inner`; dedicated CB open/close counters with `{resource_id, operation}` label via `MetricsCollector`) | — |

## How `nebula-resilience` Participates

`nebula-resource` uses `nebula-resilience` at the pool-operation boundary, while the engine/runtime can still apply broader resilience policies at action execution boundary.

- Inside `nebula-resource` pool:
  - circuit breakers guard `create` and `recycle` paths to prevent retry storms and repeated expensive failures.
  - breaker-open paths return classified errors and emit explicit events for operators.
- Outside `nebula-resource` (engine/runtime/action loop):
  - retry/backoff/rate-limit policies remain orchestration concerns and should wrap action execution flows.
  - resource errors remain policy-carrying inputs (`retryable`, category, retry hints), not policy executors.

This split keeps pool internals self-protecting under pressure, while preserving centralized resilience policy control at higher layers.

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
  - **deny-by-default scope invariant**:
    - if parent-chain information is missing or mismatched, containment is denied.
    - `Scope::contains` must return `false` for ambiguous ancestry (`None` child parent where parent scope is known).
    - manager acquire path treats all scope mismatch as non-retryable (`Error::Unavailable { retryable: false }`).

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
