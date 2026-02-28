# Architecture

## Current structure

`nebula-resource` is organized around a central coordinator:

1. `Manager`
2. `Pool<R>`
3. `HealthChecker`
4. `EventBus` + `HookRegistry`
5. `QuarantineManager` + optional `AutoScaler`

`Manager` stores resource pools by string id, validates dependencies, enforces scope compatibility, runs hooks/events, and delegates acquire/release to pools.

## Request flow

1. Caller builds `Context` with requested `Scope`.
2. `Manager::acquire(resource_id, ctx)` checks:
- quarantine status
- health state
- scope compatibility (`Strategy::Hierarchical`)
3. `before` hooks run (`HookEvent::Acquire`).
4. Pool acquires (reuse idle, or create new, or timeout/back-pressure error).
5. `after` hooks run.
6. Guard is returned; on drop it returns instance to pool and runs release-side hook/event logic.

## Scope model

`Scope` includes parent chain information:
- `Global`
- `Tenant { tenant_id }`
- `Workflow { workflow_id, tenant_id }`
- `Execution { execution_id, workflow_id, tenant_id }`
- `Action { action_id, execution_id, workflow_id, tenant_id }`
- `Custom { key, value }`

Containment is deny-by-default when parent chain is incomplete. This prevents cross-tenant or cross-workflow leakage.

## Pool model

`Pool<R>` is generic over `Resource`:
- bounded concurrency via `Semaphore`
- idle queue with FIFO/LIFO strategy
- recycle/cleanup lifecycle
- acquire timeout and pool exhausted errors
- optional maintenance loop for min-size replenishment and expiration cleanup
- latency window for p50/p95/p99 stats

## Health model

Two levels:
- inline validation in pool (`Resource::is_valid`)
- background monitoring (`HealthChecker`, `HealthCheckConfig`)

When failure threshold is exceeded, callback can quarantine resource and propagate unhealthy state.

`HealthPipeline` allows staged checks with short-circuit on `Unhealthy`.

## Integration boundaries

- `nebula-resource` exposes `ResourceProvider` so runtime/action layers depend on trait, not concrete manager internals.
- `nebula-credential` is optional feature (`credentials`) for secure config integration.
- tracing/metrics are optional features and should stay additive.
