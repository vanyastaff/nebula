# nebula-system Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Nebula is a workflow automation platform — a Rust-native n8n.
Workflows run on nodes. Nodes execute actions. Actions consume resources.

**nebula-system is the sensing layer of the platform.**

Without it, the engine is blind:
- It cannot throttle when the host is running out of memory
- It cannot alert operators when disk is full
- It cannot make autoscaling decisions based on CPU saturation
- It cannot detect when the platform is under pressure and needs to shed load

Every other crate that cares about operational health depends on nebula-system
as its source of ground truth about the machine it runs on.

```
workflow engine
  ├── nebula-memory  ←  "how much memory is available?"
  ├── nebula-resource ← "should I scale up the pool?"
  ├── nebula-resilience ← "is the system degraded? apply pressure policy"
  └── platform monitoring → "emit metrics to Prometheus / alert Grafana"
                ↑
          nebula-system (sensing layer)
```

---

## User Stories

### Story 1 — Workflow Engine (P1)

The workflow engine needs to decide whether to start a new workflow execution
or queue it. It should not start work when the host is under critical memory
or CPU pressure — that leads to OOM kills and cascading failures.

**Acceptance**: Given `memory::pressure()` returns `Critical`, the engine queues
new executions until pressure drops. Given `cpu::pressure()` returns `High`,
the engine applies a configurable throttle multiplier.

### Story 2 — nebula-memory (P1)

The memory management crate monitors system-level pressure to trigger cleanup,
pool eviction, and emergency GC. It cannot call sysinfo directly — that would
create a dependency mess. It uses nebula-system as the single source of truth.

**Acceptance**: `nebula-memory::monitoring::PressureAction` is derived from
`nebula-system::memory::pressure()`. No direct sysinfo dependency in nebula-memory.

### Story 3 — Platform Operator (P2)

An operator running Nebula in production wants system metrics flowing into
their existing Prometheus + Grafana setup. They want to alert on:
- memory usage > 80%
- disk usage > 90%
- CPU sustained > 90% for > 60 seconds

**Acceptance**: When the `metrics` feature is enabled, nebula-system emits
counter/gauge/histogram metrics using the `metrics` facade. Operator plugs
in their preferred backend (Prometheus, StatsD, etc.).

### Story 4 — Action Developer (P3)

A developer writing a compute-intensive action (e.g., image processing,
data transformation) wants to check available memory before allocating a
large buffer. They should not need to call sysinfo directly.

**Acceptance**: `nebula_system::memory::current()` returns `MemoryInfo` with
`available` bytes. Action checks `available > required` before allocating.

### Story 5 — Resource Pool Autoscaler (P3)

nebula-resource autoscaling uses CPU and memory metrics to decide when to
grow or shrink connection pools. It needs fresh readings on demand, not stale
cached values.

**Acceptance**: `SystemInfo::refresh()` returns fresh data. Autoscaler calls
refresh before making scale decisions. Cached `SystemInfo::get()` is used
for read-heavy non-critical paths.

---

## Core Principles

### I. Read-Only, Sync-First

**nebula-system is a read-only, synchronous sensing crate.**

It reads from the OS. It does not write, schedule, or orchestrate.
Async wrappers belong in consumers, not here.

**Rationale**: The moment nebula-system starts Tokio tasks, it acquires a
dependency on the Tokio runtime version and couples its lifecycle to the engine.
Keeping it sync means it can be used from any context — sync tests, CLI tools,
embedding — without ceremony.

**Rules**:
- MUST NOT spawn background tasks or threads internally
- MUST NOT depend on tokio (dev-deps permitted for tests)
- MAY expose `#[cfg(feature = "async")]` wrappers that consumers opt into
- MUST be callable from sync and async contexts without deadlock risk

### II. Feature Flags are the API

**Every optional capability is a feature flag. The baseline is minimal.**

The default feature set (`memory`, `sysinfo`) covers 95% of use cases.
Disk, network, process, component, and metrics are opt-in.

**Rationale**: A workflow automation platform runs in diverse environments —
containers with no disk access, sandboxed environments, minimal cloud VMs.
Feature flags let operators compile exactly what they need. This is not a
convenience feature — it is a security and attack-surface reduction principle.

**Rules**:
- MUST NOT enable non-default features transitively in workspace dependents
- MUST document required OS privileges per feature
- MUST compile cleanly with any subset of features
- MUST NOT use `#[cfg(feature = "...")]` inside function bodies for logic
  branches — instead, gate entire modules

### III. Pressure is a First-Class Type

