# Data Model: Resource Lifecycle Management Framework

**Branch**: `009-resource-lifecycle-framework`
**Date**: 2026-02-15

## Entity Catalog

This document catalogs all types in the `nebula-resource` system — both existing (already implemented) and planned (by phase).

### Legend

- **[E]** = Existing (implemented, tested)
- **[P2]**–**[P8]** = Planned for that phase
- Fields marked `(opt)` are `Option<T>`

---

## Core Types

### Resource Trait [E]

The central contract for all managed resources.

| Associated Type | Description |
|-----------------|-------------|
| `Config` | Configuration type, must implement `Config` trait |
| `Instance` | The managed connection/instance type |

| Method | Signature | Default | Description |
|--------|-----------|---------|-------------|
| `id` | `&self -> &str` | — | Unique resource type identifier |
| `create` | `&self, config, context -> Result<Instance>` | — | Create new instance |
| `is_valid` | `&self, instance -> Result<bool>` | `true` | Check instance is usable |
| `recycle` | `&self, &mut instance -> Result<()>` | no-op | Prepare for reuse |
| `cleanup` | `&self, instance -> Result<()>` | drop | Destroy instance |
| `dependencies` | `&self -> Vec<&str>` | `[]` | Required resources |

### Config Trait [E]

| Method | Signature | Default | Description |
|--------|-----------|---------|-------------|
| `validate` | `&self -> Result<()>` | `Ok(())` | Validate configuration |

### HealthCheckable Trait [E]

| Method | Signature | Default | Description |
|--------|-----------|---------|-------------|
| `health_check` | `&self -> Result<HealthStatus>` | — | Basic health probe |
| `detailed_health_check` | `&self, context -> Result<HealthStatus>` | — | Context-aware check |
| `health_check_interval` | `&self -> Duration` | 30s | Check frequency |
| `health_check_timeout` | `&self -> Duration` | 5s | Per-check timeout |

---

## Scope & Context

### Scope [E]

Hierarchical resource visibility with parent chain for secure containment.

| Variant | Fields | Parent Chain |
|---------|--------|-------------|
| `Global` | — | Contains all |
| `Tenant` | `tenant_id: String` | — |
| `Workflow` | `workflow_id: String`, `tenant_id: Option<String>` | Tenant |
| `Execution` | `execution_id: String`, `workflow_id: Option<String>`, `tenant_id: Option<String>` | Workflow → Tenant |
| `Action` | `action_id: String`, `execution_id: Option<String>`, `workflow_id: Option<String>`, `tenant_id: Option<String>` | Execution → Workflow → Tenant |
| `Custom` | `key: String`, `value: String` | — |

**Key method**: `contains(other: &Scope) -> bool` — deny-by-default, verifies parent chain membership.

### Strategy [E]

| Variant | Description |
|---------|-------------|
| `Strict` | Exact scope match only |
| `Hierarchical` | Broader scopes can be used (default) |
| `Fallback` | Try exact, then broader |

### Context [E]

Flat execution context passed to resource operations.

| Field | Type | Description |
|-------|------|-------------|
| `scope` | `Scope` | Request scope level |
| `execution_id` | `String` | Current execution ID |
| `workflow_id` | `String` | Current workflow ID |
| `tenant_id` | `Option<String>` | Tenant for multi-tenancy |
| `cancellation` | `CancellationToken` | Cooperative cancellation |
| `metadata` | `HashMap<String, String>` | Arbitrary key-value pairs |
| `credentials` | `Option<Arc<dyn CredentialProvider>>` | [P2] Credential access |

---

## Lifecycle

### Lifecycle [E]

10-state machine for resource instance lifecycle.

| State | Available? | Terminal? | Transitions To |
|-------|-----------|-----------|----------------|
| `Created` | No | No | Initializing |
| `Initializing` | No | No | Ready, Failed |
| `Ready` | Yes | No | InUse, Idle, Maintenance, Draining, Cleanup |
| `InUse` | No | No | Ready, Failed |
| `Idle` | Yes | No | Ready, InUse, Maintenance, Draining, Cleanup |
| `Maintenance` | No | No | Ready, Failed |
| `Draining` | No | No | Cleanup |
| `Cleanup` | No | No | Terminated, Failed |
| `Terminated` | No | Yes | (none) |
| `Failed` | No | Yes* | Cleanup, Terminated |

*Failed allows transition to Cleanup/Terminated for recovery.

---

## Pool

