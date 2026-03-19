# nebula-metrics Overhaul — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Overhaul nebula-metrics — delete dead code, introduce typed `MetricName`, safe defaults, and fix the critical resource metrics facade mismatch.

**Architecture:** Seven proposals executed in dependency order: cleanup dead code (P2/P3/P6/P7) → typed MetricName enum (P4) → safe LabelAllowlist default (P5) → unify resource metrics onto MetricsRegistry (P1).

**Tech Stack:** Rust 1.93, nebula-telemetry MetricsRegistry, lasso string interning, Prometheus text format.

---

## Task 1: Remove unused `thiserror` dependency (P7)

**Files:**
- Modify: `crates/metrics/Cargo.toml:17`

**Step 1:** Remove line `thiserror = { workspace = true }` from `[dependencies]`.

**Step 2:** Verify build

```bash
cargo check -p nebula-metrics
```

**Step 3:** Commit

```bash
git add crates/metrics/Cargo.toml
git commit -m "chore(metrics): remove unused thiserror dependency"
```

---

## Task 2: Delete legacy naming constants (P3)

**Files:**
- Modify: `crates/metrics/src/naming.rs:93-117` — delete LEGACY_* constants
- Modify: `crates/metrics/src/naming.rs` — delete `legacy_constants_follow_expected_pattern` test

**Step 1:** Delete lines 93-117 (the legacy constants block) and the test `legacy_constants_follow_expected_pattern` from `mod tests`.

**Step 2:** Verify

```bash
cargo nextest run -p nebula-metrics
```

**Step 3:** Commit

```bash
git add crates/metrics/src/naming.rs
git commit -m "chore(metrics): delete unused LEGACY_* naming constants"
```

---

## Task 3: Delete TelemetryAdapter + remove eventbus dep (P2 + P6)

This is the biggest cleanup — removes ~400 lines of dead code.

**Files:**
- Delete: `crates/metrics/src/adapter.rs`
- Modify: `crates/metrics/src/lib.rs` — remove `pub mod adapter`, `pub use adapter::TelemetryAdapter`, eventbus re-exports
- Modify: `crates/metrics/src/prelude.rs` — remove `TelemetryAdapter` re-export
- Modify: `crates/metrics/Cargo.toml` — remove `nebula-eventbus` dependency
- Modify: `crates/metrics/tests/integration.rs` — remove tests that use TelemetryAdapter, rewrite to use MetricsRegistry directly

**Step 1:** Delete `adapter.rs` entirely.

**Step 2:** Update `lib.rs`:
```rust
// Remove these lines:
pub mod adapter;
pub use adapter::TelemetryAdapter;
// Keep everything else
```

**Step 3:** Update `prelude.rs` — remove `pub use crate::adapter::TelemetryAdapter;`

**Step 4:** Update `Cargo.toml` — remove `nebula-eventbus = { path = "../eventbus" }` from `[dependencies]`.

**Step 5:** Rewrite `tests/integration.rs` — all tests used `TelemetryAdapter`. Replace with direct `MetricsRegistry` usage:

