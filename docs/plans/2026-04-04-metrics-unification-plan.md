# Metrics Unification Implementation Plan

**Goal:** Unify three independent metrics systems into a single path: nebula-telemetry registry -> nebula-metrics export.

**Architecture:** Domain crates receive `Option<Arc<MetricsRegistry>>` via DI and record to the shared registry using naming constants from `nebula-metrics`. Custom atomic structs (`ResourceMetrics`, `RotationMetrics`) are replaced. nebula-log's `metrics` crate dependency is removed. A `/metrics` Prometheus endpoint is wired in nebula-api.

**Tech Stack:** nebula-telemetry (MetricsRegistry, Counter, Gauge, Histogram), nebula-metrics (naming constants, PrometheusExporter), axum (API endpoint)

**Design doc:** `docs/plans/2026-04-04-metrics-unification-design.md`

---

### Task 1: Add Credential and Cache Naming Constants

**Files:**
- Modify: `crates/metrics/src/naming.rs`
- Modify: `crates/metrics/src/export/prometheus.rs`
- Modify: `crates/metrics/src/lib.rs`
- Test: `crates/metrics/src/naming.rs` (inline tests)

**Step 1: Write the failing test**

Add to the existing test in `crates/metrics/src/naming.rs`, after `RESOURCE_METRIC_NAMES` array (line ~133):

```rust
const CREDENTIAL_METRIC_NAMES: [&str; 5] = [
    NEBULA_CREDENTIAL_ROTATIONS_TOTAL,
    NEBULA_CREDENTIAL_ROTATION_FAILURES_TOTAL,
    NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS,
    NEBULA_CREDENTIAL_ACTIVE_TOTAL,
    NEBULA_CREDENTIAL_EXPIRED_TOTAL,
];

const CACHE_METRIC_NAMES: [&str; 4] = [
    NEBULA_CACHE_HITS_TOTAL,
    NEBULA_CACHE_MISSES_TOTAL,
    NEBULA_CACHE_EVICTIONS_TOTAL,
    NEBULA_CACHE_SIZE,
];

#[test]
fn credential_constants_are_accessible_unique_and_registry_safe() {
    let registry = MetricsRegistry::new();
    let mut unique = HashSet::new();

    for metric_name in CREDENTIAL_METRIC_NAMES {
        assert!(!metric_name.is_empty());
        assert!(metric_name.starts_with("nebula_credential_"));
        assert!(metric_name.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'));
        assert!(unique.insert(metric_name));

        let counter = registry.counter(metric_name);
        counter.inc();
        assert_eq!(counter.get(), 1);
    }

    assert_eq!(unique.len(), 5);
}

#[test]
fn cache_constants_are_accessible_unique_and_registry_safe() {
    let registry = MetricsRegistry::new();
    let mut unique = HashSet::new();

    for metric_name in CACHE_METRIC_NAMES {
        assert!(!metric_name.is_empty());
        assert!(metric_name.starts_with("nebula_cache_"));
        assert!(metric_name.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'));
        assert!(unique.insert(metric_name));

        let counter = registry.counter(metric_name);
        counter.inc();
        assert_eq!(counter.get(), 1);
    }

    assert_eq!(unique.len(), 4);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo nextest run -p nebula-metrics -- naming`
Expected: FAIL — `NEBULA_CREDENTIAL_ROTATIONS_TOTAL` not found.

**Step 3: Add constants to naming.rs**

After the EventBus section (~line 91), add:

```rust
// ---------------------------------------------------------------------------
// Credential (rotation subsystem)
// ---------------------------------------------------------------------------

/// Counter: total credential rotation attempts.
pub const NEBULA_CREDENTIAL_ROTATIONS_TOTAL: &str = "nebula_credential_rotations_total";
/// Counter: total credential rotation failures.
pub const NEBULA_CREDENTIAL_ROTATION_FAILURES_TOTAL: &str = "nebula_credential_rotation_failures_total";
/// Histogram: credential rotation duration in seconds.
pub const NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS: &str = "nebula_credential_rotation_duration_seconds";
/// Gauge: number of active (non-expired) credentials.
pub const NEBULA_CREDENTIAL_ACTIVE_TOTAL: &str = "nebula_credential_active_total";
/// Counter: total credentials that have expired.
pub const NEBULA_CREDENTIAL_EXPIRED_TOTAL: &str = "nebula_credential_expired_total";

// ---------------------------------------------------------------------------
// Cache (memory crate)
// ---------------------------------------------------------------------------

/// Counter: total cache hits.
pub const NEBULA_CACHE_HITS_TOTAL: &str = "nebula_cache_hits_total";
/// Counter: total cache misses.
pub const NEBULA_CACHE_MISSES_TOTAL: &str = "nebula_cache_misses_total";
/// Counter: total cache evictions.
pub const NEBULA_CACHE_EVICTIONS_TOTAL: &str = "nebula_cache_evictions_total";
/// Gauge: current cache size (number of entries).
pub const NEBULA_CACHE_SIZE: &str = "nebula_cache_size";
```

