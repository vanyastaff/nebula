# Implementation Plan: Resource Lifecycle Management Framework

**Branch**: `009-resource-lifecycle-framework` | **Date**: 2026-02-15 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `/specs/009-resource-lifecycle-framework/spec.md`

## Summary

Build the complete resource lifecycle management framework for Nebula — from cleaning up the existing Phase 0/1 foundation (already largely complete) through production readiness (Phase 2), observability (Phase 3), advanced health management (Phase 4), lifecycle hooks (Phase 5), driver crates (Phase 6), developer experience (Phase 7), and enterprise features (Phase 8). The `nebula-resource` crate already has a solid core (~3,500 LOC, 150+ tests) with Resource trait, Pool, Manager, DependencyGraph, HealthChecker, and Scope isolation. The action crate already defines ResourceProvider and the engine already bridges it. The remaining work is production hardening, observability, extensibility, ecosystem drivers, and enterprise-grade resilience.

## Technical Context

**Language/Version**: Rust 2024 Edition (MSRV: 1.92)
**Primary Dependencies**: Tokio async runtime (rt-multi-thread, time, sync), tokio-util (CancellationToken), serde/serde_json, thiserror, dashmap, parking_lot, uuid, chrono, async-trait
**Storage**: In-memory (all pool state, health state, dependency graphs are in-memory data structures)
**Testing**: `cargo test --workspace`, `#[tokio::test(flavor = "multi_thread")]` for async, proptest for property-based tests, criterion for benchmarks
**Target Platform**: Cross-platform (Windows primary development, Linux/macOS support)
**Project Type**: Workspace (11 crates organized in architectural layers)
**Performance Goals**: >100k pool acquire/release ops/sec, <100ms acquire latency under normal load, >50k event throughput/sec, graceful shutdown <5s for 100 pooled resources
**Constraints**: Bounded memory per pool (max_size), bounded channel backpressure, cancellation propagation everywhere, zero credential leakage
**Scale/Scope**: Hundreds of pooled resource instances per engine, tens of resource types, multi-tenant isolation, dependency graphs of 10-20 resources

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- [x] **Type Safety First**: ResourceScope uses enum variants with parent chains (not strings). Lifecycle is a 10-state enum with compile-time transition rules. ResourceId, ExecutionId, WorkflowId are typed identifiers. Pool is generic over `R: Resource`. Guard<T> provides typed RAII.
- [x] **Isolated Error Handling**: `nebula_resource::Error` is defined locally with `thiserror` (12 variants). No dependency on `nebula-error`. Errors converted at crate boundary in engine via `ActionError::fatal()`.
- [x] **Test-Driven Development**: 150+ existing tests, 11 property-based tests. Each new phase will follow TDD — tests first, then implementation. Property tests for state machines, serde roundtrips, scope isolation.
- [x] **Async Discipline**: CancellationToken propagated from engine → manager → pool → health checker. Pool acquire respects cancellation via `tokio::select!`. Health checker uses child tokens per instance. Timeouts: acquire 30s, health check 5s. Bounded channels planned for events (Phase 3). JoinSet for scoped tasks.
- [x] **Modular Architecture**: `nebula-resource` is in the System layer. ResourceProvider port is in `nebula-action` (Domain layer). Engine bridges them via `resource.rs` adapter. No circular dependencies. Driver crates (Phase 6) will be separate workspace members depending on `nebula-resource` only.
- [x] **Observability**: Phase 2 adds `tracing` instrumentation on all pub async methods. Phase 3 adds event broadcasting via `tokio::sync::broadcast` and `metrics` crate integration. Structured logging with resource_id, scope, pool stats as span fields.
- [x] **Simplicity**: Phased delivery means each increment is minimal and shippable. No premature abstractions — hooks (Phase 5), quarantine (Phase 8), auto-scaling (Phase 8) added only when needed. Pool uses semaphore (simple) not custom scheduler.

## Project Structure

### Documentation (this feature)

```text
specs/009-resource-lifecycle-framework/
├── plan.md              # This file
├── research.md          # Phase 0 output — technology decisions
├── data-model.md        # Phase 1 output — entity/type catalog
├── quickstart.md        # Phase 1 output — getting started guide
├── contracts/           # Phase 1 output — trait/API contracts
│   ├── resource-trait.md
│   ├── health-checkable.md
│   ├── resource-provider.md
│   ├── event-system.md
│   └── lifecycle-hooks.md
└── tasks.md             # Phase 2 output (/speckit.tasks)
```