```rust
use std::sync::Arc;
use nebula_metrics::naming::{...};
use nebula_metrics::{LabelAllowlist, MetricsRegistry, snapshot};

#[test]
fn resource_metrics_round_trip_via_registry() {
    let registry = Arc::new(MetricsRegistry::new());
    registry.counter(NEBULA_RESOURCE_CREATE_TOTAL).inc();
    registry.histogram(NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS).observe(0.5);
    assert_eq!(registry.counter(NEBULA_RESOURCE_CREATE_TOTAL).get(), 1);
}

#[test]
fn labeled_metrics_round_trip_to_prometheus_export() {
    let registry = Arc::new(MetricsRegistry::new());
    let allowlist = LabelAllowlist::only(["action_type"]);
    let raw = registry.interner().label_set(&[
        ("action_type", "http.request"),
        ("execution_id", "uuid-abc"),
    ]);
    let safe = allowlist.apply(&raw, registry.interner());
    registry.counter_labeled(NEBULA_ACTION_EXECUTIONS_TOTAL, &safe).inc_by(10);
    let out = snapshot(&registry);
    assert!(out.contains(r#"action_type="http.request""#));
    assert!(!out.contains("execution_id"));
}

#[test]
fn mixed_labeled_and_unlabeled_metrics_export() {
    let registry = Arc::new(MetricsRegistry::new());
    let labels = registry.interner().label_set(&[("action_type", "http.request")]);
    registry.counter(NEBULA_ACTION_EXECUTIONS_TOTAL).inc_by(5);
    registry.counter_labeled(NEBULA_ACTION_EXECUTIONS_TOTAL, &labels).inc_by(10);
    let out = snapshot(&registry);
    assert!(out.contains("nebula_action_executions_total 5\n"));
    assert!(out.contains(r#"nebula_action_executions_total{action_type="http.request"} 10"#));
}
```

**Step 6:** Verify

```bash
cargo nextest run -p nebula-metrics
cargo clippy -p nebula-metrics -- -D warnings
```

**Step 7:** Commit

```bash
git add -A crates/metrics/
git commit -m "refactor(metrics): delete unused TelemetryAdapter and eventbus dependency

BREAKING CHANGE: TelemetryAdapter removed — use MetricsRegistry directly.
nebula-eventbus no longer a dependency of nebula-metrics."
```

---

## Task 4: Typed MetricName enum (P4)

Replace stringly-typed constants with a `MetricName` struct that carries name, kind, and help text. Eliminates the `counter_help()`/`gauge_help()`/`histogram_help()` match arms in the exporter.

**Files:**
- Rewrite: `crates/metrics/src/naming.rs`
- Modify: `crates/metrics/src/lib.rs` — update re-exports
- Rewrite: `crates/metrics/src/export/prometheus.rs` — use `ALL_METRICS` for help/type lookup
- Modify: `crates/engine/src/engine.rs:22-26` — update imports
- Modify: `crates/runtime/src/runtime.rs:12-15` — update imports
- Modify: `crates/resource/src/metrics.rs:14-22` — update imports

### Step 1: Write MetricName type + all constants in naming.rs

```rust
/// The type of metric primitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricKind {
    /// Monotonically increasing counter.
    Counter,
    /// Point-in-time gauge.
    Gauge,
    /// Distribution of observed values.
    Histogram,
}

/// A well-known Nebula metric with its name, kind, and help text.
///
/// Use [`MetricName::as_str`] to pass to [`MetricsRegistry`] methods.
///
/// # Examples
///
/// ```rust
/// use nebula_metrics::naming::WORKFLOW_EXECUTIONS_STARTED;
/// assert_eq!(WORKFLOW_EXECUTIONS_STARTED.as_str(), "nebula_workflow_executions_started_total");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MetricName {
    name: &'static str,
    kind: MetricKind,
    help: &'static str,
}

impl MetricName {
    /// The Prometheus-compatible metric name string.
    #[must_use]
    pub const fn as_str(&self) -> &'static str { self.name }

    /// The metric primitive kind.
    #[must_use]
    pub const fn kind(&self) -> MetricKind { self.kind }

    /// The HELP description for Prometheus export.
    #[must_use]
    pub const fn help(&self) -> &'static str { self.help }
}

impl AsRef<str> for MetricName {
    fn as_ref(&self) -> &str { self.name }
}

impl std::fmt::Display for MetricName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}
```

Then constants (shortened names, no `NEBULA_` prefix on the Rust constant since the string already has it):

```rust
// Workflow
pub const WORKFLOW_EXECUTIONS_STARTED: MetricName = MetricName {
    name: "nebula_workflow_executions_started_total",
    kind: MetricKind::Counter,
    help: "Total workflow executions started.",
};
// ... all others follow the same pattern