**Step 4: Add HELP text to prometheus.rs match blocks**

In `counter_help()` (after resource arms, before `_ =>`):

```rust
NEBULA_CREDENTIAL_ROTATIONS_TOTAL => "Total credential rotation attempts.",
NEBULA_CREDENTIAL_ROTATION_FAILURES_TOTAL => "Total credential rotation failures.",
NEBULA_CREDENTIAL_EXPIRED_TOTAL => "Total credentials expired.",
NEBULA_CACHE_HITS_TOTAL => "Total cache hits.",
NEBULA_CACHE_MISSES_TOTAL => "Total cache misses.",
NEBULA_CACHE_EVICTIONS_TOTAL => "Total cache evictions.",
```

In `gauge_help()` (before `_ =>`):

```rust
NEBULA_CREDENTIAL_ACTIVE_TOTAL => "Number of active credentials.",
NEBULA_CACHE_SIZE => "Current cache size in entries.",
```

In `histogram_help()` (before `_ =>`):

```rust
NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS => "Credential rotation duration in seconds.",
```

Add the new imports at the top of `prometheus.rs`.

**Step 5: Add re-exports to lib.rs**

Add the new constants to the `pub use naming::{ ... }` block in `crates/metrics/src/lib.rs`.

**Step 6: Run tests**

Run: `cargo nextest run -p nebula-metrics`
Expected: ALL PASS

**Step 7: Commit**

```bash
git add crates/metrics/src/naming.rs crates/metrics/src/export/prometheus.rs crates/metrics/src/lib.rs
git commit -m "feat(metrics): add credential and cache naming constants"
```

---

### Task 2: Replace ResourceMetrics with Registry-Based Metrics

This is the largest task. nebula-resource currently uses a custom `ResourceMetrics` struct with 5 atomic counters, used in ~15 call sites across manager.rs and 4 runtime files.

**Files:**
- Modify: `crates/resource/Cargo.toml` (add nebula-telemetry, nebula-metrics deps)
- Rewrite: `crates/resource/src/metrics.rs` (replace custom atomics with registry-backed struct)
- Modify: `crates/resource/src/lib.rs` (update exports)
- Modify: `crates/resource/src/manager.rs` (update Manager struct, constructor, call sites)
- Modify: `crates/resource/src/runtime/exclusive.rs` (update record_release call)
- Modify: `crates/resource/src/runtime/pool.rs` (update record_release call)
- Modify: `crates/resource/src/runtime/service.rs` (update record_release call)
- Modify: `crates/resource/src/runtime/transport.rs` (update record_release call)
- Test: `crates/resource/src/metrics.rs` (inline tests)

**Step 1: Add dependencies to Cargo.toml**

Add to `[dependencies]` in `crates/resource/Cargo.toml`:

```toml
nebula-telemetry = { path = "../telemetry" }
nebula-metrics = { path = "../metrics" }
```

**Step 2: Write the new metrics.rs with tests**

Replace entire `crates/resource/src/metrics.rs` with:

```rust
//! Registry-backed metrics for resource operations.
//!
//! [`ResourceOpsMetrics`] wraps telemetry counters from a shared
//! [`MetricsRegistry`]. If no registry is provided, metrics are silently
//! skipped (`Option`-based no-op pattern).

use nebula_metrics::naming::{
    NEBULA_RESOURCE_ACQUIRE_TOTAL, NEBULA_RESOURCE_CREATE_TOTAL, NEBULA_RESOURCE_ERROR_TOTAL,
    NEBULA_RESOURCE_RELEASE_TOTAL,
};
use nebula_telemetry::metrics::{Counter, MetricsRegistry};

/// Registry-backed counters for resource acquire/release/create/destroy.
///
/// Obtain via [`ResourceOpsMetrics::new`] with a shared registry.
/// All methods are lock-free atomic increments.
///
/// # Examples
///
/// ```
/// use nebula_telemetry::metrics::MetricsRegistry;
/// use nebula_resource::metrics::ResourceOpsMetrics;
///
/// let registry = MetricsRegistry::new();
/// let metrics = ResourceOpsMetrics::new(&registry);
/// metrics.record_acquire();
/// metrics.record_acquire_error();
///
/// // Counters are in the shared registry — accessible via Prometheus export.
/// assert_eq!(registry.counter("nebula_resource_acquire_total").get(), 1);
/// assert_eq!(registry.counter("nebula_resource_error_total").get(), 1);
/// ```
#[derive(Debug, Clone)]
pub struct ResourceOpsMetrics {
    acquire_total: Counter,
    acquire_errors: Counter,
    release_total: Counter,
    create_total: Counter,
    destroy_total: Counter,
}

