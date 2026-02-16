# Tasks: Resource Lifecycle Management Framework

**Input**: Design documents from `/specs/009-resource-lifecycle-framework/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/
**Existing code**: Phase 0/1 largely complete (~3,500 LOC, 150+ tests). Tasks focus on improving existing code and building remaining phases (2-8).

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (US1–US7)
- Include exact file paths in descriptions

---

## Phase 1: Setup & Cleanup

**Purpose**: Clean up dead code, fix unused features, establish foundation for improvements

- [X] T001 Remove unused `lifecycle::Event` struct (dead code) from `crates/resource/src/lifecycle.rs`
- [X] T002 [P] Remove unused Cargo feature flags (`pooling`, `alloc`, `testing`) or wire them up in `crates/resource/Cargo.toml`
- [X] T003 [P] Re-export `HealthRecord` from `crates/resource/src/lib.rs` so users can import from crate root
- [X] T004 [P] Add field-level rustdoc comments to `Context` struct in `crates/resource/src/context.rs`
- [X] T005 [P] Add a `prelude` module to `crates/resource/src/lib.rs` re-exporting the most commonly used types
- [X] T006 Validate empty string scope IDs — add guard in `Scope` constructors or `contains()` in `crates/resource/src/scope.rs`
- [X] T007 Run quality gates: `cargo fmt --all`, `cargo clippy -p nebula-resource -- -D warnings`, `cargo test -p nebula-resource`, `cargo doc --no-deps -p nebula-resource`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core improvements to existing code that MUST be complete before new user story work

**Context**: The user requested improving existing code and doing it properly. These tasks fix the critical gaps found in the audit.

- [X] T008 [US1] Add scope validation to `Manager::acquire()` — check that `ctx.scope` is compatible with the resource's registered scope before delegating to pool in `crates/resource/src/manager.rs`
- [X] T009 [US1] Integrate `HealthChecker` with `Manager` — when health checks detect unhealthy instances, notify the pool to evict them; add `HealthChecker` field to `Manager` in `crates/resource/src/manager.rs`
- [X] T010 [US1] Implement phased graceful shutdown in `Manager::shutdown()` — drain phase (stop new acquires, wait for in-use returns with timeout), cleanup phase (call cleanup on all instances), terminate phase (cancel background tasks) in `crates/resource/src/manager.rs`
- [X] T011 [US1] Add dependency-aware shutdown ordering to `Manager::shutdown()` — shut down dependents before dependencies using reverse topological sort in `crates/resource/src/manager.rs`
- [X] T012 [US1] Add automatic `maintain()` scheduling — spawn a background task in `Pool` that periodically calls `maintain()` (configurable interval, cancellable via CancellationToken) in `crates/resource/src/pool.rs`
- [X] T013 [US1] Add `credentials` field to `Context` — `Option<Arc<dyn CredentialProvider>>` with `with_credentials()` builder, gated behind `credentials` feature in `crates/resource/src/context.rs`
- [X] T014 Run quality gates: `cargo fmt --all`, `cargo clippy -p nebula-resource -- -D warnings`, `cargo test -p nebula-resource`, `cargo doc --no-deps -p nebula-resource`

**Checkpoint**: Foundation improved — scope validation, health integration, phased shutdown, and credential support in place

---

## Phase 3: User Story 1 — Workflow Author Connects External Services (Priority: P1)

**Goal**: Actions acquire managed, pooled resources by name with automatic return and cleanup on shutdown.

**Independent Test**: Register a resource, acquire from action, use, verify pool return and shutdown cleanup.

**Note**: Core acquire/release flow exists but needs hardening: LIFO strategy, tracing on acquire/release, proper integration tests through engine.

### Tests for User Story 1

- [X] T015 [P] [US1] Write property test: pool acquire/release roundtrip preserves pool invariants (idle + active <= max_size) in `crates/resource/tests/pool_property.rs`
- [X] T016 [P] [US1] Write integration test: full engine → action → resource acquire → use → release → shutdown cycle in `crates/engine/tests/resource_integration.rs` (extend existing file)
- [X] T017 [P] [US1] Write test: phased shutdown waits for in-use resources then cleans up in `crates/resource/tests/shutdown_integration.rs`
- [X] T018 [P] [US1] Write test: scope validation denies cross-scope acquire in `crates/resource/tests/scope_isolation.rs` (extend existing)

### Implementation for User Story 1

- [X] T019 [P] [US1] Add configurable pool selection strategy (FIFO/LIFO) via `Strategy` enum in `PoolConfig` and branch on `pop_front()` vs `pop_back()` in `crates/resource/src/pool.rs`
- [X] T020 [P] [US1] Add tracing instrumentation for acquire and release operations (span with resource_id, scope, wait_duration) gated behind `tracing` feature in `crates/resource/src/pool.rs`
- [X] T021 [P] [US1] Add tracing instrumentation for register, acquire, shutdown operations in `crates/resource/src/manager.rs`
- [X] T022 [US1] Wire `CredentialProvider` from engine's `Resources` adapter through `Context` to `Resource::create()` in `crates/engine/src/resource.rs`
- [X] T023 [US1] Run quality gates and verify all US1 acceptance scenarios pass

**Checkpoint**: US1 complete — actions can acquire/release pooled resources with scope validation, tracing, credentials, and phased shutdown

---

## Phase 4: User Story 2 — Platform Operator Monitors Resource Health (Priority: P2)

**Goal**: Operators see health status, events, and metrics for all managed resources in real time.

**Independent Test**: Register resource with health checks, simulate failure, verify event emission, metrics update, and structured logs.

### Tests for User Story 2

- [X] T024 [P] [US2] Write test: `EventBus` emits `ResourceEvent` variants and subscribers receive them in `crates/resource/tests/events_integration.rs`
- [X] T025 [P] [US2] Write test: `MetricsCollector` updates counters/gauges/histograms from events in `crates/resource/tests/metrics_integration.rs`
- [X] T026 [P] [US2] Write test: health state transition emits `HealthChanged` event in `crates/resource/tests/events_integration.rs`
- [X] T027 [P] [US2] Write test: pool exhaustion emits `PoolExhausted` event in `crates/resource/tests/events_integration.rs`

### Implementation for User Story 2

- [X] T028 [P] [US2] Implement `ResourceEvent` enum and `CleanupReason` enum in `crates/resource/src/events.rs` (new file) per contracts/event-system.md
- [X] T029 [P] [US2] Implement `EventBus` using `tokio::sync::broadcast` with configurable buffer (default 1024) in `crates/resource/src/events.rs`
- [X] T030 [US2] Integrate `EventBus` into `Manager` — emit `Created` on register, `Acquired`/`Released` on acquire/release, `Error` on failures in `crates/resource/src/manager.rs`
- [X] T031 [US2] Integrate `EventBus` into `Pool` — emit `PoolExhausted` when semaphore is full, `CleanedUp` on eviction in `crates/resource/src/pool.rs`
- [X] T032 [US2] Integrate `EventBus` into `HealthChecker` — emit `HealthChanged` on state transitions in `crates/resource/src/health.rs`
- [X] T033 [P] [US2] Implement `MetricsCollector` — background task subscribing to `EventBus`, updating `metrics` crate counters/gauges/histograms in `crates/resource/src/metrics.rs` (new file)
- [X] T034 [US2] Add structured tracing spans with `resource.id`, `resource.scope`, `pool.size`, `pool.available` fields on all lifecycle operations in `crates/resource/src/manager.rs` and `crates/resource/src/pool.rs`
- [X] T035 [US2] Export `events` and `metrics` modules from `crates/resource/src/lib.rs` with appropriate feature gates
- [X] T036 [US2] Run quality gates and verify all US2 acceptance scenarios pass

**Checkpoint**: US2 complete — events broadcast on every lifecycle operation, metrics exported, structured logging with context

---

## Phase 5: User Story 3 — Resource Author Creates a New Driver (Priority: P3)

**Goal**: A developer implements the Resource trait in <50 LOC and the framework handles pooling, health, and lifecycle automatically.

**Independent Test**: Implement an in-memory test resource, register it, verify full lifecycle management.

### Tests for User Story 3

- [X] T037 [P] [US3] Write benchmark: pool acquire/release throughput (target >100k ops/sec) in `crates/resource/benches/pool_throughput.rs` (new file)
- [X] T038 [P] [US3] Write test: reference resource implementation covers full lifecycle (create, validate, recycle, cleanup, dependencies) in `crates/resource/tests/reference_resource.rs`

### Implementation for User Story 3

- [X] T039 [P] [US3] Improve config validation — add structured `ValidationError` with field name, constraint, actual value to `Error` enum in `crates/resource/src/error.rs`
- [X] T040 [P] [US3] Create reference implementation example: `InMemoryCache` resource showing complete lifecycle in `crates/resource/examples/basic_resource.rs`
- [X] T041 [P] [US3] Create pooled resource example: resource with custom PoolConfig, health checking, dependencies in `crates/resource/examples/pooled_resource.rs`
- [X] T042 [US3] Write `crates/resource/docs/ResourceTrait.md` — tutorial for implementing the Resource trait (replaces aspirational docs)
- [X] T043 [US3] Write `crates/resource/docs/Pooling.md` — pool configuration, strategies (FIFO/LIFO), sizing guide
- [X] T044 [US3] Run quality gates, verify benchmarks meet >100k ops/sec target, `cargo doc` builds cleanly

**Checkpoint**: US3 complete — clear docs, examples, benchmarks proving the framework is easy to extend and performant

---

## Phase 6: User Story 4 — Multi-Tenant Platform Isolates Resources (Priority: P4)

**Goal**: Resources scoped to tenants are isolated — cross-tenant access is impossible.

**Independent Test**: Create resources scoped to Tenant A and Tenant B, verify cross-access denied.

### Tests for User Story 4

- [X] T045 [P] [US4] Write property test: scope containment is transitive — if A contains B and B contains C, then A contains C in `crates/resource/tests/scope_property.rs`
- [X] T046 [P] [US4] Write integration test: Manager enforces scope on acquire (deny cross-tenant, allow same-tenant, allow global) in `crates/resource/tests/scope_manager_integration.rs`
- [X] T047 [P] [US4] Write test: parent scope shutdown cascades cleanup to child-scoped resources in `crates/resource/tests/scope_cascade.rs`

### Implementation for User Story 4

- [X] T048 [US4] Add scoped pool support to `Manager` — register resources with a `Scope`, store scope alongside pool, validate on acquire in `crates/resource/src/manager.rs`
- [X] T049 [US4] Implement cascade cleanup — when a scope is terminated, find and shut down all pools with child scopes in `crates/resource/src/manager.rs`
- [X] T050 [US4] Wire scope from engine — `Resources` adapter uses workflow/execution/tenant IDs to build proper `Scope` instead of always `Scope::Global` in `crates/engine/src/resource.rs`
- [X] T051 [US4] Run quality gates, verify all scope isolation tests pass including property tests

**Checkpoint**: US4 complete — multi-tenant resource isolation enforced at manager level with cascade cleanup

---

## Phase 7: User Story 5 — Operator Handles Partial Failures Gracefully (Priority: P5)

**Goal**: Failed resources are quarantined, recovery is attempted with backoff, cascade failures detected.

**Independent Test**: Simulate N consecutive health failures, verify quarantine, backoff recovery, dependency cascade.

### Tests for User Story 5

- [X] T052 [P] [US5] Write test: resource quarantined after N consecutive health failures in `crates/resource/tests/quarantine_integration.rs`
- [X] T053 [P] [US5] Write test: quarantined resource recovery with exponential backoff in `crates/resource/tests/quarantine_integration.rs`
- [X] T054 [P] [US5] Write test: dependency cascade — B unhealthy marks A as degraded in `crates/resource/tests/quarantine_integration.rs`
- [X] T055 [P] [US5] Write test: successful recovery returns resource to active pool in `crates/resource/tests/quarantine_integration.rs`

### Implementation for User Story 5

- [X] T056 [P] [US5] Implement `HealthPipeline` and `HealthStage` trait with short-circuit semantics in `crates/resource/src/health.rs`
- [X] T057 [P] [US5] Implement built-in health stages: `ConnectivityStage`, `PerformanceStage` in `crates/resource/src/health.rs`
- [X] T058 [US5] Implement degraded state handling — when HealthState::Degraded, continue issuing but emit warning; when Unhealthy(recoverable=true), stop issuing and attempt recovery in `crates/resource/src/manager.rs`
- [X] T059 [US5] Implement `QuarantineManager` with `QuarantineEntry`, `QuarantineReason`, configurable threshold, max_recovery_attempts in `crates/resource/src/quarantine.rs` (new file)
- [X] T060 [US5] Implement `RecoveryStrategy` trait with exponential backoff (1s, 2s, 4s... max 60s) in `crates/resource/src/quarantine.rs`
- [X] T061 [US5] Implement dependency health cascade — when resource becomes Unhealthy, propagate Degraded to all dependents via `DependencyGraph` in `crates/resource/src/manager.rs`
- [X] T062 [US5] Integrate `QuarantineManager` into `Manager` — automatic quarantine on threshold, recovery task scheduling, manual release in `crates/resource/src/manager.rs`
- [X] T063 [US5] Export `quarantine` module from `crates/resource/src/lib.rs`
- [X] T064 [US5] Run quality gates, verify all quarantine and cascade tests pass

**Checkpoint**: US5 complete — automatic quarantine, backoff recovery, cascade detection operational

---

## Phase 8: User Story 6 — Developer Extends Lifecycle with Custom Logic (Priority: P6)

**Goal**: Developers register hooks that fire before/after lifecycle events without modifying driver code.

**Independent Test**: Register a before-acquire hook, acquire resource, verify hook fired in correct order.

### Tests for User Story 6

- [X] T065 [P] [US6] Write test: hooks execute in priority order (lower = earlier) in `crates/resource/tests/hooks_integration.rs`
- [X] T066 [P] [US6] Write test: before-hook returning Err cancels the operation in `crates/resource/tests/hooks_integration.rs`
- [X] T067 [P] [US6] Write test: HookFilter scopes hooks to specific resource types in `crates/resource/tests/hooks_integration.rs`
- [X] T068 [P] [US6] Write test: after-hook errors are logged but do not affect the operation in `crates/resource/tests/hooks_integration.rs`

### Implementation for User Story 6

- [X] T069 [P] [US6] Implement `ResourceHook` trait, `HookEvent` enum, `HookFilter` enum, `HookResult` enum per contracts/lifecycle-hooks.md in `crates/resource/src/hooks.rs` (new file)
- [X] T070 [US6] Implement `HookRegistry` with priority-sorted insertion, `run_before()`, `run_after()` methods in `crates/resource/src/hooks.rs`
- [X] T071 [US6] Integrate `HookRegistry` into `Manager` — call `run_before()`/`run_after()` around acquire, release, create, cleanup operations in `crates/resource/src/manager.rs`
- [X] T072 [P] [US6] Implement built-in `AuditHook` (priority 10) — logs all operations via `tracing::info!` in `crates/resource/src/hooks.rs`
- [X] T073 [P] [US6] Implement built-in `SlowAcquireHook` (priority 90) — warns if acquire wait > threshold in `crates/resource/src/hooks.rs`
- [X] T074 [US6] Export `hooks` module from `crates/resource/src/lib.rs`
- [X] T075 [US6] Run quality gates, verify all hook tests pass

**Checkpoint**: US6 complete — extensible lifecycle hooks with priority ordering and filtering

---

## Phase 9: User Story 7 — Operator Auto-Scales Pools Under Load (Priority: P7)

**Goal**: Pools automatically grow under sustained high load and shrink under sustained low load.

**Independent Test**: Simulate high utilization, verify pool grows; simulate low utilization, verify pool shrinks.

### Tests for User Story 7

- [X] T076 [P] [US7] Write test: pool scales up when utilization > high_watermark for window duration in `crates/resource/tests/autoscale_integration.rs`
- [X] T077 [P] [US7] Write test: pool scales down when utilization < low_watermark for window duration in `crates/resource/tests/autoscale_integration.rs`
- [X] T078 [P] [US7] Write test: auto-scaler respects min_size/max_size bounds in `crates/resource/tests/autoscale_integration.rs`
- [X] T079 [P] [US7] Write test: config hot-reload creates new pool and drains old in `crates/resource/tests/hotreload_integration.rs`

### Implementation for User Story 7

- [X] T080 [P] [US7] Implement `AutoScalePolicy` struct with validation (watermarks in 0.0-1.0, min <= max, positive steps) in `crates/resource/src/autoscale.rs` (new file)
- [X] T081 [US7] Implement auto-scaler background task — periodically evaluate utilization, scale up/down within bounds in `crates/resource/src/autoscale.rs`
- [X] T082 [US7] Integrate auto-scaler with `Pool` — auto-scaler reads `PoolStats`, calls internal scale methods in `crates/resource/src/pool.rs`
- [X] T083 [US7] Implement `Manager::reload_config()` — create new pool with new config, drain old, swap atomically in `crates/resource/src/manager.rs`
- [X] T084 [US7] Export `autoscale` module from `crates/resource/src/lib.rs`
- [X] T085 [US7] Run quality gates, verify auto-scale and hot-reload tests pass

**Checkpoint**: US7 complete — pools auto-scale under load, config hot-reload without downtime

---

## Phase 10: Polish & Cross-Cutting Concerns

**Purpose**: Documentation, final cleanup, and cross-cutting improvements

- [X] T086 [P] Write `crates/resource/docs/Architecture.md` — overall architecture with diagrams matching actual code
- [X] T087 [P] Write `crates/resource/docs/HealthChecks.md` — health pipeline setup, stages, degraded handling
- [X] T088 [P] Write `crates/resource/docs/Integration.md` — how action/engine integrate with resource framework
- [X] T089 Ensure all pub items have rustdoc — run `cargo doc --no-deps -p nebula-resource` and fix any missing docs
- [X] T090 Run full CI pipeline: `cargo fmt --all -- --check`, `cargo clippy --workspace -- -D warnings`, `cargo check --workspace --all-targets`, `cargo test --workspace`, `cargo doc --no-deps --workspace`, `cargo audit`
- [X] T091 Run quickstart.md validation — verify all code snippets compile and match actual API

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)**: No dependencies — start immediately
- **Phase 2 (Foundational)**: Depends on Phase 1 — BLOCKS all user stories
- **Phase 3 (US1)**: Depends on Phase 2 — core acquire/release hardening
- **Phase 4 (US2)**: Depends on Phase 2 — can run in parallel with US1 (different files: events.rs, metrics.rs)
- **Phase 5 (US3)**: Depends on Phase 3 (US1) — docs/examples reference hardened API
- **Phase 6 (US4)**: Depends on Phase 2 — can run in parallel with US1/US2
- **Phase 7 (US5)**: Depends on Phase 4 (US2) events + Phase 2 health integration
- **Phase 8 (US6)**: Depends on Phase 2 — hooks.rs is a new standalone module
- **Phase 9 (US7)**: Depends on Phase 4 (US2) events + Phase 3 (US1) pool improvements
- **Phase 10 (Polish)**: Depends on all previous phases

### User Story Dependencies

```
         ┌─── US1 (P1) ──── US3 (P3) ──── US7 (P7)
         │                                   │