### PoolConfig [E]

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `min_size` | `usize` | 1 | Minimum idle instances |
| `max_size` | `usize` | 10 | Maximum total instances |
| `acquire_timeout` | `Duration` | 30s | Max wait for acquire |
| `idle_timeout` | `Duration` | 600s | Max idle time before cleanup |
| `max_lifetime` | `Duration` | 3600s | Max instance lifetime |
| `validation_interval` | `Duration` | 30s | How often to validate idle instances |

### PoolStats [E]

| Field | Type | Description |
|-------|------|-------------|
| `total_acquisitions` | `u64` | Lifetime acquire count |
| `total_releases` | `u64` | Lifetime release count |
| `active` | `usize` | Currently in-use count |
| `idle` | `usize` | Currently idle count |
| `created` | `u64` | Lifetime instances created |
| `destroyed` | `u64` | Lifetime instances destroyed |

---

## Health

### HealthStatus [E]

| Field | Type | Description |
|-------|------|-------------|
| `state` | `HealthState` | Current health assessment |
| `latency` | `Option<Duration>` | Check response time |
| `metadata` | `HashMap<String, String>` | Additional health details |

### HealthState [E]

| Variant | Fields | Description |
|---------|--------|-------------|
| `Healthy` | — | Fully operational |
| `Degraded` | `reason: String`, `performance_impact: f64` | Working but impaired (0.0–1.0) |
| `Unhealthy` | `reason: String`, `recoverable: bool` | Not operational |
| `Unknown` | — | Status not determined |

### HealthPipeline [P4]

| Field | Type | Description |
|-------|------|-------------|
| `stages` | `Vec<Box<dyn HealthStage>>` | Ordered check stages |

### HealthStage Trait [P4]

| Method | Description |
|--------|-------------|
| `name() -> &str` | Stage identifier |
| `check(instance) -> StageResult` | Execute stage check |

---

## Error

### Error [E]

| Variant | Key Fields | Retryable? | Description |
|---------|-----------|-----------|-------------|
| `Configuration` | `message` | No | Invalid config |
| `Initialization` | `resource_id, source` | No | Creation failed |
| `Unavailable` | `resource_id, retryable` | Depends | Resource not available |
| `HealthCheck` | `resource_id, source` | No | Health check failed |
| `MissingCredential` | `resource_id, credential_key` | No | Credential required but absent |
| `Cleanup` | `resource_id, source` | No | Cleanup failed |
| `Timeout` | `resource_id, operation, duration` | Yes | Operation timed out |
| `CircuitBreakerOpen` | `resource_id` | Yes | Circuit breaker active |
| `PoolExhausted` | `resource_id, max_size` | Yes | No instances available |
| `DependencyFailure` | `resource_id, dependency_id` | No | Dependency failed |
| `CircularDependency` | `cycle` | No | Cycle in dependency graph |
| `InvalidStateTransition` | `from, to` | No | Invalid lifecycle transition |
| `Internal` | `message` | No | Unexpected internal error |

---

## Guard

### Guard\<T\> [E]

RAII wrapper that invokes callback on drop.

| Field | Type | Description |
|-------|------|-------------|
| `resource` | `Option<T>` | The wrapped instance |
| `on_drop` | `Option<Box<dyn FnOnce(T) + Send>>` | Drop callback |

| Method | Description |
|--------|-------------|
| `new(resource, callback)` | Create guard |
| `into_inner()` | Take resource without callback |
| `Deref/DerefMut` | Transparent access |

---

## Manager

### Manager [E]

Central coordinator for all resources.

| Field | Type | Description |
|-------|------|-------------|
| `pools` | `DashMap<String, Arc<dyn AnyPool>>` | Type-erased pool storage |
| `deps` | `RwLock<DependencyGraph>` | Dependency graph |

### DependencyGraph [E]

| Field | Type | Description |
|-------|------|-------------|
| `dependencies` | `HashMap<String, Vec<String>>` | Resource → its dependencies |
| `dependents` | `HashMap<String, Vec<String>>` | Resource → resources depending on it |

| Method | Description |
|--------|-------------|
| `add_dependency(resource, depends_on)` | Register dependency (detects cycles) |
| `topological_sort()` | Kahn's algorithm initialization order |
| `get_init_order(resource)` | Init order for single resource |
| `depends_on(a, b)` | Transitive dependency check |
| `detect_cycle()` | Return cycle if present |

---

## Events [P3]

### ResourceEvent [P3]