impl ResourceOpsMetrics {
    /// Creates a new metrics instance backed by the given registry.
    #[must_use]
    pub fn new(registry: &MetricsRegistry) -> Self {
        Self {
            acquire_total: registry.counter(NEBULA_RESOURCE_ACQUIRE_TOTAL),
            acquire_errors: registry.counter(NEBULA_RESOURCE_ERROR_TOTAL),
            release_total: registry.counter(NEBULA_RESOURCE_RELEASE_TOTAL),
            create_total: registry.counter(NEBULA_RESOURCE_CREATE_TOTAL),
            destroy_total: registry.counter("nebula_resource_destroy_total"),
        }
    }

    /// Records a successful acquire.
    #[inline]
    pub fn record_acquire(&self) {
        self.acquire_total.inc();
    }

    /// Records a failed acquire attempt.
    #[inline]
    pub fn record_acquire_error(&self) {
        self.acquire_errors.inc();
    }

    /// Records a release (handle drop).
    #[inline]
    pub fn record_release(&self) {
        self.release_total.inc();
    }

    /// Records a new resource instance creation.
    #[inline]
    pub fn record_create(&self) {
        self.create_total.inc();
    }

    /// Records a resource instance destruction.
    #[inline]
    pub fn record_destroy(&self) {
        self.destroy_total.inc();
    }

    /// Returns the current acquire count.
    #[must_use]
    pub fn acquire_total(&self) -> u64 {
        self.acquire_total.get()
    }

    /// Returns the current acquire error count.
    #[must_use]
    pub fn acquire_errors(&self) -> u64 {
        self.acquire_errors.get()
    }

    /// Returns the current release count.
    #[must_use]
    pub fn release_total(&self) -> u64 {
        self.release_total.get()
    }

    /// Returns the current create count.
    #[must_use]
    pub fn create_total(&self) -> u64 {
        self.create_total.get()
    }

    /// Returns the current destroy count.
    #[must_use]
    pub fn destroy_total(&self) -> u64 {
        self.destroy_total.get()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nebula_telemetry::metrics::MetricsRegistry;

    use super::*;

    #[test]
    fn records_acquire_and_error() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry);

        metrics.record_acquire();
        metrics.record_acquire();
        metrics.record_acquire_error();

        assert_eq!(metrics.acquire_total(), 2);
        assert_eq!(metrics.acquire_errors(), 1);
    }

    #[test]
    fn records_create_release_destroy() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry);

        metrics.record_create();
        metrics.record_release();
        metrics.record_release();
        metrics.record_destroy();

        assert_eq!(metrics.create_total(), 1);
        assert_eq!(metrics.release_total(), 2);
        assert_eq!(metrics.destroy_total(), 1);
    }

    #[test]
    fn shared_registry_sees_all_writes() {
        let registry = Arc::new(MetricsRegistry::new());
        let m1 = ResourceOpsMetrics::new(&registry);
        let m2 = ResourceOpsMetrics::new(&registry);

        m1.record_acquire();
        m2.record_acquire();

        // Both write to the same registry counter.
        assert_eq!(registry.counter(NEBULA_RESOURCE_ACQUIRE_TOTAL).get(), 2);
    }

    #[test]
    fn per_resource_isolation_via_labels() {
        let registry = MetricsRegistry::new();
        let interner = registry.interner();

        let labels_db = interner.label_set(&[("resource", "postgres")]);
        let labels_http = interner.label_set(&[("resource", "http_pool")]);

        registry
            .counter_labeled(NEBULA_RESOURCE_ACQUIRE_TOTAL, &labels_db)
            .inc_by(10);
        registry
            .counter_labeled(NEBULA_RESOURCE_ACQUIRE_TOTAL, &labels_http)
            .inc_by(3);

        assert_eq!(
            registry
                .counter_labeled(NEBULA_RESOURCE_ACQUIRE_TOTAL, &labels_db)
                .get(),
            10
        );
        assert_eq!(
            registry
                .counter_labeled(NEBULA_RESOURCE_ACQUIRE_TOTAL, &labels_http)
                .get(),
            3
        );
    }
}
```

**Step 3: Run tests on the metrics module**

Run: `cargo nextest run -p nebula-resource -- metrics`
Expected: ALL PASS

**Step 4: Update lib.rs exports**

In `crates/resource/src/lib.rs`, change line 56:

```rust
// Old:
pub use metrics::{MetricsSnapshot, ResourceMetrics};

// New:
pub use metrics::ResourceOpsMetrics;
```

**Step 5: Update Manager struct and constructor**

In `crates/resource/src/manager.rs`:

Change import (line 31):
```rust
// Old:
use crate::metrics::ResourceMetrics;