/// All well-known metrics for exporter iteration.
pub const ALL_METRICS: &[MetricName] = &[
    WORKFLOW_EXECUTIONS_STARTED,
    WORKFLOW_EXECUTIONS_COMPLETED,
    // ... all 25 metrics
];
```

**Important:** Keep the old `NEBULA_*` constants as `pub const NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL: &str = WORKFLOW_EXECUTIONS_STARTED.as_str();` for one release cycle if desired, or just delete them (breaking changes allowed). **Decision: delete them.** Callers use `METRIC.as_str()`.

### Step 2: Update prometheus.rs exporter

Delete `counter_help()`, `gauge_help()`, `histogram_help()` functions. Replace with:

```rust
use crate::naming::{ALL_METRICS, MetricKind, MetricName};

fn lookup_metric(name: &str) -> Option<MetricName> {
    ALL_METRICS.iter().find(|m| m.as_str() == name).copied()
}

fn metric_help(name: &str) -> &'static str {
    lookup_metric(name).map_or("Custom metric.", MetricName::help)
}

fn metric_type_str(name: &str, fallback: &str) -> &'static str {
    match lookup_metric(name).map(|m| m.kind()) {
        Some(MetricKind::Counter) => "counter",
        Some(MetricKind::Gauge) => "gauge",
        Some(MetricKind::Histogram) => "histogram",
        None => fallback,
    }
}
```

Then in `snapshot()`, use these instead of the three match functions.

### Step 3: Update lib.rs re-exports

```rust
pub use naming::{MetricKind, MetricName, ALL_METRICS};
// Remove old NEBULA_* re-exports
```

### Step 4: Update engine imports

`crates/engine/src/engine.rs:22-26`:
```rust
// Before:
use nebula_metrics::naming::{
    NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS, ...
};
// After:
use nebula_metrics::naming::{
    WORKFLOW_EXECUTION_DURATION, WORKFLOW_EXECUTIONS_COMPLETED,
    WORKFLOW_EXECUTIONS_FAILED, WORKFLOW_EXECUTIONS_STARTED,
};
```

Update call sites: `self.metrics.counter(NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL)` → `self.metrics.counter(WORKFLOW_EXECUTIONS_STARTED.as_str())`

### Step 5: Update runtime imports

`crates/runtime/src/runtime.rs:12-15` — same pattern as engine.

### Step 6: Update resource imports

`crates/resource/src/metrics.rs:14-22` — update all 15 constant names. The `metrics::counter!()` macro calls still use string names, so: `metrics::counter!(RESOURCE_CREATE.as_str(), "resource_id" => id)`.

### Step 7: Update all tests in nebula-metrics

All naming tests, prometheus tests, and integration tests need updated constant names.

### Step 8: Verify

```bash
cargo fmt
cargo clippy --workspace -- -D warnings
cargo nextest run -p nebula-metrics -p nebula-engine -p nebula-runtime -p nebula-resource
cargo test --doc -p nebula-metrics
```

### Step 9: Commit

```bash
git add crates/metrics/ crates/engine/src/engine.rs crates/runtime/src/runtime.rs crates/resource/src/metrics.rs
git commit -m "feat(metrics): replace string constants with typed MetricName enum

BREAKING CHANGE: All NEBULA_* constants replaced with shorter typed constants.
MetricName carries name, kind, and help text. Callers use .as_str() for registry methods."
```

---

## Task 5: LabelAllowlist default to deny-all (P5)

**Files:**
- Modify: `crates/metrics/src/filter.rs:113-117`
- Modify: `crates/metrics/src/filter.rs` — update tests

### Step 1: Add `none()` constructor, change `Default`

```rust
/// Allow no labels — the safe default for production.
#[must_use]
pub fn none() -> Self {
    Self { inner: AllowlistInner::Keys(Vec::new()) }
}

