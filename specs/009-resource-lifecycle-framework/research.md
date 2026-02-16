# Research: Resource Lifecycle Management Framework

**Branch**: `009-resource-lifecycle-framework`
**Date**: 2026-02-15
**Status**: Complete — all unknowns resolved

## Current State Assessment

The `nebula-resource` crate is at the end of Phase 1 with a solid foundation:

| Aspect | Status | Details |
|--------|--------|---------|
| Resource trait | Complete | bb8-style: create, is_valid, recycle, cleanup, dependencies |
| Pool | Complete | Semaphore-based, acquire/release/maintain/shutdown |
| Manager | Complete | Type-erased pool storage, DependencyGraph, register/acquire/shutdown |
| Health checker | Complete | Background monitoring, consecutive failure tracking, per-instance tokens |
| Scope isolation | Complete | 6 scope levels with parent chain, deny-by-default containment |
| Lifecycle state machine | Complete | 10 states, transition validation |
| Guard (RAII) | Complete | Drop callback, into_inner, Deref/DerefMut |
| Action integration | Complete | ResourceProvider trait in action crate, Resources bridge in engine |
| Tests | 150+ | Property tests, unit tests, integration tests |

**No NEEDS CLARIFICATION items** — all technical decisions were resolved during codebase exploration.

---

## Decision 1: Event Broadcasting Strategy

**Decision**: Use `tokio::sync::broadcast` channel for lifecycle events.

**Rationale**:
- Broadcast is explicitly recommended for stateless events in CLAUDE.md
- Subscribers process events asynchronously, not blocking the manager
- Built into tokio — no additional dependency
- Configurable buffer size provides backpressure control
- Multiple subscribers (metrics collector, logger, external listeners) supported natively

**Alternatives considered**:
- **mpsc per subscriber**: Requires manager to know subscribers at send time. Rejected — tight coupling.
- **Custom event bus crate**: Over-engineered for internal events. Rejected — YAGNI (Principle VII).
- **Polling-based**: Subscribers poll for state changes. Rejected — higher latency, more complex.

**Configuration**: Buffer size 1024 events (configurable). Lagging subscribers receive `RecvError::Lagged` and can recover.

---

## Decision 2: Metrics Integration Approach

**Decision**: Use the `metrics` crate (metrics facade pattern) for all metrics emission.

**Rationale**:
- `metrics` crate is the Rust ecosystem standard facade (like `log` for logging)
- Decouples metric emission from collection backend (Prometheus, StatsD, etc.)
- Feature-gated (`metrics` feature flag) to keep framework zero-cost when unused
- Already listed as optional dependency in Cargo.toml

**Alternatives considered**:
- **Direct Prometheus client**: Ties framework to one backend. Rejected — not composable.
- **Custom metrics types**: Reinventing the wheel. Rejected — `metrics` crate is mature.
- **OpenTelemetry metrics**: Heavier dependency, more complex API. Rejected — `metrics` is simpler and sufficient.

**Metrics to emit**:
- Counters: `resource.acquire.total`, `resource.release.total`, `resource.create.total`, `resource.cleanup.total`, `resource.error.total`
- Gauges: `resource.pool.size`, `resource.pool.available`, `resource.pool.in_use`
- Histograms: `resource.acquire.duration_seconds`, `resource.health_check.duration_seconds`, `resource.create.duration_seconds`

---

## Decision 3: Tracing Integration Approach

**Decision**: Use `tracing::instrument` attribute on all pub async methods, with structured span fields.

**Rationale**:
- `tracing` is already used across Nebula (engine, telemetry)
- `instrument` attribute is low-overhead and auto-creates spans
- Structured fields (resource_id, scope, pool stats) enable filtering in Jaeger/Zipkin
- Feature-gated (`tracing` feature flag) for zero-cost when unused

**Alternatives considered**:
- **Manual spans**: More flexible but verbose. Rejected — `instrument` covers 90% of cases.
- **OpenTelemetry direct**: Would bypass tracing ecosystem. Rejected — tracing is the standard layer.

**Span hierarchy**:
- `resource.register` (resource_id)
- `resource.acquire` (resource_id, scope)
- `resource.release` (resource_id, duration)
- `resource.health_check` (resource_id, status)
- `resource.shutdown` (resource_count)

---

## Decision 4: Credential Integration Pattern

**Decision**: Pass `Option<Arc<dyn CredentialProvider>>` through `ResourceContext`, not at registration time.

**Rationale**:
- Credentials may rotate — fetching at create-time ensures freshness
- CredentialProvider is already defined in `nebula-action` — reuse the trait
- `nebula-credential` already exists as optional dependency in resource crate
- Feature-gated (`credentials` feature) to avoid forcing credential dependency

**Alternatives considered**:
- **Credentials at registration**: Stale if rotated. Rejected — security risk.
- **Separate CredentialResource**: Credentials as pooled resources. Rejected — over-complicated, different lifecycle.
- **Environment variables**: Not suitable for multi-tenant. Rejected — no isolation.

**Integration points**:
- `Context` gains `credentials: Option<Arc<dyn CredentialProvider>>`
- `Resource::create()` receives credentials via context
- `Resource::recycle()` can refresh credentials if expiring
- Engine's `Resources` adapter passes CredentialProvider from ActionContext

---

## Decision 5: Graceful Shutdown Strategy

**Decision**: Three-phase shutdown with configurable per-phase timeouts.

**Rationale**:
- Current `Manager::shutdown()` is basic — drain pools and cleanup
- Production needs: stop new acquisitions, wait for in-use returns, cleanup all, cancel background tasks
- CancellationToken already propagated from engine → manager → health checker
- Per-phase timeouts prevent shutdown from hanging indefinitely

