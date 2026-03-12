# Plan: Cross-Cutting Crates — Tier 1 Critical Fixes

- **Branch:** `chore/cross-cutting-tier1-fixes`
- **Created:** 2026-03-12
- **Type:** chore

## Settings

- **Testing:** yes
- **Logging:** verbose
- **Docs:** yes

## Roadmap Linkage

- **Milestones:**
  - "EventBus: Lock Poisoning Recovery"
  - "EventBus: Add Prelude Module"
  - "Telemetry: Histogram OOM Fix"
  - "Telemetry: Add Prelude Module"
  - "Metrics: Add Prelude Module"
- **Rationale:** These 5 milestones are the Tier 1 blockers in `.ai-factory/ROADMAP.md` — critical
  fixes that must be resolved before any production deployment of cross-cutting crates.

## Context

### Problem

1. **Lock poisoning panics (14 sites):** `eventbus/registry.rs` has 8 `.expect("...lock poisoned")`
   calls and `telemetry/metrics.rs` has 6. If a panic occurs while holding a lock, subsequent
   accesses cascade-panic the entire application. Production systems must recover from poisoned locks.

2. **Histogram OOM:** `telemetry::Histogram` stores ALL observations in `Vec<f64>`. Under sustained
   load (millions of requests) this will exhaust memory. Production histograms need bounded storage
   with percentile support.

3. **Missing prelude modules (3 crates):** `nebula-eventbus`, `nebula-telemetry`, and `nebula-metrics`
   lack `pub mod prelude`, breaking workspace convention established by `nebula-validator`,
   `nebula-action`, `nebula-system`, and `nebula-sdk`.

### Affected Files

| Crate | File | Change Type |
|-------|------|-------------|
| `nebula-eventbus` | `crates/eventbus/src/registry.rs` | Edit — replace 8 `.expect()` with poison recovery |
| `nebula-eventbus` | `crates/eventbus/src/prelude.rs` | Create — new prelude module |
| `nebula-eventbus` | `crates/eventbus/src/lib.rs` | Edit — add `pub mod prelude` |
| `nebula-telemetry` | `crates/telemetry/src/metrics.rs` | Edit — replace Histogram + fix 6 `.expect()` |
| `nebula-telemetry` | `crates/telemetry/src/prelude.rs` | Create — new prelude module |
| `nebula-telemetry` | `crates/telemetry/src/lib.rs` | Edit — add `pub mod prelude` |
| `nebula-metrics` | `crates/metrics/src/prelude.rs` | Create — new prelude module |
| `nebula-metrics` | `crates/metrics/src/lib.rs` | Edit — add `pub mod prelude` |

---

## Tasks

### Phase 1 — Lock Poisoning Recovery

#### Task 1: EventBus Registry — Replace `.expect()` with Poison Recovery
- [x] Done

**File:** `crates/eventbus/src/registry.rs`

Replace all 8 `.expect("eventbus registry ... lock poisoned")` calls with
`.unwrap_or_else(|poisoned| poisoned.into_inner())` pattern. This recovers the inner
data from a poisoned lock instead of panicking.

**Specific sites (line numbers approximate):**
1. `get_or_create()` — read lock (line ~58)
2. `get_or_create()` — write lock (line ~65)
3. `get()` — read lock (line ~81)
4. `remove()` — write lock (line ~91)
5. `len()` — read lock (line ~101)
6. `clear()` — write lock (line ~112)
7. `prune_without_subscribers()` — write lock (line ~125)
8. `stats()` — read lock (line ~140)

**Pattern to apply:**
```rust
// BEFORE:
self.buses.read().expect("eventbus registry read lock poisoned")

// AFTER:
self.buses.read().unwrap_or_else(|poisoned| poisoned.into_inner())
```

**Logging:** Add `tracing::warn!("eventbus registry lock was poisoned, recovering")` on the
recovery path (inside the `unwrap_or_else` closure). Import `tracing` in the module.

**Tests:**
- Add test `poisoned_lock_recovery_does_not_panic` — spawn a thread that panics while holding
  the write lock, then verify the registry remains usable from another thread.
- Add test `concurrent_access_with_poisoned_lock` — after poisoning, verify `get_or_create()`,
  `get()`, `remove()`, `len()`, `stats()` all work correctly.

---

#### Task 2: Telemetry Metrics — Replace `.expect()` with Poison Recovery
- [x] Done

**File:** `crates/telemetry/src/metrics.rs`

Replace all 6 `.expect("...lock poisoned")` calls with poison recovery:

