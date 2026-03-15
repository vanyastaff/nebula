# Resource Cookbook

Practical patterns for composing `runtime`, `action`, and `resource`.

## 1) Startup registration + scoped acquisition

Use startup to register all resources, then acquire per execution context.

```rust
use nebula_resource::{Context, Manager, PoolConfig, Scope};

let manager = Manager::new();
// manager.register(..., PoolConfig::default())?;

let ctx = Context::new(
    Scope::try_execution_in_workflow("exec-1", "wf-orders", Some("tenant-a".into()))
        .expect("execution/workflow ids must be non-empty"),
    nebula_resource::WorkflowId::new(),
    nebula_resource::ExecutionId::new(),
);
// let guard = manager.acquire(&resource_key, &ctx).await?;
```

## 2) Action-facing error policy

Use `ErrorCategory` for action/runtime policy decisions:

- `Validation` => fail node immediately
- `Fatal` => fail step, no retry
- `Retryable` => apply retry/backoff via resilience layer

Note:

- `nebula-resource` already applies `nebula-resilience` circuit breakers to pool `create`/`recycle` internals.
- keep action-level retry/backoff/rate-limit in engine/runtime so policy stays centralized.

## 3) Backpressure profile selection

Configure per-resource pool behavior:

- `FailFast` for latency-sensitive paths
- `BoundedWait` for throughput-oriented workloads
- `Adaptive` when pressure varies across tenants/workflows

## 4) Runtime reload guardrails

When `reload_config` is invalid:

- old pool remains active
- `ConfigReloadRejected` event is emitted
- caller gets explicit error

## 5) Live observability stream

Subscribe via `manager.event_bus().subscribe()` and forward:

- health transitions
- quarantine transitions
- acquire/release/pool exhaustion
- config reload accepted/rejected

## 6) Tenant-safe metrics

For high-cardinality multi-tenant systems:

- `resource_id` labels are capped
- overflow ids are aggregated under `__other`
