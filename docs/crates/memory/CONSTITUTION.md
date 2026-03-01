# nebula-memory Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Nebula is a workflow automation platform. When a workflow runs "Process CSV → Split 10k rows
→ call OpenAI per row → write to Postgres", each of those 10 000 parallel action executions
needs to allocate buffers, cache intermediate results, and then release memory the moment the
execution completes — not when the GC decides to.

**nebula-memory is the memory discipline layer of the Nebula platform.**

It answers one question: *How does a workflow engine that runs hundreds of concurrent executions
avoid OOM kills, memory leaks, and cache invalidation bugs — without forcing every action
developer to think about allocation?*

```
workflow engine starts execution "exec-42"
    ↓
memory crate provides scoped arena for exec-42
    ↓
actions in exec-42 allocate from arena: zero per-allocation overhead
    ↓
expression evaluator caches computed values keyed by exec-42
    ↓
exec-42 completes → arena.reset() → all allocations freed in O(1)
    ↓
cache entries scoped to exec-42 are evicted automatically
    ↓
zero memory leaks by construction, no per-object GC required
```

This is the memory contract. It is guaranteed by scope, not by convention.

---

## User Stories

### Story 1 — Workflow Engine Bounds Memory Under Load (P1)

A workflow engine runs 200 concurrent executions. Without memory control, a single
runaway execution processing a 500 MB CSV can OOM the node. The engine needs to give
each execution a bounded memory budget and shed load when system pressure rises.

**Acceptance**:
- `MemoryBudget::reserve(bytes)` enforces per-execution limits
- `MemoryMonitor` emits `PressureAction::Critical` when system memory drops below threshold
- Engine receives pressure event and stops admitting new executions
- No OOM kill — just back-pressure at admission
- Budget exceeded → `BudgetExceeded` error, execution fails cleanly, not the node

### Story 2 — Action Developer Caches Expensive Computation (P1)

A developer writing a "Parse CSV" action needs to cache parsed schemas to avoid
re-parsing the same file structure on every row. They should not write LRU logic —
just store and retrieve.

**Acceptance**:
```rust
let schema = cache.get_or_compute(file_hash, || parse_schema(&raw)).await?;
// Second call: O(1) lookup, no parse
// Cache evicted when execution scope ends — no stale data across executions
```
No knowledge of eviction policy, TTL, or concurrency required.

### Story 3 — Expression Evaluator Reuses Computation Results (P1)

The expression evaluator computes `{{ item.price * 1.2 }}` on 10 000 rows in a split
execution. Recomputing from scratch per row is 10 000x overhead. Results for
identical expressions within a workflow run should be cached at the workflow scope.

**Acceptance**:
- `ComputeCache` with workflow-scoped key caches expression results
- Cache hit: return in < 1µs
- Cache evicted when workflow run completes — no cross-run leakage
- Expression cache is separate from execution-scoped buffer cache

### Story 4 — Object Pool Eliminates Hot-Path Allocation (P2)

A high-frequency trigger processes 50 000 webhook events per minute. Each event
allocates a `RequestBuffer`. Without pooling, this is 50 000 malloc/free cycles per
minute. The platform needs RAII-safe object reuse with zero-overhead return.

**Acceptance**:
- `ObjectPool::get()` returns a `PooledValue<T>` that returns item on `Drop`
- Pool pre-warms to configured size at startup
- `PoolExhausted` → retryable error — caller can wait or shed
- No manual `pool.return(item)` — RAII handles it

### Story 5 — Platform Operator Observes Memory Pressure (P2)

An operator deploying Nebula on a 4 GB node needs to understand when the platform
is approaching memory limits, which workflows are consuming the most, and what
pressure actions were taken in the last 24 hours.

**Acceptance**:
- `MemoryMonitor` emits pressure transitions: Normal → Warning → Critical → Emergency
- `StatsCollector` provides `MemoryStats` with per-pool and per-budget breakdowns
- Metrics exposed via `extensions` feature (Prometheus-compatible)
- Operator can tune pressure thresholds without code changes — config only

---

## Core Principles

### I. Scopes are the Garbage Collector

**Memory lifetime is determined by execution scope, not by reference counting or GC.
When a scope ends, all memory allocated within it is freed — in O(1), regardless of count.**

**Rationale**: A workflow execution is a bounded unit of work. It starts, it runs, it ends.
Every buffer, every cache entry, every intermediate result has the same lifetime as the
execution that produced it. Tying memory to scope means zero memory leaks by construction.
Arenas make this O(1): one pointer reset frees 10 000 allocations instantly.