Phase 2 ─┼─── US2 (P2) ──── US5 (P5)        │
         │                                   │
         ├─── US4 (P4)                       │
         │                                   │
         └─── US6 (P6)                       │
                                             ▼
                                        Phase 10
```

### Parallel Opportunities

**After Phase 2 completes, these can run in parallel:**
- US1 (pool hardening in pool.rs, manager.rs tracing)
- US2 (new files: events.rs, metrics.rs)
- US4 (scope additions to manager.rs — different methods)
- US6 (new file: hooks.rs)

**Within each user story, tasks marked [P] can run in parallel.**

---

## Parallel Example: Phase 4 (US2 — Observability)

```
# These can all run in parallel (different files):
Task T028: "Implement ResourceEvent enum in crates/resource/src/events.rs"
Task T029: "Implement EventBus in crates/resource/src/events.rs"
Task T033: "Implement MetricsCollector in crates/resource/src/metrics.rs"

# Then sequentially (modify existing files):
Task T030: "Integrate EventBus into Manager"
Task T031: "Integrate EventBus into Pool"
Task T032: "Integrate EventBus into HealthChecker"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup & Cleanup
2. Complete Phase 2: Foundational improvements (scope validation, health integration, phased shutdown, credentials)
3. Complete Phase 3: US1 — hardened acquire/release with LIFO, tracing, engine integration test
4. **STOP and VALIDATE**: Run full test suite, verify all US1 acceptance scenarios
5. This MVP delivers: production-hardened resource management with proper scope validation, health-aware pooling, phased shutdown

### Incremental Delivery

1. Setup + Foundational → Clean, correct codebase
2. US1 → Hardened core lifecycle (MVP)
3. US2 → Observable system (events, metrics, logging)
4. US3 → Developer-friendly (docs, examples, benchmarks)
5. US4 → Multi-tenant isolation
6. US5 → Enterprise resilience (quarantine, cascade)
7. US6 → Extensible hooks
8. US7 → Auto-scaling and hot-reload
9. Polish → Documentation and final quality pass

Each increment is independently shippable and builds on the previous.

---

## Notes

- Existing code (~3,500 LOC, 150+ tests) is the foundation — tasks improve and extend it
- All tasks follow TDD: test tasks precede implementation tasks within each user story
- [P] tasks = different files, no dependencies — safe for parallel execution
- Quality gate tasks (T007, T014, T023, etc.) must pass before moving to next phase
- The user specifically requested "improve and do it properly" — foundational phase addresses the audit gaps