**Phases**:
1. **Drain** (default 30s): Close semaphore (no new acquires), wait for in-use resources to return
2. **Cleanup** (default 10s): Call `cleanup()` on all pooled instances
3. **Terminate** (default 5s): Cancel all CancellationTokens, join background tasks

**Alternatives considered**:
- **Single timeout**: Can't distinguish between waiting for returns and cleanup. Rejected — less control.
- **Immediate kill**: Leaks resources. Rejected — violates graceful shutdown requirement.

---

## Decision 6: Pool Strategy Selection

**Decision**: Support FIFO (default) and LIFO. Defer LRU to Phase 4 when real use cases exist.

**Rationale**:
- FIFO provides fairness — oldest idle connections used first, spreading wear evenly
- LIFO provides locality — most recently used connections likely still warm (good for databases)
- LRU requires additional tracking overhead with no current use case
- Current pool uses `VecDeque` — FIFO is push_back/pop_front, LIFO is push_back/pop_back

**Alternatives considered**:
- **LRU from start**: Extra complexity without demonstrated need. Rejected — YAGNI.
- **Priority queue**: Over-engineered. Rejected — no priority differentiation needed currently.

---

## Decision 7: Health Check Pipeline (Phase 4)

**Decision**: Multi-stage pipeline with short-circuit on failure.

**Rationale**:
- Single health check conflates connectivity, performance, and dependency health
- Pipeline stages: Connectivity → Performance → DependencyHealth
- Short-circuit: if connectivity fails, skip performance check (saves time)
- Each stage has independent timeout

**Built-in stages**:
1. **Connectivity**: Simple ping/query (fast, catches network issues)
2. **Performance**: Latency under threshold (catches degradation)
3. **DependencyHealth**: All dependencies healthy (catches cascade failures)

---

## Decision 8: Lifecycle Hooks Architecture (Phase 5)

**Decision**: Trait-based hooks with priority ordering and resource-type filtering.

**Rationale**:
- Hooks must be composable — multiple hooks can fire for the same event
- Priority ordering ensures deterministic execution order
- Resource-type filter prevents hooks from firing on irrelevant resources
- `before` hooks can cancel operations (return Err) — enables validation/authorization

**Alternatives considered**:
- **Callback closures**: Less composable, harder to test. Rejected — traits are more idiomatic Rust.
- **Event listeners only**: Can't cancel operations. Rejected — need before-hooks for credential refresh, audit.
- **Middleware pattern**: Too complex for resource lifecycle. Rejected — hooks are simpler.

---

## Decision 9: Driver Crate Architecture (Phase 6)

**Decision**: One crate per driver, each depending only on `nebula-resource` + driver library.

**Rationale**:
- Follows existing pattern: `nebula-sandbox-inprocess` is a separate crate for sandbox impl
- Each driver has unique dependencies (sqlx, redis, rdkafka, mongodb, reqwest)
- Users compile only the drivers they need
- Independent versioning and release cycles

**Driver crate structure** (each follows identical pattern):
```
crates/resource-{name}/
├── Cargo.toml          # depends on nebula-resource + driver lib
├── src/
│   ├── lib.rs          # Resource + HealthCheckable impl
│   └── config.rs       # Config with Custom Debug (redact secrets)
├── tests/
│   └── integration.rs  # testcontainers-based tests
├── examples/
│   └── basic.rs        # Usage example
└── README.md           # Getting started
```

---

## Decision 10: Quarantine and Recovery Strategy (Phase 8)

**Decision**: Automatic quarantine after N consecutive failures, exponential backoff recovery, configurable max attempts.

**Rationale**:
- Consecutive failures (not single failures) indicate persistent problems
- Exponential backoff prevents hammering a failing service
- Max attempts prevent infinite recovery loops
- Manual release provides operator escape hatch

**Configuration defaults**:
- Quarantine threshold: 3 consecutive failures
- Initial backoff: 1 second
- Max backoff: 60 seconds
- Backoff multiplier: 2x
- Max recovery attempts: 10
- After max attempts: permanent failure, operator notification

---

## Decision 11: Type Erasure Strategy for Manager

**Decision**: Keep existing `AnyGuard` (Box<dyn AnyGuardTrait>) approach with `as_any()` for downcasting.

**Rationale**:
- Already implemented and working in manager.rs
- TypeId → String mapping at register() time
- ResourceHandle wraps AnyGuard for action context
- Actions downcast via `Box<dyn Any + Send>` (standard Rust pattern)
- No need to change — design is clean and correct

**Alternatives considered**:
- **Generic manager**: `Manager<R>` — can only hold one resource type. Rejected.
- **Enum dispatch**: Fixed set of resource types. Rejected — not extensible.

---

## Decision 12: Auto-Scaling Strategy (Phase 8)

**Decision**: Rule-based auto-scaling with watermark thresholds, not ML/predictive.

**Rationale**:
- Simple rules are predictable, debuggable, and sufficient for the use case
- Utilization = in_use / max_size — easy to compute from existing PoolStats
- Scale-up: utilization > high_watermark for scale_up_window → add scale_up_step instances
- Scale-down: utilization < low_watermark for scale_down_window → remove scale_down_step idle instances
- Absolute bounds (min_size, max_size) prevent runaway scaling

**Alternatives considered**:
- **ML-based prediction**: Over-engineered, unpredictable. Rejected — Principle VII.
- **Request-rate based**: Requires request tracking. Rejected — utilization is simpler and already available.