**Rules**:
- Arenas MUST be scoped to at minimum execution granularity
- `Arena::reset()` MUST free all items in one operation — no per-item traversal
- Cache entries MUST be evicatable by scope key — not just by size/TTL
- MUST NOT silently promote memory from a narrow scope to a wider one

### II. Reuse Before Allocate

**The first optimization is always object reuse. Allocation is the last resort.**

**Rationale**: Workflow engines are allocation-heavy by nature — every data transform creates
new values. But many objects have the same structure and can be reused: request buffers,
JSON builders, scratch vectors. ObjectPool makes reuse the default path.
Cache makes recomputation the exception.

**Rules**:
- `ObjectPool` is the primary path for hot-path object creation
- `ComputeCache::get_or_compute` is preferred over raw compute + manual cache insert
- Reuse primitives MUST return items via RAII Drop — never require explicit `return()`
- Pool exhaustion MUST produce a retryable error — never silently allocate outside pool

### III. Pressure is Observable and Actionable

**Memory pressure is a first-class signal. It MUST be observable, it MUST be typed,
and it MUST cause explicit platform action — not silent degradation.**

**Rationale**: OOM kills are the worst possible outcome for a workflow automation platform.
By the time the OOM killer runs, data is lost and users are confused. The right response
to memory pressure is early, explicit, and graduated: throttle admission at Warning,
shed load at Critical, emergency cleanup at Emergency.

**Rules**:
- `PressureAction` MUST have named variants: Normal / Warning / Critical / Emergency
- Engine/runtime MUST subscribe to pressure events and take explicit action
- `MemoryMonitor` MUST emit transitions — not just current state on poll
- MUST NOT silently reduce pool sizes or evict caches without emitting pressure events

### IV. Observability is Optional, Correctness is Not

**Stats, metrics, and monitoring are feature-gated. Allocation, pooling, and budgeting
are always correct whether features are enabled or not.**

**Rationale**: Production deployments need metrics. Test environments and embedded deployments
do not want the overhead. Feature flags let consumers opt in to observability without
changing the memory correctness contract.

**Rules**:
- `stats`, `monitoring`, `logging`, `metrics` MUST compile out cleanly when disabled
- Pool exhaustion and budget exceeded errors MUST surface even without `stats` feature
- Adding observability MUST NOT change the behavioral contract of any primitive
- Each feature flag MUST have a documented binary: included / excluded — no partial features

### V. Corruption is Terminal

**`MemoryError::Corruption` is a process-level event. It MUST never be retried, swallowed,
or treated as a transient failure.**

**Rationale**: A corrupted allocator or pool means the process state is undefined. Continuing
to execute workflows against a corrupted memory subsystem will produce wrong results silently.
The safe action is immediate escalation to the operator — not retry.

**Rules**:
- `Corruption` variant MUST be `#[non_exhaustive]` marker for operator escalation
- Callers MUST NOT match `Corruption` with `is_retryable` — it is always fatal
- `MemoryError` MUST be `#[non_exhaustive]` — downstream match arms must use `_`
- Any unsafe code boundary MUST document its invariants and panic on violation

---

## Production Vision

### The workflow execution memory model

In a production Nebula deployment, memory management is invisible to action developers
but critical to platform stability:

```
WorkflowRun "wf-42"
    │
    ├── Arena[workflow-scope]: workflow-wide buffers (schema cache, lookup tables)
    │
    ├── Execution[exec-101]
    │       ├── Arena[exec-scope]: all buffers for this execution
    │       ├── Budget[exec-101]: max 256 MB, reject at 90%
    │       └── ExpressionCache[exec-scope]: computed expression results
    │
    └── Execution[exec-102]
            ├── Arena[exec-scope]: independent arena, no sharing with exec-101
            └── Budget[exec-102]: separate budget, no cross-execution accounting
```

When exec-101 completes:
- `arena.reset()` → O(1) free of all exec-101 allocations
- `budget.release()` → budget returns to pool for next execution
- Cache entries tagged with exec-101 evicted in next GC cycle

### From the archives: TieredMemoryCache

The archive `archive-overview.md` describes a richer cache design:

```
TieredMemoryCache
    │
    ├── L1 hot: LRU in-memory, sub-microsecond access
    ├── L2 warm: BTreeMap with background eviction
    ├── L3 external: Redis for shared-fleet expression results
    └── ExpressionResultCache: dedicated hot path for expression evaluator
```

Current implementation has the cache primitives. The path to production requires:
- Scope-aware eviction keys (`(scope_id, cache_key)`)
- L3 Redis integration as an optional feature
- Expression-specific cache with workflow-scoped TTL

### Policy-driven memory strategy (archive P002 insight)