**`MemoryPressure`, `CpuPressure`, `DiskPressure` are not strings or enums
you figure out from percentages. They are typed signals with actionable meaning.**

```
Low       → normal operation
Medium    → watch, no action
High      → throttle new work, start GC hints
Critical  → emergency: shed load, alert operator, stop accepting new work
```

**Rationale**: Every consumer of pressure signals needs to make a decision.
If pressure is just a number, every consumer must implement thresholds.
If pressure is a typed enum, the decision is clear and consistent across
the entire platform. This is platform-level standardization.

**Rules**:
- MUST implement `is_concerning()` → true for High/Critical
- MUST implement `is_critical()` → true for Critical only
- MUST include threshold documentation in the type (not just in docs)
- MUST be `Copy + Eq + Ord + Debug + Serialize`

### IV. Caching with Explicit Staleness

**Cached reads are explicit. Fresh reads are explicit. Nothing is implicit.**

`SystemInfo::get()` returns the cached snapshot — cheap, but potentially stale.
`SystemInfo::refresh()` returns fresh data from sysinfo — costs ~1ms per call.
`memory::current()` is always fresh — direct OS call, not cached.

**Rationale**: Callers need to understand the freshness trade-off. Hidden
auto-refresh creates unpredictable latency spikes. In a hot workflow execution
path, a stale 100ms-old CPU reading is fine. In an autoscaler making a scale
decision, it needs fresh data.

**Rules**:
- MUST document cache lifetime for every cached API
- MUST NOT auto-refresh in background
- MUST expose explicit refresh API for consumers that need fresh data
- cached `SystemInfo` snapshot SHOULD be timestamped

### V. Graceful Degradation over Panic

**When system info is unavailable, degrade gracefully — never panic.**

Permission denied → return `Err(SystemError::PermissionDenied)`.
Feature not compiled → return `Err(SystemError::FeatureNotSupported)`.
Parse error → log warning, return `None` or default.

**Rationale**: In a production workflow engine, a permission error reading
disk usage should not crash the workflow. It should alert the operator and
degrade the monitoring path. The automation must continue.

**Rules**:
- MUST NOT `unwrap()` or `expect()` on system calls in library code
- MUST return `SystemResult<T>` for all fallible operations
- MUST treat `PermissionDenied` as non-retryable
- MUST NOT remove features from returned types when unavailable — use `Option`

---

## Production Vision

### What prod looks like

A production Nebula deployment runs with:

1. **Pressure events** — nebula-system emits pressure change events on a
   Tokio broadcast channel (opt-in feature). Engine subscribes and reacts:
   - `Critical` → stop accepting new executions, drain gracefully
   - `High` → apply throttle, emit alert metric
   - Recovery to `Low` → resume normal operation

2. **Metrics integration** — `metrics` feature enabled, wired to Prometheus:
   ```
   nebula_system_memory_usage_bytes{host="node-1"}
   nebula_system_memory_pressure{host="node-1", level="high"}
   nebula_system_cpu_usage_percent{host="node-1", core="all"}
   nebula_system_disk_usage_percent{host="node-1", path="/"}
   ```

3. **NUMA-aware scheduling** (Phase 3) — For multi-socket servers running
   large workflow batches, CPU topology informs thread pool placement to
   minimize cross-NUMA memory access.

4. **Async refresh loop** (Phase 4) — An optional managed `SystemMonitor`
   spawns a configurable refresh loop (default: every 5s) and exposes
   a `subscribe()` method for pressure change events. Consumers don't poll.

5. **Composable health checks** (Phase 4) — From the archives: `legacy-crates-dependencies.md`
   and system design notes. A `HealthChecker` builder that composes system checks:
   ```rust
   let health = HealthChecker::new()
       .memory_threshold(0.80)          // alert at 80% usage
       .disk_threshold("/", 0.90)       // alert at 90% disk
       .cpu_threshold(0.90, 60s);       // alert if sustained 90% for 60s

   let status = health.check().await;
   ```

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|---------|-------|
| Pressure events via broadcast channel | High | nebula-memory and engine need push, not poll |
| Prometheus/OTel metrics emission | High | Operators require observability integration |
| `SystemMonitor` with managed refresh loop | Medium | Removes polling boilerplate from consumers |
| Composable `HealthChecker` builder | Medium | From archive: strong design, easy to implement |
| NUMA topology | Low | High value for large servers, low priority now |
| Async wrappers | Low | Consumers can wrap sync calls today |

---

## Key Decisions

### D-001: sysinfo as the Backend

**Decision**: Use `sysinfo` crate as the cross-platform backend.