// New:
use crate::metrics::ResourceOpsMetrics;
use nebula_telemetry::metrics::MetricsRegistry;
```

Change Manager struct field (line 114):
```rust
// Old:
metrics: Arc<ResourceMetrics>,

// New:
metrics: Option<Arc<ResourceOpsMetrics>>,
```

Change `ManagerConfig` to include optional registry:
```rust
pub struct ManagerConfig {
    pub release_queue_workers: usize,
    pub metrics_registry: Option<Arc<MetricsRegistry>>,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            release_queue_workers: 2,
            metrics_registry: None,
        }
    }
}
```

Change `Manager::with_config()` (line 129):
```rust
// Old:
metrics: Arc::new(ResourceMetrics::new()),

// New:
metrics: config.metrics_registry.as_ref().map(|r| Arc::new(ResourceOpsMetrics::new(r))),
```

**Step 6: Update all metrics call sites in manager.rs**

Each `self.metrics.record_*()` becomes `if let Some(m) = &self.metrics { m.record_*(); }`.

Create a helper macro at the top of manager.rs to avoid repetition:

```rust
macro_rules! record_metric {
    ($metrics:expr, $method:ident) => {
        if let Some(m) = &$metrics {
            m.$method();
        }
    };
}
```

Then replace all call sites:

Line 212-213:
```rust
// Old:
self.metrics.record_create();
managed.metrics.record_create();

// New:
record_metric!(self.metrics, record_create);
record_metric!(managed.metrics(), record_create);
```

Line 1139:
```rust
// Old:
self.metrics.record_destroy();

// New:
record_metric!(self.metrics, record_destroy);
```

Lines 1331-1332:
```rust
// Old:
self.metrics.record_acquire();
managed.metrics.record_acquire();

// New:
record_metric!(self.metrics, record_acquire);
record_metric!(managed.metrics(), record_acquire);
```

Lines 1339-1340:
```rust
// Old:
self.metrics.record_acquire_error();
managed.metrics.record_acquire_error();

// New:
record_metric!(self.metrics, record_acquire_error);
record_metric!(managed.metrics(), record_acquire_error);
```

**Step 7: Update ManagedResource metrics field**

In `crates/resource/src/runtime/managed.rs`, change the `metrics` field type from `Arc<ResourceMetrics>` to `Option<Arc<ResourceOpsMetrics>>`. The `metrics()` accessor returns `&Option<Arc<ResourceOpsMetrics>>`.

In `Manager::register()`, construct per-resource metrics:
```rust
// Old:
let per_resource_metrics = Arc::new(ResourceMetrics::new());

// New:
let per_resource_metrics = config.metrics_registry.as_ref()
    .map(|r| Arc::new(ResourceOpsMetrics::new(r)));
```

**Step 8: Update runtime files**

In each topology runtime file, change the `metrics` parameter type from `Arc<ResourceMetrics>` to `Option<Arc<ResourceOpsMetrics>>`.

`crates/resource/src/runtime/exclusive.rs` line 99:
```rust
// Old:
metrics.record_release();

// New:
if let Some(m) = &metrics { m.record_release(); }
```

Same pattern for `pool.rs:461`, `service.rs:93`, `transport.rs:117`.

**Step 9: Update ResourceHealthSnapshot**

In `crates/resource/src/manager.rs`, `ResourceHealthSnapshot.metrics` field changes. Either:
- Remove the `metrics: MetricsSnapshot` field (snapshot is now in registry)
- Or keep a simple struct with counter values read from the `ResourceOpsMetrics`

**Step 10: Run full test suite**

Run: `cargo check -p nebula-resource && cargo nextest run -p nebula-resource`
Expected: ALL PASS

**Step 11: Commit**

```bash
git add crates/resource/
git commit -m "refactor(resource): replace ResourceMetrics with registry-backed ResourceOpsMetrics"
```

---

### Task 3: Replace RotationMetrics with Registry-Based Metrics

Simpler than Task 2 — `RotationMetrics` has NO production callers. Only defined, tested, and re-exported from `rotation/mod.rs`.

**Files:**
- Modify: `crates/credential/Cargo.toml` (add nebula-telemetry, nebula-metrics deps)
- Rewrite: `crates/credential/src/rotation/metrics.rs`
- Modify: `crates/credential/src/rotation/mod.rs` (update re-export)

**Step 1: Add dependencies**

In `crates/credential/Cargo.toml`, add:

```toml
nebula-telemetry = { path = "../telemetry" }
nebula-metrics = { path = "../metrics" }
```

**Step 2: Write the new rotation metrics**

Replace `crates/credential/src/rotation/metrics.rs` with a registry-backed struct:

```rust
//! Registry-backed metrics for credential rotation.