impl Default for LabelAllowlist {
    fn default() -> Self {
        Self::none()
    }
}
```

### Step 2: Update test `default_is_passthrough` → `default_is_deny_all`

```rust
#[test]
fn default_is_deny_all() {
    let d = LabelAllowlist::default();
    assert!(!d.is_passthrough());
    // Verify it actually strips labels
    let reg = registry();
    let labels = reg.interner().label_set(&[("key", "value")]);
    let filtered = d.apply(&labels, reg.interner());
    assert_eq!(filtered.len(), 0);
}
```

### Step 3: Verify

```bash
cargo nextest run -p nebula-metrics
```

### Step 4: Commit

```bash
git add crates/metrics/src/filter.rs
git commit -m "feat(metrics): LabelAllowlist default to deny-all for cardinality safety

BREAKING CHANGE: LabelAllowlist::default() now strips all labels.
Use LabelAllowlist::all() explicitly for passthrough."
```

---

## Task 6: Unify resource metrics onto MetricsRegistry (P1)

This is the critical fix — resource metrics currently go to the `metrics` crate facade and are invisible to PrometheusExporter.

**Files:**
- Rewrite: `crates/resource/src/metrics.rs` — accept `Arc<MetricsRegistry>`, record via registry
- Modify: `crates/resource/Cargo.toml` — remove `metrics = { workspace = true }`
- Modify: `crates/resource/src/lib.rs` — if MetricsCollector re-export changes
- Rewrite: `crates/resource/tests/metrics_integration.rs` — pass registry, verify actual metric values
- Modify: `crates/resource/src/metrics.rs` — delete `RESOURCE_LABELS` static, use `LabelAllowlist` from nebula-metrics

### Step 1: Rewrite MetricsCollector struct

```rust
use std::sync::Arc;
use nebula_core::ResourceKey;
use nebula_metrics::naming::*;  // new MetricName constants
use nebula_metrics::filter::LabelAllowlist;
use nebula_telemetry::metrics::MetricsRegistry;
use nebula_telemetry::labels::LabelSet;
use tokio_util::sync::CancellationToken;
use crate::events::{EventBus, EventSubscriber, ResourceEvent};

pub struct MetricsCollector {
    subscriber: EventSubscriber<ResourceEvent>,
    registry: Arc<MetricsRegistry>,
    allowlist: LabelAllowlist,
}