### Source Code (repository root)

```text
crates/
├── resource/              # PRIMARY: Framework crate (System layer) — all phases
│   ├── src/
│   │   ├── lib.rs         # Public API + prelude
│   │   ├── resource.rs    # Resource + Config traits [existing]
│   │   ├── error.rs       # Error enum [existing, expand Phase 3+]
│   │   ├── scope.rs       # Scope enum with parent chains [existing]
│   │   ├── lifecycle.rs   # Lifecycle state machine [existing]
│   │   ├── context.rs     # Context struct [existing]
│   │   ├── guard.rs       # RAII Guard [existing]
│   │   ├── pool.rs        # Pool<R> [existing, expand Phase 4]
│   │   ├── health.rs      # HealthChecker [existing, expand Phase 4]
│   │   ├── manager.rs     # Manager + DependencyGraph [existing, expand Phase 2+]
│   │   ├── events.rs      # EventBus + ResourceEvent [NEW: Phase 3]
│   │   ├── hooks.rs       # HookRegistry + ResourceHook [NEW: Phase 5]
│   │   ├── metrics.rs     # MetricsCollector [NEW: Phase 3]
│   │   ├── quarantine.rs  # QuarantineManager [NEW: Phase 8]
│   │   └── autoscale.rs   # AutoScalePolicy [NEW: Phase 8]
│   ├── tests/
│   │   ├── lifecycle_property.rs   [existing]
│   │   ├── pool_exhaustion.rs      [existing]
│   │   ├── resource_guard.rs       [existing]
│   │   ├── scope_isolation.rs      [existing]
│   │   ├── serde_roundtrip.rs      [existing]
│   │   ├── events_integration.rs   [NEW: Phase 3]
│   │   ├── hooks_integration.rs    [NEW: Phase 5]
│   │   └── quarantine_integration.rs [NEW: Phase 8]
│   ├── benches/
│   │   └── pool_throughput.rs      [NEW: Phase 2]
│   └── examples/
│       └── simple_pool.rs          [existing]
│
├── action/                # MODIFIED: ResourceProvider port (Domain layer)
│   └── src/provider.rs    # ResourceProvider trait [existing, already complete]
│
├── engine/                # MODIFIED: Bridge layer
│   └── src/resource.rs    # Resources adapter [existing, already complete]
│
├── resource-derive/       # NEW CRATE: Proc macros [Phase 7]
│   ├── src/lib.rs
│   └── Cargo.toml
│
├── resource-http/         # NEW CRATE: HTTP driver [Phase 6]
├── resource-postgres/     # NEW CRATE: PostgreSQL driver [Phase 6]
├── resource-redis/        # NEW CRATE: Redis driver [Phase 6]
├── resource-kafka/        # NEW CRATE: Kafka driver [Phase 6]
└── resource-mongodb/      # NEW CRATE: MongoDB driver [Phase 6]
```

**Structure Decision**: The primary work is in the existing `nebula-resource` crate (System layer). The action crate's `ResourceProvider` port and engine's `Resources` bridge are already implemented. New crates are created only for Phase 6 drivers (each depends only on `nebula-resource`) and Phase 7 proc macros (separate crate required by Rust). This follows Principle V — drivers are separate to avoid framework depending on driver-specific dependencies.

## Implementation Phases

### Phase 2: Production Readiness

**Goal**: Make the existing framework production-ready with credentials, metrics, tracing, graceful shutdown, and benchmarks.

**Changes to existing crate**:
- Add `tracing::instrument` on all pub async methods in manager.rs, pool.rs, health.rs
- Add `metrics` crate counters/gauges/histograms for pool stats, health checks, lifecycle operations
- Add `CredentialProvider` integration: `Resource::create()` receives optional credentials via Context
- Harden `Manager::shutdown()` with phased drain/cleanup/terminate and configurable per-phase timeouts
- Add `PoolConfig::validate()` with structured validation errors
- Add property tests (proptest): lifecycle state machine invariants, serde roundtrip for all public types, scope containment transitivity
- Add criterion benchmarks: pool acquire/release throughput, manager register/lookup latency
- Replace 19 aspirational docs with 5 real docs matching actual code