**Rationale**: sysinfo covers Linux/macOS/Windows with a unified API.
The alternatives (raw `/proc` parsing, platform syscalls) are higher
maintenance burden for the same cross-platform result.

**Trade-off accepted**: sysinfo's API may change between versions,
requiring adaptation. This is an acceptable cost for cross-platform coverage.

**Rejected**: `heim` (heavier, async-only), raw `/proc` (Linux-only),
custom sysctl (macOS-only).

### D-002: Sync-Only Core, Async Optional

**Decision**: Core APIs are sync. Async wrappers are a future feature flag.

**Rationale**: sysinfo's `refresh_*` calls block briefly (~1ms). Wrapping in
`tokio::task::spawn_blocking` is callers' responsibility — they know their
async context. Forcing tokio dependency into the crate would be overreach.

**Rejected**: Making `SystemInfo::refresh()` async-native would require tokio
as a hard dep and break sync use cases.

### D-003: `LazyLock` for Global Init

**Decision**: Use `std::sync::LazyLock` for `SYSINFO_SYSTEM` and
`SYSTEM_INFO_CACHE`. `init()` forces initialization; subsequent calls are no-ops.

**Rationale**: Global lazy initialization is idiomatic for system-wide singletons.
`parking_lot::RwLock` wraps the mutable sysinfo system for thread-safe refresh.

### D-004: Pressure Thresholds as Named Constants

**Decision**: Pressure thresholds are named constants, not configuration.

```rust
const MEMORY_HIGH_THRESHOLD: f64 = 0.70;    // 70%
const MEMORY_CRITICAL_THRESHOLD: f64 = 0.85; // 85%
```

**Rationale**: Configurable thresholds require a config struct, which
requires init ordering, which couples nebula-system to nebula-config.
Named constants are simple, visible, and sufficient.
Operators who need custom thresholds implement their own pressure
classification on top of the raw `MemoryInfo` values.

**Future**: If config-driven thresholds are needed (P-001), they can be
added as a builder pattern without breaking the default constants path.

---

## Open Proposals

### P-001: Config-Driven Pressure Thresholds

**Problem**: Different workloads (memory-intensive ML, I/O-heavy ETL, CPU-bound
processing) need different pressure thresholds. Hardcoded constants don't serve
all deployments.

**Proposal**: Optional `PressureConfig` struct with builder:
```rust
let pressure = memory::pressure_with_config(&PressureConfig {
    high: 0.75,
    critical: 0.90,
});
```

**Impact**: Non-breaking (additive API). Default constants unchanged.

### P-002: Push-Based Pressure Events

**Problem**: nebula-memory and the engine poll pressure. Polling wastes CPU and
adds latency between pressure change and reaction.

**Proposal**: `SystemMonitor::subscribe()` returns `broadcast::Receiver<PressureEvent>`.
Background task (spawned by caller) polls on configurable interval and emits
on change.

**Impact**: Opt-in via feature flag. No change to sync polling API.

### P-003: Composable `HealthChecker`

**Problem**: Operators want composable health checks (memory + disk + CPU)
as a unified boolean/status signal for `/health` endpoint and k8s probes.

**Proposal** (from archive `legacy-business-cross.md`):
```rust
let health = HealthChecker::new()
    .add_check(MemoryHealthCheck { threshold: 0.80 })
    .add_check(DiskSpaceHealthCheck { path: "/", threshold: 0.90 })
    .add_check(CpuHealthCheck { threshold: 0.90, window: Duration::from_secs(60) });

let status: HealthStatus = health.check();
// HealthStatus: Healthy | Degraded | Unhealthy
```

**Impact**: Additive. Replaces per-consumer ad-hoc health logic with
a shared, tested primitive.

---

## Non-Negotiables

1. **No tokio runtime dependency in core** — ever. Feature-flag only.
2. **No panics in library code** — all fallible operations return `SystemResult<T>`.
3. **Pressure types are `Copy + Ord + Eq`** — they must be usable in match arms without cloning.
4. **Feature flags must be additive** — disabling a feature must never break code that doesn't use it.
5. **`SystemInfo::get()` must complete in < 1µs** — it's called on hot paths.
6. **`memory::pressure()` must complete in < 1ms** — it's called before allocation decisions.

---

## Governance

Amendments to this constitution require:
- A PATCH: wording/clarification — PR with explanation
- A MINOR: new principle or new non-negotiable — review with a note in DECISIONS.md
- A MAJOR: removing a non-negotiable or reversing a core principle — full design review

All PRs to nebula-system must verify compliance with principles I–V before merge.