use std::sync::Arc;

use nebula_metrics::naming::{
    NEBULA_CREDENTIAL_ACTIVE_TOTAL, NEBULA_CREDENTIAL_EXPIRED_TOTAL,
    NEBULA_CREDENTIAL_ROTATIONS_TOTAL, NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS,
    NEBULA_CREDENTIAL_ROTATION_FAILURES_TOTAL,
};
use nebula_telemetry::metrics::{Counter, Gauge, Histogram, MetricsRegistry};

/// Registry-backed counters and histograms for credential rotation.
///
/// All methods are lock-free. Duration percentiles are computed by the
/// telemetry `Histogram` with O(1) bounded buckets — no manual sorting.
#[derive(Debug, Clone)]
pub struct RotationMetrics {
    rotations_total: Counter,
    failures_total: Counter,
    duration_seconds: Histogram,
    active_total: Gauge,
    expired_total: Counter,
}

impl RotationMetrics {
    /// Creates metrics backed by the given registry.
    #[must_use]
    pub fn new(registry: &MetricsRegistry) -> Self {
        Self {
            rotations_total: registry.counter(NEBULA_CREDENTIAL_ROTATIONS_TOTAL),
            failures_total: registry.counter(NEBULA_CREDENTIAL_ROTATION_FAILURES_TOTAL),
            duration_seconds: registry.histogram(NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS),
            active_total: registry.gauge(NEBULA_CREDENTIAL_ACTIVE_TOTAL),
            expired_total: registry.counter(NEBULA_CREDENTIAL_EXPIRED_TOTAL),
        }
    }

    /// Records a rotation attempt with its duration and outcome.
    pub fn record_rotation(&self, duration: std::time::Duration, success: bool) {
        self.rotations_total.inc();
        self.duration_seconds.observe(duration.as_secs_f64());
        if !success {
            self.failures_total.inc();
        }
    }

    /// Records a rotation failure (without duration — e.g., pre-check failure).
    pub fn record_failure(&self) {
        self.rotations_total.inc();
        self.failures_total.inc();
    }

    /// Sets the current number of active credentials.
    pub fn set_active(&self, count: i64) {
        self.active_total.set(count);
    }

    /// Records a credential expiration.
    pub fn record_expired(&self) {
        self.expired_total.inc();
    }

    /// Total rotation attempts.
    #[must_use]
    pub fn total_rotations(&self) -> u64 {
        self.rotations_total.get()
    }

    /// Total rotation failures.
    #[must_use]
    pub fn total_failures(&self) -> u64 {
        self.failures_total.get()
    }