**Feature gates**: `credentials`, `tracing`, `metrics`

### Phase 3: Observability & Events

**Goal**: Full runtime observability via events, metrics, and structured logging.

**New modules**:
- `events.rs`: `ResourceEvent` enum (Created, Acquired, Released, HealthChanged, PoolExhausted, CleanedUp, Error) + `EventBus` using `tokio::sync::broadcast`
- `metrics.rs`: `MetricsCollector` aggregating pool/health/lifecycle metrics, exporting via `metrics` crate

**Changes to existing modules**:
- Manager emits events on acquire/release/register/shutdown
- Pool emits events on exhaustion
- HealthChecker emits events on state transitions

### Phase 4: Advanced Resource Management

**Goal**: Resilience to partial failures — multi-stage health checks, degraded state handling, resource warming, connection recycling, dependency health cascading.

**Changes to existing modules**:
- health.rs: Add `HealthPipeline` with multiple `HealthStage` implementations (connectivity, performance, utilization)
- health.rs: Add `HealthState::Degraded` handling — degrade score affects acquire priority
- pool.rs: Add resource warming (pre-create `min_idle` on register), max_lifetime/idle_timeout enforcement in background maintenance
- manager.rs: Add dependency health propagation — when resource B becomes Unhealthy, resource A (depending on B) is marked Degraded

### Phase 5: Lifecycle Hooks

**Goal**: Extensibility without modifying resource driver code.

**New modules**:
- `hooks.rs`: `ResourceHook` trait (before/after callbacks), `HookRegistry` with priority ordering and resource-type filtering, `HookEvent` enum
- Built-in hooks: AuditHook, MetricsHook, CredentialRefreshHook, SlowAcquireHook

### Phase 6: Ecosystem Drivers

**Goal**: Ready-to-use resource implementations for popular services.

**New crates** (each follows the same pattern):
- `nebula-resource-http`: reqwest::Client wrapper, SSRF protection, configurable health endpoint
- `nebula-resource-postgres`: sqlx::PgPool wrapper (delegates pooling to sqlx), credential from CredentialProvider
- `nebula-resource-redis`: redis::Client wrapper, PING health check
- `nebula-resource-kafka`: rdkafka producer/consumer wrapper, metadata fetch health check
- `nebula-resource-mongodb`: mongodb::Client wrapper, ping health check

Each driver: implements Resource + HealthCheckable, Custom Debug (redact secrets), testcontainers integration tests, README + example.

### Phase 7: Developer Experience

**Goal**: Make creating new resources trivial.

**New crate**: `nebula-resource-derive`
- `#[derive(Resource)]` generates Resource trait impl boilerplate
- `#[derive(ResourceConfig)]` generates Config trait impl with validation
- `#[config(secret)]` generates Custom Debug with redaction

**Additions to `nebula-resource`**:
- `testing` module: `MockResource`, `TestPool`, `ResourceTestHarness`
- 4 examples: basic, pooled, health-checked, authenticated

### Phase 8: Enterprise Features

**Goal**: Enterprise-grade resilience for large deployments.

**New modules**:
- `quarantine.rs`: `QuarantineManager` isolates failed resources, `RecoveryStrategy` with exponential backoff, max attempts, manual release
- `autoscale.rs`: `AutoScalePolicy` with high/low watermarks, scale up/down steps, evaluation windows, min/max bounds
- Cascade failure detection via DependencyGraph health propagation
- Configuration hot-reload: create new pool with new config → drain old → swap

## Complexity Tracking

No constitution violations. All complexity is justified by concrete use cases:

| Decision | Justification | Simpler Alternative |
|----------|--------------|---------------------|
| 5+ new driver crates (Phase 6) | Each driver has unique dependencies (sqlx, redis, rdkafka, mongodb, reqwest). Bundling them would force users to compile all drivers. | Single crate with features — rejected because feature-gated deps still affect compile times and are harder to maintain independently |
| Proc macro crate (Phase 7) | Rust requires proc macros in separate crates. Reduces boilerplate from ~100 lines to ~20 for new resources. | Manual trait impl — acceptable for now, macro added only when 3+ drivers exist to justify it |
| Event broadcasting (Phase 3) | Operators need real-time visibility. `broadcast` channel is the simplest async pub-sub primitive. | Logging only — rejected because logs aren't queryable for dashboards or alerting |