**In `Histogram`:**
1. `observe()` — write lock
2. `count()` — read lock
3. `sum()` — read lock

**In `MetricsRegistry`:**
4. `counter()` — write lock
5. `gauge()` — write lock
6. `histogram()` — write lock

**Same pattern as Task 1.** Add `tracing::warn!` on recovery path. Add `tracing` to
`nebula-telemetry` dependencies if not already present (it is — already in Cargo.toml).

**Tests:**
- Add test `histogram_recovers_from_poisoned_lock` — poison the RwLock, verify observe/count/sum
  still work.
- Add test `registry_recovers_from_poisoned_lock` — poison a registry lock, verify counter/gauge/
  histogram creation still works.

---

### Phase 2 — Histogram OOM Fix

#### Task 3: Replace Vec-based Histogram with Bounded Bucket Storage\n- [x] Done

**File:** `crates/telemetry/src/metrics.rs`

Replace the current `Histogram` implementation (`Arc<RwLock<Vec<f64>>>`) with a bounded
bucket-based histogram that:

1. **Uses fixed buckets** — Prometheus-style default boundaries:
   `[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, +Inf]`
2. **Tracks per-bucket counts** — `Vec<AtomicU64>` (one per boundary + `+Inf`)
3. **Tracks running sum** — `AtomicU64` storing bits of `f64` (via `f64::to_bits()`)
4. **Tracks total count** — `AtomicU64`
5. **Does NOT store individual observations** — constant memory O(buckets), not O(observations)

**New API (backward-compatible, additive):**
```rust
impl Histogram {
    pub fn new() -> Self                          // default Prometheus buckets
    pub fn with_buckets(boundaries: Vec<f64>) -> Self  // custom buckets
    pub fn observe(&self, value: f64)             // O(log n) bucket lookup
    pub fn count(&self) -> usize                  // unchanged API
    pub fn sum(&self) -> f64                      // unchanged API
    // NEW:
    pub fn buckets(&self) -> Vec<(f64, u64)>      // (upper_bound, cumulative_count)
    pub fn percentile(&self, p: f64) -> f64       // linear interpolation within bucket
}
```

**Implementation notes:**
- `observe()` uses binary search on boundaries → O(log n) not O(1)
- No `RwLock` needed — all storage is atomic. This is a performance win.
- `percentile()` uses linear interpolation within the bucket containing the target count.
  This is an approximation (same tradeoff as Prometheus).
- Bucket boundaries must be sorted, non-empty, and all positive (validated in constructor).

**Backward compatibility:**
- `new()`, `observe()`, `count()`, `sum()` — signatures unchanged
- `Default` impl unchanged
- Tests calling `.count()` and `.sum()` will work as before
- Remove the `Vec<f64>` warning doc comment ("Suitable for development...")

**Logging:** `tracing::debug!("histogram created with {} buckets", boundaries.len())`

**Tests:**
- `histogram_default_buckets` — verify 12 default Prometheus boundaries
- `histogram_custom_buckets` — verify custom boundaries
- `histogram_observe_updates_correct_bucket` — verify bucket placement
- `histogram_count_and_sum_accurate` — verify running totals
- `histogram_percentile_basic` — verify p50/p95/p99 approximation
- `histogram_percentile_empty` — returns 0.0 (or NaN) for empty histogram
- `histogram_percentile_single_observation` — edge case
- `histogram_constant_memory` — observe 1M values, verify memory doesn't grow
- `histogram_concurrent_observe` — 100 threads observing concurrently, verify no data loss
- `histogram_invalid_buckets_panics` — empty or unsorted boundaries panic in constructor

---

### Phase 3 — Prelude Modules

#### Task 4: Create EventBus Prelude Module
- [x] Done

**Files:**
- Create: `crates/eventbus/src/prelude.rs`
- Edit: `crates/eventbus/src/lib.rs` — add `pub mod prelude;`

**Prelude exports (follow `nebula-validator` / `nebula-action` pattern):**
```rust
//! Convenience re-exports for eventbus users.
//!
//! ```rust,ignore
//! use nebula_eventbus::prelude::*;
//! ```