impl MetricsCollector {
    /// Create a new collector.
    ///
    /// # Arguments
    /// - `event_bus` — source of resource lifecycle events
    /// - `registry` — telemetry registry where metrics are recorded
    #[must_use]
    pub fn new(event_bus: &EventBus, registry: Arc<MetricsRegistry>) -> Self {
        Self {
            subscriber: event_bus.subscribe(),
            registry,
            allowlist: LabelAllowlist::only(["resource_id", "operation"]),
        }
    }
```

### Step 2: Rewrite record_event to use MetricsRegistry

Replace every `metrics::counter!(NAME, "resource_id" => id).increment(1)` with:
```rust
let labels = self.resource_labels(resource_key);
self.registry.counter_labeled(RESOURCE_CREATE.as_str(), &labels).inc();
```

For histogram:
```rust
self.registry.histogram_labeled(RESOURCE_ACQUIRE_WAIT_DURATION.as_str(), &labels)
    .observe(wait_duration.as_secs_f64());
```

For gauge:
```rust
self.registry.gauge_labeled(RESOURCE_HEALTH_STATE.as_str(), &labels)
    .set(score as i64);  // Note: Gauge uses i64, not f64
```

**Important:** Gauge API uses `i64`, not `f64`. Health score needs mapping: 1.0→100, 0.5→50, 0.0→0 (or keep as 1/0 binary with degraded=0).

### Step 3: Replace static RESOURCE_LABELS with per-instance cardinality tracking

Delete the `LazyLock<DashSet<String>>` global. Replace with instance-level tracking:

```rust
use dashmap::DashSet;

pub struct MetricsCollector {
    subscriber: EventSubscriber<ResourceEvent>,
    registry: Arc<MetricsRegistry>,
    allowlist: LabelAllowlist,
    seen_labels: DashSet<String>,
}
```

The `resource_label()` method moves to `&self` and uses `self.seen_labels`.

### Step 4: Update spawn_metrics_collector signature

```rust
pub fn spawn_metrics_collector(
    event_bus: &Arc<EventBus>,
    registry: Arc<MetricsRegistry>,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    let collector = MetricsCollector::new(event_bus, registry);
    tokio::spawn(collector.run(cancel))
}
```

### Step 5: Remove `metrics` dep from Cargo.toml

Delete line 38: `metrics = { workspace = true }`.

Also remove `dashmap` from resource Cargo.toml IF it's only used for RESOURCE_LABELS.
Check: dashmap is also used elsewhere in resource (DashMap for pool internals), so keep it.

### Step 6: Rewrite integration tests

Tests can now **verify actual metric values** instead of just "didn't panic":

```rust
use std::sync::Arc;
use nebula_metrics::MetricsRegistry;
use nebula_metrics::naming::*;
use nebula_resource::events::{EventBus, ResourceEvent, CleanupReason};
use nebula_resource::metrics::MetricsCollector;

#[tokio::test]
async fn collector_records_create_event_to_registry() {
    let registry = Arc::new(MetricsRegistry::new());
    let bus = Arc::new(EventBus::new(64));
    let collector = MetricsCollector::new(&bus, Arc::clone(&registry));
    let cancel = tokio_util::sync::CancellationToken::new();
    let handle = tokio::spawn(collector.run(cancel));

    let key = nebula_core::resource_key!("db");
    bus.emit(ResourceEvent::Created { resource_key: key, scope: Scope::Global });
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Now we can actually verify the metric was recorded!
    let counters = registry.snapshot_counters();
    let create_count: u64 = counters.iter()
        .filter(|(k, _)| registry.interner().resolve(k.name) == RESOURCE_CREATE.as_str())
        .map(|(_, c)| c.get())
        .sum();
    assert_eq!(create_count, 1);

    drop(bus);
    let _ = tokio::time::timeout(Duration::from_secs(1), handle).await;
}
```

### Step 7: Verify

```bash
cargo fmt
cargo clippy -p nebula-resource -- -D warnings
cargo nextest run -p nebula-resource
# Full workspace check
cargo clippy --workspace -- -D warnings
cargo nextest run --workspace
```

### Step 8: Commit

```bash
git add crates/resource/
git commit -m "fix(resource): migrate metrics from metrics crate to MetricsRegistry

BREAKING CHANGE: MetricsCollector::new() and spawn_metrics_collector()
now require Arc<MetricsRegistry>. Resource metrics are now visible
in PrometheusExporter output.

Previously resource metrics were recorded via the metrics crate facade
and silently lost — PrometheusExporter only reads MetricsRegistry."
```

---

## Task 7: Update context files + final verification

**Files:**
- Modify: `.claude/crates/metrics.md`
- Modify: `.claude/crates/resource.md`
- Modify: `.claude/active-work.md`

### Step 1: Update metrics context

Document: TelemetryAdapter removed, MetricName enum added, LabelAllowlist default changed.

### Step 2: Update resource context

Document: metrics crate dependency removed, MetricsCollector now takes Arc<MetricsRegistry>.

### Step 3: Full validation

```bash
cargo fmt
cargo clippy --workspace -- -D warnings
cargo nextest run --workspace
cargo test --workspace --doc
cargo deny check
```

### Step 4: Commit

```bash
git add .claude/
git commit -m "docs: update context files for metrics overhaul"
```

---

## Verification Summary

After all tasks:
- `cargo clippy --workspace -- -D warnings` — zero warnings
- `cargo nextest run --workspace` — all tests pass
- `cargo test --workspace --doc` — doc tests pass
- `cargo deny check` — dependency audit passes
- Resource metrics visible in `snapshot()` output
- No `metrics` crate facade in the workspace (nebula-resource was the only user)
- No TelemetryAdapter references anywhere
- All metric constants are typed `MetricName`
- `LabelAllowlist::default()` is deny-all