    /// Success rate as 0.0..=1.0. Returns 0.0 if no rotations recorded.
    #[must_use]
    pub fn success_rate(&self) -> f64 {
        let total = self.rotations_total.get();
        if total == 0 {
            return 0.0;
        }
        let failures = self.failures_total.get();
        (total - failures) as f64 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use nebula_telemetry::metrics::MetricsRegistry;

    use super::*;

    #[test]
    fn records_successful_rotation() {
        let registry = MetricsRegistry::new();
        let metrics = RotationMetrics::new(&registry);

        metrics.record_rotation(std::time::Duration::from_secs(2), true);

        assert_eq!(metrics.total_rotations(), 1);
        assert_eq!(metrics.total_failures(), 0);
        assert!((metrics.success_rate() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn records_failed_rotation() {
        let registry = MetricsRegistry::new();
        let metrics = RotationMetrics::new(&registry);

        metrics.record_rotation(std::time::Duration::from_millis(500), false);

        assert_eq!(metrics.total_rotations(), 1);
        assert_eq!(metrics.total_failures(), 1);
        assert!(metrics.success_rate().abs() < f64::EPSILON);
    }

    #[test]
    fn success_rate_mixed() {
        let registry = MetricsRegistry::new();
        let metrics = RotationMetrics::new(&registry);

        metrics.record_rotation(std::time::Duration::from_secs(1), true);
        metrics.record_rotation(std::time::Duration::from_secs(1), true);
        metrics.record_rotation(std::time::Duration::from_secs(1), false);

        assert_eq!(metrics.total_rotations(), 3);
        assert!((metrics.success_rate() - 2.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn success_rate_zero_rotations() {
        let registry = MetricsRegistry::new();
        let metrics = RotationMetrics::new(&registry);
        assert!(metrics.success_rate().abs() < f64::EPSILON);
    }

    #[test]
    fn active_and_expired_tracking() {
        let registry = MetricsRegistry::new();
        let metrics = RotationMetrics::new(&registry);

        metrics.set_active(10);
        metrics.record_expired();
        metrics.record_expired();

        assert_eq!(
            registry
                .gauge(NEBULA_CREDENTIAL_ACTIVE_TOTAL)
                .get(),
            10
        );
        assert_eq!(
            registry
                .counter(NEBULA_CREDENTIAL_EXPIRED_TOTAL)
                .get(),
            2
        );
    }

    #[test]
    fn duration_recorded_in_histogram() {
        let registry = MetricsRegistry::new();
        let metrics = RotationMetrics::new(&registry);

        metrics.record_rotation(std::time::Duration::from_millis(250), true);
        metrics.record_rotation(std::time::Duration::from_secs(3), true);

        let hist = registry.histogram(NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS);
        assert_eq!(hist.count(), 2);
        assert!((hist.sum() - 3.25).abs() < 0.01);
    }
}
```

**Step 3: Update mod.rs re-export**

In `crates/credential/src/rotation/mod.rs` line 278, remove `CredentialMetrics`:

```rust
// Old:
pub use metrics::{CredentialMetrics, RotationMetrics};

// New:
pub use metrics::RotationMetrics;
```

**Step 4: Run tests**

Run: `cargo check -p nebula-credential && cargo nextest run -p nebula-credential`
Expected: ALL PASS

**Step 5: Commit**

```bash
git add crates/credential/
git commit -m "refactor(credential): replace RotationMetrics with registry-backed implementation"
```

---

### Task 4: Add Optional Registry Recording to nebula-memory

nebula-memory's stats structs (`AtomicCacheStats`, `MemoryStats`) serve dual purpose: internal cache logic (eviction decisions, pressure detection) AND observability. We keep the internal structs but add optional registry recording alongside.

**Files:**
- Modify: `crates/memory/Cargo.toml` (add optional nebula-telemetry, nebula-metrics deps)
- Create: `crates/memory/src/cache/registry_stats.rs` (thin bridge)
- Modify: `crates/memory/src/cache/mod.rs` (re-export)
- Test: `crates/memory/src/cache/registry_stats.rs`

**Step 1: Add optional dependencies**

In `crates/memory/Cargo.toml`:

```toml
[dependencies]
nebula-telemetry = { path = "../telemetry", optional = true }
nebula-metrics = { path = "../metrics", optional = true }

[features]
telemetry = ["nebula-telemetry", "nebula-metrics"]
```

**Step 2: Write registry_stats.rs**

Create `crates/memory/src/cache/registry_stats.rs`:

```rust
//! Bridge between [`AtomicCacheStats`] and the telemetry registry.
//!
//! Call [`sync_to_registry`] periodically to push cache stats
//! into the shared metrics registry for Prometheus export.

use nebula_metrics::naming::{
    NEBULA_CACHE_EVICTIONS_TOTAL, NEBULA_CACHE_HITS_TOTAL, NEBULA_CACHE_MISSES_TOTAL,
    NEBULA_CACHE_SIZE,
};
use nebula_telemetry::metrics::MetricsRegistry;

use super::stats::CacheStats;

/// Pushes a [`CacheStats`] snapshot into the registry as gauge values.
///
/// This is a point-in-time sync — call it periodically (e.g., every 30s)
/// from a background task, not on every cache operation.
pub fn sync_to_registry(stats: &CacheStats, registry: &MetricsRegistry) {
    registry
        .gauge(NEBULA_CACHE_HITS_TOTAL)
        .set(stats.hits as i64);
    registry
        .gauge(NEBULA_CACHE_MISSES_TOTAL)
        .set(stats.misses as i64);
    registry
        .gauge(NEBULA_CACHE_EVICTIONS_TOTAL)
        .set(stats.evictions as i64);
    registry
        .gauge(NEBULA_CACHE_SIZE)
        .set(stats.entry_count as i64);
}

#[cfg(test)]
mod tests {
    use nebula_telemetry::metrics::MetricsRegistry;

    use super::*;

    #[test]
    fn sync_pushes_stats_to_registry() {
        let registry = MetricsRegistry::new();
        let stats = CacheStats {
            hits: 100,
            misses: 20,
            evictions: 5,
            insertions: 120,
            deletions: 15,
            entry_count: 105,
            size_bytes: 4096,
        };

        sync_to_registry(&stats, &registry);

        assert_eq!(registry.gauge(NEBULA_CACHE_HITS_TOTAL).get(), 100);
        assert_eq!(registry.gauge(NEBULA_CACHE_MISSES_TOTAL).get(), 20);
        assert_eq!(registry.gauge(NEBULA_CACHE_EVICTIONS_TOTAL).get(), 5);
        assert_eq!(registry.gauge(NEBULA_CACHE_SIZE).get(), 105);
    }

    #[test]
    fn sync_overwrites_previous_values() {
        let registry = MetricsRegistry::new();

        let stats1 = CacheStats {
            hits: 50, misses: 10, evictions: 2,
            insertions: 60, deletions: 8, entry_count: 52, size_bytes: 2048,
        };
        sync_to_registry(&stats1, &registry);

        let stats2 = CacheStats {
            hits: 200, misses: 40, evictions: 15,
            insertions: 240, deletions: 25, entry_count: 215, size_bytes: 8192,
        };
        sync_to_registry(&stats2, &registry);

        assert_eq!(registry.gauge(NEBULA_CACHE_HITS_TOTAL).get(), 200);
        assert_eq!(registry.gauge(NEBULA_CACHE_SIZE).get(), 215);
    }
}
```

**Step 3: Add module to cache/mod.rs**

Add conditional module:

```rust
#[cfg(feature = "telemetry")]
pub mod registry_stats;
```

**Step 4: Run tests**

Run: `cargo nextest run -p nebula-memory --features telemetry`
Expected: ALL PASS

**Step 5: Commit**

```bash
git add crates/memory/
git commit -m "feat(memory): add optional telemetry bridge for cache stats"
```

---

### Task 5: Remove Metrics from nebula-log

Remove the `observability` feature, `metrics` crate dependency, and related code from nebula-log. No external crates depend on these — only nebula-log's own examples use them.

**Files:**
- Modify: `crates/log/Cargo.toml` (remove metrics dep, observability feature)
- Delete: `crates/log/src/metrics/helpers.rs` (TimingGuard, timed_block)
- Modify: `crates/log/src/metrics/mod.rs` (remove observability re-exports)
- Modify: `crates/log/src/observability/hooks.rs` (remove MetricsHook)
- Modify: `crates/log/src/observability/mod.rs` (remove MetricsHook export)
- Modify: `crates/log/src/lib.rs` (remove metrics re-exports)
- Delete or update: `crates/log/examples/prometheus_integration.rs`
- Update: `crates/log/examples/observability_hooks.rs` (remove MetricsHook usage)

**Step 1: Remove metrics dependency and feature from Cargo.toml**

In `crates/log/Cargo.toml`:

Remove:
```toml
metrics = { workspace = true, optional = true }
```

Change features:
```toml
# Old:
observability = ["metrics"]
telemetry = ["opentelemetry", "opentelemetry_sdk", "opentelemetry-otlp", "tracing-opentelemetry", "observability"]
full = ["ansi", "async", "file", "log-compat", "telemetry", "sentry", "observability"]

# New:
# Remove observability feature entirely
telemetry = ["opentelemetry", "opentelemetry_sdk", "opentelemetry-otlp", "tracing-opentelemetry"]
full = ["ansi", "async", "file", "log-compat", "telemetry", "sentry"]
```

**Step 2: Gut the metrics helpers module**

Replace `crates/log/src/metrics/helpers.rs` with empty module (or keep only non-metrics helpers if any):

```rust
//! Metrics helpers — removed during metrics unification.
//!
//! Timing utilities (`timed_block`, `TimingGuard`) have moved to
//! `nebula-telemetry`. See `docs/plans/2026-04-04-metrics-unification-design.md`.
```

**Step 3: Clean up metrics/mod.rs**

Replace `crates/log/src/metrics/mod.rs`:

```rust
//! Metrics module — ecosystem `metrics` crate support removed.
//!
//! Use `nebula-telemetry` and `nebula-metrics` for all metric recording
//! and export. See `docs/plans/2026-04-04-metrics-unification-design.md`.

pub mod helpers;

#[cfg(test)]
mod tests {
    #[test]
    fn module_compiles() {
        // Placeholder — metrics functionality moved to nebula-telemetry.
    }
}
```

**Step 4: Remove MetricsHook from observability/hooks.rs**

Delete the entire `MetricsHook` struct and its `ObservabilityHook` impl (lines ~279-316 in `crates/log/src/observability/hooks.rs`).

**Step 5: Update observability/mod.rs**

Remove `MetricsHook` from the public exports.

**Step 6: Update lib.rs**

Remove all `#[cfg(feature = "observability")]` exports:

```rust
// Remove these lines:
#[cfg(feature = "observability")]
pub use crate::metrics::{counter, gauge, histogram, timed_block, timed_block_async};

#[cfg(feature = "observability")]
pub use crate::observability::MetricsHook;
```

And from the prelude:

```rust
// Remove:
#[cfg(feature = "observability")]
pub use crate::metrics::{counter, gauge, histogram, timed_block, timed_block_async};

#[cfg(feature = "observability")]
pub use crate::observability::MetricsHook;
```

**Step 7: Update/remove examples**

- Delete `crates/log/examples/prometheus_integration.rs` (depends on `metrics` crate)
- Update `crates/log/examples/observability_hooks.rs` to remove `MetricsHook` usage

**Step 8: Run tests**

Run: `cargo check -p nebula-log && cargo nextest run -p nebula-log`
Expected: ALL PASS

Run: `cargo check -p nebula-log --features full` (verify full feature still works)
Expected: PASS

**Step 9: Commit**

```bash
git add crates/log/
git commit -m "refactor(log): remove metrics crate dependency and observability feature"
```

---

### Task 6: Remove Legacy Naming Constants

**Files:**
- Modify: `crates/metrics/src/naming.rs` (remove LEGACY_* constants)
- Modify: `crates/metrics/src/lib.rs` (remove LEGACY re-exports)

**Step 1: Check no code uses legacy constants**

Run: `cargo grep LEGACY_EXECUTIONS` and `cargo grep LEGACY_ACTIONS` across workspace.

If no external uses found:

**Step 2: Remove legacy constants from naming.rs**

Delete lines 96-116 (the entire "Legacy names" section).

**Step 3: Remove legacy re-exports from lib.rs**

Remove any `LEGACY_*` from the `pub use naming::{ ... }` block.

**Step 4: Run tests**

Run: `cargo check --workspace && cargo nextest run -p nebula-metrics`
Expected: ALL PASS

**Step 5: Commit**

```bash
git add crates/metrics/
git commit -m "chore(metrics): remove legacy naming constants"
```

---

### Task 7: Wire GET /metrics Endpoint in nebula-api

**Files:**
- Modify: `crates/api/Cargo.toml` (add nebula-metrics, nebula-telemetry deps)
- Modify: `crates/api/src/state.rs` (add MetricsRegistry to AppState)
- Create: `crates/api/src/routes/metrics.rs` (Prometheus handler)
- Modify: `crates/api/src/routes/mod.rs` (mount /metrics route)

**Step 1: Add dependencies**

In `crates/api/Cargo.toml`:

```toml
nebula-metrics = { path = "../metrics" }
nebula-telemetry = { path = "../telemetry" }
```

**Step 2: Add registry to AppState**

In `crates/api/src/state.rs`:

```rust
use nebula_telemetry::metrics::MetricsRegistry;

pub struct AppState {
    // ... existing fields ...

    /// Shared metrics registry for Prometheus export.
    pub metrics_registry: Option<Arc<MetricsRegistry>>,
}
```

Update `AppState::new()` to accept optional registry.

**Step 3: Create metrics route handler**

Create `crates/api/src/routes/metrics.rs`:

```rust
//! Prometheus metrics endpoint.

use axum::{Router, extract::State, http::StatusCode, response::IntoResponse};

use crate::state::AppState;

/// GET /metrics — Prometheus text format.
async fn prometheus_handler(State(state): State<AppState>) -> impl IntoResponse {
    match &state.metrics_registry {
        Some(registry) => {
            let body = nebula_metrics::snapshot(registry);
            let content_type = nebula_metrics::content_type();
            (StatusCode::OK, [(axum::http::header::CONTENT_TYPE, content_type)], body)
        }
        None => {
            (StatusCode::SERVICE_UNAVAILABLE, [(axum::http::header::CONTENT_TYPE, "text/plain")], "Metrics not configured".to_string())
        }
    }
}

pub fn router() -> Router<AppState> {
    Router::new().route("/metrics", axum::routing::get(prometheus_handler))
}
```

**Step 4: Mount in routes/mod.rs**

```rust
pub mod metrics;

pub fn create_routes(state: AppState, _config: &ApiConfig) -> Router {
    Router::new()
        .merge(health::router())
        .merge(metrics::router())  // No auth — Prometheus scrapes unauthenticated
        .nest("/api/v1", api_v1_routes(state.clone()))
        .with_state(state)
}
```

**Step 5: Run tests**

Run: `cargo check -p nebula-api && cargo nextest run -p nebula-api`
Expected: ALL PASS

**Step 6: Commit**

```bash
git add crates/api/
git commit -m "feat(api): add GET /metrics Prometheus endpoint"
```

---

### Task 8: Full Workspace Verification

**Step 1: Format and lint**

Run: `cargo fmt && cargo clippy --workspace -- -D warnings`
Expected: PASS with zero warnings

**Step 2: Full test suite**

Run: `cargo nextest run --workspace`
Expected: ALL PASS

**Step 3: Doctests**

Run: `cargo test --workspace --doc`
Expected: ALL PASS

**Step 4: Dependency check**

Run: `cargo deny check`
Expected: PASS

**Step 5: Context files**

Update `.claude/crates/metrics.md`, `.claude/crates/resource.md`, `.claude/crates/credential.md`, `.claude/crates/log.md`, `.claude/active-work.md` to reflect the unification.

**Step 6: Final commit**

```bash
git add .claude/
git commit -m "docs: update context files for metrics unification"
```