pub use crate::EventBus;
pub use crate::EventBusRegistry;
pub use crate::EventBusRegistryStats;
pub use crate::EventFilter;
pub use crate::EventSubscriber;
pub use crate::FilteredSubscriber;
pub use crate::PublishOutcome;
pub use crate::ScopedEvent;
pub use crate::SubscriptionScope;
pub use crate::BackPressurePolicy;
pub use crate::EventBusStats;
pub use crate::Subscriber;
```

**Logging:** Not applicable (no runtime logic).

**Tests:**
- Add compile test: `use nebula_eventbus::prelude::*;` in a test function — verify all types
  resolve without ambiguity.

---

#### Task 5: Create Telemetry Prelude Module
- [x] Done

**Files:**
- Create: `crates/telemetry/src/prelude.rs`
- Edit: `crates/telemetry/src/lib.rs` — add `pub mod prelude;`

**Prelude exports:**
```rust
//! Convenience re-exports for telemetry users.
//!
//! ```rust,ignore
//! use nebula_telemetry::prelude::*;
//! ```

// ── Event Bus ───────────────────────────────────────────────────────────
pub use crate::event::{EventBus, EventSubscriber, ExecutionEvent, ScopedSubscriber};
pub use crate::EventFilter;
pub use crate::PublishOutcome;
pub use crate::ScopedEvent;
pub use crate::SubscriptionScope;

// ── Metrics ─────────────────────────────────────────────────────────────
pub use crate::metrics::{Counter, Gauge, Histogram, MetricsRegistry, NoopMetricsRegistry};

// ── Service ─────────────────────────────────────────────────────────────
pub use crate::service::{NoopTelemetry, TelemetryService};

// ── Trace ───────────────────────────────────────────────────────────────
pub use crate::trace::{
    CallBody, CallPayload, CallRecord, CallStatus, DropReason, NoopRecorder, Recorder,
    ResourceUsageRecord,
};
```

**Tests:**
- Compile test: `use nebula_telemetry::prelude::*;` — verify all types resolve.

---

#### Task 6: Create Metrics Prelude Module
- [x] Done

**Files:**
- Create: `crates/metrics/src/prelude.rs`
- Edit: `crates/metrics/src/lib.rs` — add `pub mod prelude;`

**Prelude exports:**
```rust
//! Convenience re-exports for metrics users.
//!
//! ```rust,ignore
//! use nebula_metrics::prelude::*;
//! ```

// ── Adapter ─────────────────────────────────────────────────────────────
pub use crate::adapter::TelemetryAdapter;

// ── Export ───────────────────────────────────────────────────────────────
pub use crate::export::prometheus::{PrometheusExporter, content_type, snapshot};

// ── Metric Types (from nebula-telemetry) ────────────────────────────────
pub use nebula_telemetry::metrics::{Counter, Gauge, Histogram, MetricsRegistry};
```

**Tests:**
- Compile test: `use nebula_metrics::prelude::*;` — verify all types resolve.

---

### Phase 4 — Verification & Documentation

#### Task 7: Run Full CI Suite Locally
- [x] Done

Run and verify all checks pass:
```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo check --workspace --all-targets
cargo test --workspace
cargo doc --no-deps --workspace
```

Fix any warnings, errors, or test failures introduced by Tasks 1–6.

---

#### Task 8: Update Crate Documentation
- [x] Done

**Files to update:**
- `crates/eventbus/src/lib.rs` — add `prelude` to module-level doc comment's "Core Types" list
- `crates/telemetry/src/lib.rs` — add `prelude` to module-level doc comment
- `crates/metrics/src/lib.rs` — add `prelude` to module-level doc comment
- `crates/telemetry/src/metrics.rs` — update `Histogram` doc comment to describe bucket-based
  storage (remove "Suitable for development..." caveat)

**Logging:** Not applicable.

---

## Commit Plan

### Commit 1 (after Tasks 1–2)
```
fix(eventbus,telemetry): replace lock poisoning panics with recovery

Replace all .expect("...lock poisoned") calls with
unwrap_or_else(|p| p.into_inner()) pattern across eventbus registry
and telemetry metrics. Adds tracing::warn on recovery path.
Includes tests for poisoned lock scenarios.
```

### Commit 2 (after Task 3)
```
feat(telemetry): replace Vec histogram with bounded bucket storage

Histogram now uses Prometheus-style bucket boundaries instead of
storing all observations in Vec<f64>. Constant memory usage
regardless of observation count. Adds percentile() and buckets()
methods. Backward-compatible API.
```

### Commit 3 (after Tasks 4–6)
```
feat(eventbus,telemetry,metrics): add prelude modules

Add pub mod prelude to eventbus, telemetry, and metrics crates,
following workspace convention from validator/action/system/sdk.
```

### Commit 4 (after Tasks 7–8)
```
docs(eventbus,telemetry,metrics): update crate docs for new features

Document prelude modules, updated histogram implementation, and
lock recovery behavior in crate-level doc comments.
```