| Variant | Fields | Description |
|---------|--------|-------------|
| `Created` | `resource_id, scope` | New resource registered |
| `Acquired` | `resource_id, pool_stats` | Instance acquired from pool |
| `Released` | `resource_id, duration` | Instance returned to pool |
| `HealthChanged` | `resource_id, from, to` | Health state transition |
| `PoolExhausted` | `resource_id, waiters` | Pool capacity reached |
| `CleanedUp` | `resource_id, reason` | Instance cleaned up |
| `Error` | `resource_id, error` | Error occurred |

### CleanupReason [P3]

| Variant | Description |
|---------|-------------|
| `Expired` | Max lifetime exceeded |
| `IdleTimeout` | Idle too long |
| `HealthCheckFailed` | Failed validation |
| `Shutdown` | Manager shutting down |
| `Evicted` | Removed by auto-scaler |

---

## Hooks [P5]

### HookEvent [P5]

| Variant | Key Fields | Description |
|---------|-----------|-------------|
| `BeforeCreate` | `resource_id, config` | Before instance creation |
| `AfterCreate` | `resource_id` | After instance created |
| `BeforeAcquire` | `resource_id` | Before pool acquire |
| `AfterAcquire` | `resource_id, wait_duration` | After acquire completes |
| `BeforeRelease` | `resource_id, usage_duration` | Before return to pool |
| `AfterRelease` | `resource_id` | After returned to pool |
| `BeforeCleanup` | `resource_id, reason` | Before instance destroyed |
| `AfterCleanup` | `resource_id` | After instance destroyed |
| `HealthChanged` | `resource_id, from, to` | Health state transition |

### HookFilter [P5]

| Variant | Description |
|---------|-------------|
| `All` | Fires for all resources |
| `ResourceId(String)` | Fires only for specific resource |
| `ResourceIds(Vec<String>)` | Fires for any in set |

---

## Quarantine [P8]

### QuarantineEntry [P8]

| Field | Type | Description |
|-------|------|-------------|
| `resource_id` | `String` | Quarantined resource |
| `reason` | `QuarantineReason` | Why quarantined |
| `quarantined_at` | `Instant` | When quarantined |
| `recovery_attempts` | `u32` | Attempts so far |
| `max_recovery_attempts` | `u32` | Maximum attempts |
| `next_recovery_at` | `Instant` | Next recovery attempt time |

### QuarantineReason [P8]

| Variant | Description |
|---------|-------------|
| `ConsecutiveFailures(u32)` | N consecutive health check failures |
| `CascadeFailure` | Dependency cascade detected |
| `Manual(String)` | Operator-initiated quarantine |

---

## Auto-Scaling [P8]

### AutoScalePolicy [P8]

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `high_watermark` | `f32` | 0.8 | Scale up threshold |
| `scale_up_window` | `Duration` | 30s | Sustained high duration |
| `scale_up_step` | `usize` | 2 | Instances to add |
| `low_watermark` | `f32` | 0.2 | Scale down threshold |
| `scale_down_window` | `Duration` | 5min | Sustained low duration |
| `scale_down_step` | `usize` | 1 | Instances to remove |
| `min_size` | `usize` | 1 | Absolute minimum |
| `max_size` | `usize` | 50 | Absolute maximum |

---

## Cross-Crate Types

### ResourceProvider (nebula-action) [E]

| Method | Signature | Description |
|--------|-----------|-------------|
| `acquire` | `&self, key: &str -> Result<Box<dyn Any + Send>, ActionError>` | Acquire type-erased resource |

### Resources (nebula-engine) [E]

Bridge adapter: wraps `Manager` to implement `ResourceProvider`.

| Field | Type | Description |
|-------|------|-------------|
| `manager` | `Arc<Manager>` | Resource manager reference |
| `workflow_id` | `String` | Current workflow context |
| `execution_id` | `String` | Current execution context |
| `cancellation` | `CancellationToken` | Per-node cancellation |

### ResourceHandle (nebula-resource) [E]

Opaque wrapper for `AnyGuard`, returned to actions via `ResourceProvider`.

---

## Entity Relationships

```
ResourceProvider (action port)
    │
    ▼
Resources (engine bridge)
    │
    ▼
Manager ──────────► DependencyGraph
    │                    │
    ▼                    ▼
Pool<R> ◄──────── Resource (trait)
    │                    │
    ▼                    ▼
Guard<T> ◄──────── Resource::Instance
    │
    ▼
HealthChecker ──► HealthStatus/HealthState
    │
    ▼
[P3] EventBus ──► ResourceEvent
    │
    ▼
[P5] HookRegistry ──► ResourceHook
    │
    ▼
[P8] QuarantineManager ──► QuarantineEntry
    │
    ▼
[P8] AutoScaler ──► AutoScalePolicy
```