The archive proposes routing allocations through a policy:
```rust
enum AllocationPolicy {
    LatencyOptimized,   // prefer pool reuse, minimize contention
    ReuseOptimized,     // maximize pool/arena utilization
    ThroughputOptimized, // maximize allocation rate, accept fragmentation
}
```
Engine can switch policy per execution type without changing action code.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|---------|-------|
| Scope-aware cache eviction by execution/workflow ID | Critical | Prevents cross-execution cache hits |
| Budget-to-backpressure integration | High | Connect `BudgetExceeded` to scheduler admission |
| L3 Redis expression cache | Medium | Shared across fleet for hot workflows |
| Unified `MemoryRuntimeConfig` | Medium | Simplify engine bootstrap (archive P001) |
| NUMA-aware arena allocation | Low | Performance on multi-socket servers |

---

## Key Decisions

### D-001: Multi-Paradigm Toolkit in One Crate

**Decision**: Allocators, object pools, cache, and budgeting live in one crate with
feature flags rather than as separate crates.

**Rationale**: Runtime consumers often need a combined strategy: a pool for hot-path objects,
an arena for execution buffers, and a cache for expression results. One crate reduces
integration friction.

**Rejected**: Separate `nebula-pool`, `nebula-cache`, `nebula-budget` crates — import
complexity and version skew without a clear boundary between them.

### D-002: Feature-Gated Modularity

**Decision**: `pool`, `arena`, `cache`, `stats`, `budget`, `async`, `monitoring` are
independent feature flags — none required for compilation.

**Rationale**: Embedded or test deployments should not pay for Redis clients or monitoring
overhead they don't use. Feature flags make the baseline zero-cost.

**Rejected**: Single monolithic feature set — forces all consumers to compile all dependencies.

### D-003: Reuse-First, Allocation-Last

**Decision**: The primary optimization target is object reuse (`ObjectPool`, `Arena::reset`,
cache hits), not raw allocation throughput.

**Rationale**: For workflow automation workloads, most allocations are for the same object
shapes (request buffers, JSON trees, scratch space). Reuse beats raw allocation speed.

**Rejected**: Global arena per-process — too coarse, no scope isolation, no per-execution limits.

### D-004: Corruption is Always Fatal

**Decision**: `MemoryError::Corruption` is a terminal error with no retry semantics.
It is documented as requiring operator escalation, not caller recovery.

**Rationale**: Corrupt memory state means all subsequent operations are undefined.
Retrying would compound the problem.

**Rejected**: Returning `None` from corrupted pools (callers might treat as empty-but-healthy).

---

## Open Proposals

### P-001: Unified `MemoryRuntimeConfig`

**Problem**: Each module (pool, cache, budget) has its own config type. Engine bootstrap
must configure each separately.

**Proposal**: `MemoryRuntimeConfig` that composes all module configs, with builder API
for common presets (`single_node_small`, `fleet_standard`, `embedded`).

**Impact**: Engine/runtime initialization simplifies. Module configs remain unchanged as inner fields.

### P-002: Budget-to-Backpressure Integration

**Problem**: `BudgetExceeded` errors are returned to callers but not connected to the
scheduler's admission control. Engine can keep admitting new executions even when
existing ones are over budget.

**Proposal**: `MemoryBudget` emits a pressure channel that `runtime/scheduler` subscribes
to. Budget pressure → reduce admission rate. Budget critical → halt new executions.

**Dependency**: Requires Tokio watch channel integration in budget module.

### P-003: Scope-Aware Cache Eviction

**Problem**: Current cache eviction is size/TTL-based. Cache entries from completed
executions may remain in L1 until evicted by pressure — potentially serving stale data.

**Proposal**: Cache keys include `scope_id`. On execution complete, caller calls
`cache.evict_scope(exec_id)` → O(scope_size) cleanup.

**Impact**: No breaking API change — additive `evict_scope` method.

---

## Non-Negotiables

1. **`Arena::reset()` is O(1)** — bulk reset, never per-item traversal
2. **`PooledValue` returns item on Drop** — no manual `pool.return()` in public API
3. **`MemoryError::Corruption` is terminal** — never matched as retryable
4. **Feature flags are clean boundaries** — disabled features compile out completely
5. **No cross-scope cache hits** — cache key MUST include scope identifier
6. **`MemoryError` is `#[non_exhaustive]`** — downstream MUST use `_` match arms

---

## Governance

Amendments require:
- PATCH: wording — PR with explanation
- MINOR: new principle, feature flag, or non-negotiable — review with note in DECISIONS.md
- MAJOR: removing scope isolation guarantee, changing corruption semantics, or adding
  mandatory dependencies to always-on features — full architecture review required

All PRs must verify:
- No cross-scope cache hits possible with new changes
- Corruption error path untouched
- Feature off-paths still compile cleanly
