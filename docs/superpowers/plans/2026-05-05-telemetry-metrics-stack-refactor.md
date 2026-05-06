# Telemetry/Metrics Stack Refactor — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the coordinated `nebula-telemetry` + `nebula-metrics` corrections from the joint audit so that the safe path of recording an operator metric satisfies all 7 points of `nebula-telemetry-metrics-joint-audit.md` "7-Point Safe-Path Answer" (today: 6 of 7 answer "No").

**Architecture:** Two crates, strict layer split. `nebula-telemetry` owns primitive identity, atomic correctness, snapshot semantics, and an explicit error model. `nebula-metrics` owns the typed `MetricCatalog`, per-metric `LabelSchema`, Prometheus rendering correctness, and the only public recording path. Boundary types — `MetricKind`, `BucketSchema`, atomic `HistogramSnapshot` — are defined in `nebula-telemetry` and consumed unchanged by `nebula-metrics`.

**Tech Stack:** Rust 1.95+, `dashmap`, `lasso`, `thiserror`, `nebula-error`, `tracing`, `proptest` (already used elsewhere), `criterion` for benches.

---

## Order rationale: telemetry first

Asked: "metrics or telemetry first by dependence level?"

Answer: **telemetry first**. Direct dependency-graph evidence from the joint audit cross-reference matrix (`docs/audits/nebula-telemetry-metrics-joint-audit.md` lines 1147-1166):

| Metrics-side fix | Blocked on telemetry-side fix |
|------------------|--------------------------------|
| J-008 typed `MetricDescriptor` catalog | J-002 / T-002 — registry must reject same-key-different-kind, otherwise descriptor enforcement is purely advisory at the metrics layer. |
| J-009 Prometheus type-conflict export check | J-002 — telemetry rejects at registration; exporter assertion only catches what telemetry permitted to slip through. |
| J-010 `+Inf` / `NaN` rendering | J-006 / T-006 — saturating sum at primitive level; renderer fixes are downstream of the overflow contract. |
| J-013 reject invalid catalog names at descriptor registration | J-002, J-009 — relies on a typed-identity registry to fail-fast instead of sanitizing. |
| J-015 catalog descriptor carries `BucketSchema` | J-005 / T-004 — telemetry must treat bucket layout as identity, otherwise the descriptor's bucket field is silently ignored. |
| J-016 pre-registration of zero-traffic metrics | J-008 — needs a typed catalog to walk; itself blocked on J-002. |
| J-019 kind-tagged exporter iterator | J-002 — telemetry must publish `(MetricKey, MetricKind, value)` tuples first. |
| M-002 filter-before-intern `SafeLabels` | T-016 — telemetry must expose a primitive that interns only allowed pairs; filter-after-intern remains unsound otherwise. |
| M-004 atomic histogram exporter input | T-003 — `HistogramSnapshot` is a primitive contract; exporter consumes it unchanged. |

There is **no** finding where a metrics-side change blocks a telemetry-side change. Phase A (Telemetry) is therefore a strict prerequisite for Phase B (Metrics). Phases C-E layer on top.

The plan also keeps boundary-contract findings (J-009, J-010, J-019) as a dedicated Phase C so neither crate's feature is "done" before the seam between them is exercised end-to-end.

## Phase Map

| Phase | Scope | Joint findings | Per-crate findings |
|-------|-------|----------------|--------------------|
| A — Telemetry primitives | Identity, type-per-key, snapshot, error model, retention/clone semantics | J-001, J-002, J-003, J-005, J-006, J-011, J-017, J-020, J-021, J-023, J-024, J-025 | T-001..T-021 |
| B — Metrics catalog and safe path | Typed `MetricCatalog`, `SafeLabels`, prelude tightening, descriptor-only adapter, exporter sanitization rejection | J-004, J-007, J-008, J-013, J-015, J-016 | M-001..M-026 |
| C — Boundary contracts | Type-conflict export check, Prometheus numeric encoder, kind-tagged iterator, OpenMetrics target gating | J-009, J-010, J-019, J-026 | M-005, M-014 |
| D — Joint tests and benches | End-to-end safe-path, concurrency under scrape, high-cardinality misuse, descriptor round-trip | J-018 | T-tests, M-tests |
| E — Canon and docs | PRODUCT_CANON output-vs-decision, interner non-guarantee, reverse-flow self-throttle | J-012, J-014, J-022 | — |

## File Structure

### `crates/telemetry/src/` (after Phase A)

- `error.rs` — extended `TelemetryError` with `MetricConflict`, `BucketLayoutConflict`, `ForeignLabelSet`, `Overflow`, `InvalidObservation`, `InvalidBucketLayout` variants.
- `labels.rs` — `LabelInterner` no longer `Clone`; new `LabelInternerHandle` (read-only `Arc` view) replaces external clones; new `InternerId` (u64 generation tag); `LabelSet` becomes opaque, carries `InternerId`; `MetricKey` becomes opaque (private fields).
- `metrics.rs` — `MetricsRegistry` no longer `Clone`; held only as `Arc<MetricsRegistry>`. Single `DashMap<MetricKey, MetricEntry>` keyed by identity, where `MetricEntry { kind: MetricKind, value: MetricValue, buckets: Option<Arc<BucketLayout>> }`. New `MetricKind { Counter, Gauge, Histogram }` (re-exported by `nebula-metrics`). `Counter::inc_by` / `Gauge::inc` / `Gauge::dec` saturate; `Histogram::sum` saturates at `f64::MAX`. `HistogramSnapshot` is a frozen value snapshot; `snapshot_*` returns those instead of live handles.
- `snapshot.rs` (new) — `HistogramSnapshot { count, sum, finite_buckets: Box<[u64]>, layout: Arc<BucketLayout> }`, `RegistrySnapshot` (kind-tagged iterator), all `#[non_exhaustive]`.
- `lib.rs` — exports `MetricKind`, `BucketLayout`, `BucketSchema`, `HistogramSnapshot`, `RegistrySnapshot`; removes `MetricsRegistry: Clone` from re-exports if any.
- `examples/basic_metrics.rs` — moved to root-level `examples/` workspace member per `feedback_examples_location.md`; renamed metrics to non-canonical names (e.g. `demo_counter`).
- `tests/` — new `concurrency.rs`, `identity.rs`, `snapshot_consistency.rs`, `overflow.rs`, `histogram_layout.rs`, `interner_isolation.rs`.

### `crates/metrics/src/` (after Phase B)

- `descriptor.rs` (new) — `MetricDescriptor { name, kind, unit, help, labels: LabelSchema, buckets: Option<BucketSchema>, stability: Stability }`; `MetricName(&'static str)` newtype with compile-time `nebula_*` snake-case validation through a const fn; `Stability { Stable, Reserved }`.
- `catalog.rs` (new) — `MetricCatalog` consts (one per family), enumerable via `inventory::collect!` or a manual `&'static [&'static MetricDescriptor]` table; replaces `naming.rs` constants. Old `naming.rs` becomes a thin compat shim that delegates to descriptor names for the duration of one release, then deleted in same phase (no shim left behind per `feedback_no_shims.md`).
- `safe_labels.rs` (new) — `SafeLabels<'reg>` builder; checks `LabelSchema` before calling `LabelInterner::intern` so unsafe values never enter the interner (closes M-002). Returns `LabelSet` bound to the registry's `InternerId`.
- `adapter.rs` — `TelemetryAdapter::new(Arc<MetricsRegistry>)` (no `LabelAllowlist::all()` default); only descriptor-keyed methods; raw `counter`/`gauge`/`histogram` removed (not renamed — per `feedback_no_shims.md` no `*_unchecked` shim, callers migrate to descriptor-typed methods or import `nebula_telemetry` directly with documented caveats); `registry()` and `interner()` accessors removed; new `record(&self, descriptor: &MetricDescriptor, labels: SafeLabels<'_>)` is the only public entry.
- `filter.rs` — deleted. `LabelAllowlist` is replaced by per-descriptor `LabelSchema`.
- `export/prometheus.rs` — consumes `RegistrySnapshot::iter()` (kind-tagged); rejects type conflicts with `ExportError::TypeConflict { name }`; `ExportError::InvalidName` returned for non-catalog names instead of sanitizing; numeric encoder formats `+Inf`, `-Inf`, `NaN` correctly (closes J-010 / M-014); content-type stays text 0.0.4 but exporter is parameterized by `Format::{Text004, OpenMetricsV1}` to unblock J-026 without implementing OpenMetrics yet.
- `prelude.rs` — re-exports descriptor types only; no `MetricsRegistry`, no `Counter`/`Gauge`/`Histogram` (closes J-004 / M-001).
- `lib.rs` — same surface contraction; new `unsafe_telemetry` module gated by `#[doc(hidden)]` for the rare custom-metric escape hatch.
- `tests/` — new `catalog_completeness.rs`, `safe_labels.rs`, `export_validity.rs`, `descriptor_round_trip.rs`.
- `benches/` — `record_hot_path.rs`, `scrape_under_load.rs`.

### Cross-cutting

- `docs/canon/PRODUCT_CANON.md` — new section "Observability dataflow: output vs decision input" (closes J-014).
- `docs/canon/observability.md` (new) — declares interner non-guarantee, reverse-flow self-throttle contract (closes J-012, J-022).
- `examples/observability/` (root workspace member) — runnable demo of the safe path (per `feedback_examples_location.md`).

---

## Phase A — Telemetry primitives

### Task A-1: Extend `TelemetryError` with the variants the rest of the phase will return

**Files:**
- Modify: `crates/telemetry/src/error.rs:1-15`
- Test: `crates/telemetry/tests/error.rs` (new)

- [ ] **Step 1: Write failing tests for each new variant**

```rust
// crates/telemetry/tests/error.rs
use nebula_telemetry::error::TelemetryError;

#[test]
fn metric_conflict_carries_kinds() {
    let e = TelemetryError::MetricConflict {
        name: "x".into(),
        existing: "Counter",
        requested: "Gauge",
    };
    let s = e.to_string();
    assert!(s.contains("Counter"));
    assert!(s.contains("Gauge"));
    assert!(s.contains("\"x\""));
}

#[test]
fn bucket_layout_conflict_renders() {
    let e = TelemetryError::BucketLayoutConflict {
        name: "lat".into(),
        existing: vec![0.1, 1.0],
        requested: vec![0.5, 5.0],
    };
    assert!(e.to_string().contains("[0.1, 1.0]"));
}

#[test]
fn foreign_label_set_renders() {
    let e = TelemetryError::ForeignLabelSet { expected: 7, found: 9 };
    assert!(e.to_string().contains("7"));
    assert!(e.to_string().contains("9"));
}

#[test]
fn overflow_renders() {
    let e = TelemetryError::Overflow { metric: "c".into(), kind: "counter" };
    assert!(e.to_string().contains("counter"));
}

#[test]
fn invalid_observation_renders() {
    let e = TelemetryError::InvalidObservation { value: f64::NAN };
    assert!(e.to_string().contains("NaN"));
}

#[test]
fn invalid_bucket_layout_renders() {
    let e = TelemetryError::InvalidBucketLayout("empty".into());
    assert!(e.to_string().contains("empty"));
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```
cargo test -p nebula-telemetry --test error
```

Expected: compile errors — variants do not exist.

- [ ] **Step 3: Add variants to `TelemetryError`**

Replace `crates/telemetry/src/error.rs` body with:

```rust
//! Error types for the telemetry subsystem.

#[derive(Debug, thiserror::Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum TelemetryError {
    #[classify(category = "internal", code = "TELEMETRY:IO")]
    #[error("sink I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[classify(category = "invariant", code = "TELEMETRY:METRIC_CONFLICT")]
    #[error("metric {name:?} already registered as {existing}, requested {requested}")]
    MetricConflict {
        name: String,
        existing: &'static str,
        requested: &'static str,
    },

    #[classify(category = "invariant", code = "TELEMETRY:BUCKET_LAYOUT_CONFLICT")]
    #[error("histogram {name:?} bucket layout conflict: existing {existing:?}, requested {requested:?}")]
    BucketLayoutConflict {
        name: String,
        existing: Vec<f64>,
        requested: Vec<f64>,
    },

    #[classify(category = "invariant", code = "TELEMETRY:FOREIGN_LABEL_SET")]
    #[error("label set from interner {found} used with registry interner {expected}")]
    ForeignLabelSet { expected: u64, found: u64 },

    #[classify(category = "internal", code = "TELEMETRY:OVERFLOW")]
    #[error("{kind} {metric:?} overflowed primitive bounds")]
    Overflow { metric: String, kind: &'static str },

    #[classify(category = "validation", code = "TELEMETRY:INVALID_OBSERVATION")]
    #[error("invalid histogram observation: {value}")]
    InvalidObservation { value: f64 },

    #[classify(category = "validation", code = "TELEMETRY:INVALID_BUCKET_LAYOUT")]
    #[error("invalid bucket layout: {0}")]
    InvalidBucketLayout(String),
}

pub type TelemetryResult<T> = Result<T, TelemetryError>;
```

Confirm `nebula-error::Classify` categories `invariant` and `validation` exist; if not, add them in the same step under `crates/error/src/category.rs` and document in the same commit.

- [ ] **Step 4: Re-run tests; expect pass**

```
cargo test -p nebula-telemetry --test error
```

- [ ] **Step 5: Commit**

```
git add crates/telemetry/src/error.rs crates/telemetry/tests/error.rs
git commit -m "feat(telemetry): extend TelemetryError with primitive failure variants

Adds MetricConflict, BucketLayoutConflict, ForeignLabelSet, Overflow,
InvalidObservation, InvalidBucketLayout. Closes T-009."
```

### Task A-2: Add `InternerId` generation tag and bind `LabelInterner` clones to one identity

Closes T-001 (precondition for J-001) by making it impossible to use a `LabelSet` with a registry that did not produce its symbols.

**Files:**
- Modify: `crates/telemetry/src/labels.rs:131-200, 220-260`
- Test: `crates/telemetry/tests/interner_isolation.rs` (new)

- [ ] **Step 1: Write failing tests**

```rust
// crates/telemetry/tests/interner_isolation.rs
use nebula_telemetry::labels::{LabelInterner, LabelSet};
use nebula_telemetry::error::TelemetryError;

#[test]
fn interners_have_distinct_ids() {
    let a = LabelInterner::new();
    let b = LabelInterner::new();
    assert_ne!(a.id(), b.id());
}

#[test]
fn label_set_carries_interner_id() {
    let interner = LabelInterner::new();
    let labels = interner.label_set(&[("status", "ok")]);
    assert_eq!(labels.interner_id(), interner.id());
}

#[test]
fn cross_interner_use_is_detectable() {
    let a = LabelInterner::new();
    let b = LabelInterner::new();
    let labels_a = a.label_set(&[("status", "ok")]);
    // Resolving against a foreign interner returns Err, not panic.
    assert!(matches!(
        b.try_resolve_label_set(&labels_a),
        Err(TelemetryError::ForeignLabelSet { .. })
    ));
}
```

- [ ] **Step 2: Run; confirm fail**

```
cargo test -p nebula-telemetry --test interner_isolation
```

- [ ] **Step 3: Implement**

In `crates/telemetry/src/labels.rs`:

- Replace `#[derive(Clone, Debug)] pub struct LabelInterner` (line 131) with a non-`Clone` struct carrying a `u64` id allocated from a process-wide `AtomicU64` counter.
- Add `pub fn id(&self) -> u64`.
- Replace public `LabelSet { kv: Vec<...> }` with opaque struct carrying `interner_id: u64` and the `kv` vec; expose `interner_id()` accessor.
- Add `LabelInterner::try_resolve_label_set(&self, set: &LabelSet) -> Result<Vec<(&str, &str)>, TelemetryError>` which checks `set.interner_id() == self.id()` and returns `ForeignLabelSet` otherwise.
- Update `label_set` to stamp the new ID on the produced `LabelSet`.

```rust
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_INTERNER_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub struct LabelInterner {
    rodeo: Arc<ThreadedRodeo>,
    id: u64,
}

impl LabelInterner {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rodeo: Arc::new(ThreadedRodeo::new()),
            id: NEXT_INTERNER_ID.fetch_add(1, Ordering::Relaxed),
        }
    }

    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }
    // ... existing intern/resolve methods unchanged ...
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LabelSet {
    interner_id: u64,
    kv: Vec<(LabelKey, LabelValue)>,
}

impl LabelSet {
    #[must_use]
    pub fn interner_id(&self) -> u64 { self.interner_id }
    pub(crate) fn kv(&self) -> &[(LabelKey, LabelValue)] { &self.kv }
}
```

`MetricKey` follows the same opaque pattern in Task A-3.

- [ ] **Step 4: Run tests; expect pass**

```
cargo test -p nebula-telemetry --test interner_isolation
cargo test -p nebula-telemetry --lib
```

- [ ] **Step 5: Commit**

```
git add crates/telemetry/src/labels.rs crates/telemetry/tests/interner_isolation.rs
git commit -m "feat(telemetry): bind LabelInterner and LabelSet to a generation id

Removes Clone from LabelInterner; LabelSet now carries the producing
interner's id. Cross-interner use returns ForeignLabelSet instead of
panicking on resolve. Closes T-001 (J-001)."
```

### Task A-3: Make `MetricKey` opaque and registry-bound

**Files:**
- Modify: `crates/telemetry/src/labels.rs` (MetricKey definition area)
- Test: `crates/telemetry/tests/identity.rs` (new)

- [ ] **Step 1: Write failing test**

```rust
// crates/telemetry/tests/identity.rs
use nebula_telemetry::metrics::MetricsRegistry;

#[test]
fn metric_key_fields_are_private() {
    // Compile-time check: this should NOT compile when fields are private.
    // Documented as a doc-test failing example in metrics.rs.
}

#[test]
fn same_name_same_labels_same_key() {
    let reg = MetricsRegistry::new();
    let labels = reg.interner().label_set(&[("status", "ok")]);
    let k1 = reg.metric_key("nebula_x", &labels).unwrap();
    let k2 = reg.metric_key("nebula_x", &labels).unwrap();
    assert_eq!(k1, k2);
}
```

- [ ] **Step 2: Confirm fail; cannot construct `MetricKey` outside the crate.**

- [ ] **Step 3: Implement**

In `labels.rs`: change `MetricKey { pub name: ..., pub labels: ... }` to private fields. Add a crate-private constructor used only by `MetricsRegistry`. Add `MetricKey::name(&self) -> &str` and `MetricKey::labels(&self) -> &LabelSet` for read access. Expose `MetricsRegistry::metric_key(&self, name: &str, labels: &LabelSet) -> TelemetryResult<MetricKey>` that validates `labels.interner_id() == self.interner.id()` and returns `ForeignLabelSet` otherwise.

- [ ] **Step 4: Run tests; expect pass**

- [ ] **Step 5: Commit**

```
git add crates/telemetry/src/labels.rs crates/telemetry/tests/identity.rs
git commit -m "feat(telemetry): make MetricKey opaque and registry-bound

MetricKey no longer constructible outside the crate; created only
via MetricsRegistry::metric_key, which validates interner identity.
Closes T-001 (J-001) registry-bound half."
```

### Task A-4: Single identity table with `MetricKind` per key

Closes T-002 (J-002). This is the largest single change in Phase A and unblocks every catalog/exporter type-conflict fix downstream.

**Files:**
- Modify: `crates/telemetry/src/metrics.rs` substantial rework around lines 380-700
- Test: `crates/telemetry/tests/identity.rs` (extended)

- [ ] **Step 1: Write failing tests**

```rust
// append to crates/telemetry/tests/identity.rs
use nebula_telemetry::metrics::{MetricsRegistry, MetricKind};
use nebula_telemetry::error::TelemetryError;

#[test]
fn same_key_same_kind_succeeds() {
    let reg = MetricsRegistry::new();
    let _ = reg.counter("nebula_x", &[]).unwrap();
    let _ = reg.counter("nebula_x", &[]).unwrap();
}

#[test]
fn same_key_different_kind_errors() {
    let reg = MetricsRegistry::new();
    let _ = reg.counter("nebula_x", &[]).unwrap();
    let err = reg.gauge("nebula_x", &[]).unwrap_err();
    assert!(matches!(
        err,
        TelemetryError::MetricConflict {
            existing: "Counter",
            requested: "Gauge",
            ..
        }
    ));
}

#[test]
fn registry_iter_is_kind_tagged() {
    let reg = MetricsRegistry::new();
    reg.counter("c", &[]).unwrap().inc();
    reg.gauge("g", &[]).unwrap().set(5);
    let kinds: Vec<MetricKind> = reg.snapshot().iter().map(|e| e.kind()).collect();
    assert!(kinds.contains(&MetricKind::Counter));
    assert!(kinds.contains(&MetricKind::Gauge));
}
```

- [ ] **Step 2: Confirm fail**

- [ ] **Step 3: Implement**

Replace the three-`DashMap` registry with one `DashMap<MetricKey, MetricEntry>`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricKind { Counter, Gauge, Histogram }

impl MetricKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Counter => "Counter",
            Self::Gauge => "Gauge",
            Self::Histogram => "Histogram",
        }
    }
}

#[derive(Debug)]
struct MetricEntry {
    kind: MetricKind,
    value: MetricValue,
    layout: Option<Arc<BucketLayout>>,
}

#[derive(Debug, Clone)]
enum MetricValue {
    Counter(Counter),
    Gauge(Gauge),
    Histogram(Histogram),
}

pub struct MetricsRegistry {
    entries: Arc<DashMap<MetricKey, MetricEntry>>,
    interner: LabelInterner,
}
```

Replace `counter`, `gauge`, `histogram`, `*_labeled`, and `histogram_with_buckets_labeled` with fallible `Result<_, TelemetryError>` versions that:
- Build `MetricKey` (validates interner id).
- `entry(key).or_insert_with(...)` for new entries.
- For existing entries, check `kind`. Mismatch returns `MetricConflict` with the existing variant name.
- Return shared handle (`Arc`-backed inside the variant).

The `MetricsRegistry: Clone` derive is removed in this task: production composition holds `Arc<MetricsRegistry>`. This closes J-021 / J-023 simultaneously.

- [ ] **Step 4: Audit all repo call sites**

```
cargo check --workspace
```

Expected: compile errors at every direct `nebula_telemetry::MetricsRegistry::counter(...)` call site that ignored the `Result`. Fix each by `?`-propagating or `.expect("registered at startup")` only at composition root. Audit lists from `M-001` and `M-010` of the metrics audit identify the call sites:

- `crates/engine/src/engine.rs:47`
- `crates/engine/src/runtime/runtime.rs:22`
- `crates/api/src/services/webhook/transport.rs:39`
- `crates/resource/src/metrics.rs:18`
- `crates/engine/src/control_consumer.rs:213, 237`
- `crates/api/src/state.rs:89`

Touch each in this same task — Phase A is not done while the workspace does not compile.

- [ ] **Step 5: Run tests; expect pass workspace-wide**

```
cargo test --workspace --no-run
cargo test -p nebula-telemetry
```

- [ ] **Step 6: Commit**

```
git add -A
git commit -m "feat(telemetry): single identity table with MetricKind

MetricsRegistry now holds one DashMap<MetricKey, MetricEntry> tagged
with MetricKind. Same key + different kind returns MetricConflict.
Removes MetricsRegistry: Clone; production code holds Arc<MetricsRegistry>.
Closes T-002 (J-002), T-021, J-021, J-023."
```

### Task A-5: Bucket layout is part of histogram identity

Closes T-004 (J-005).

**Files:**
- Modify: `crates/telemetry/src/metrics.rs` (Histogram registration path)
- Test: `crates/telemetry/tests/histogram_layout.rs` (new)

- [ ] **Step 1: Write failing test**

```rust
// crates/telemetry/tests/histogram_layout.rs
use nebula_telemetry::metrics::MetricsRegistry;
use nebula_telemetry::error::TelemetryError;

#[test]
fn same_key_same_layout_succeeds() {
    let reg = MetricsRegistry::new();
    let _ = reg.histogram_with_buckets("h", &[], &[0.1, 0.5, 1.0]).unwrap();
    let _ = reg.histogram_with_buckets("h", &[], &[0.1, 0.5, 1.0]).unwrap();
}

#[test]
fn same_key_different_layout_errors() {
    let reg = MetricsRegistry::new();
    let _ = reg.histogram_with_buckets("h", &[], &[0.1, 0.5, 1.0]).unwrap();
    let err = reg.histogram_with_buckets("h", &[], &[2.0, 4.0, 8.0]).unwrap_err();
    assert!(matches!(err, TelemetryError::BucketLayoutConflict { .. }));
}
```

- [ ] **Step 2: Confirm fail**

- [ ] **Step 3: Implement**

In the histogram registration branch of `MetricEntry`:

```rust
match entry {
    Entry::Occupied(o) => {
        let existing = o.get();
        if existing.kind != MetricKind::Histogram {
            return Err(TelemetryError::MetricConflict {
                name: name.into(),
                existing: existing.kind.as_str(),
                requested: "Histogram",
            });
        }
        let existing_layout = existing.layout.as_ref().expect("histogram has layout");
        if existing_layout.boundaries() != requested_boundaries {
            return Err(TelemetryError::BucketLayoutConflict {
                name: name.into(),
                existing: existing_layout.boundaries().to_vec(),
                requested: requested_boundaries.to_vec(),
            });
        }
        // ...existing behavior: clone the handle
    }
    Entry::Vacant(v) => { /* insert new histogram with layout */ }
}
```

Remove the `tracing::warn!` first-layout-wins path completely (no shim).

- [ ] **Step 4: Run tests; expect pass**

- [ ] **Step 5: Commit**

```
git add crates/telemetry/src/metrics.rs crates/telemetry/tests/histogram_layout.rs
git commit -m "feat(telemetry): bucket layout is part of histogram identity

Same-key different-layout returns BucketLayoutConflict; first-wins
warning path removed. Closes T-004 (J-005)."
```

### Task A-6: Saturating counter/gauge arithmetic; saturating histogram sum

Closes T-006 (J-006) and the telemetry half of J-010.

**Files:**
- Modify: `crates/telemetry/src/metrics.rs` (Counter, Gauge, Histogram bodies)
- Test: `crates/telemetry/tests/overflow.rs` (new)

- [ ] **Step 1: Write failing tests**

```rust
// crates/telemetry/tests/overflow.rs
use nebula_telemetry::metrics::MetricsRegistry;

#[test]
fn counter_saturates_at_u64_max() {
    let reg = MetricsRegistry::new();
    let c = reg.counter("c", &[]).unwrap();
    c.inc_by(u64::MAX);
    c.inc(); // would wrap if not saturating
    assert_eq!(c.get(), u64::MAX);
}

#[test]
fn gauge_saturates_at_i64_bounds() {
    let reg = MetricsRegistry::new();
    let g = reg.gauge("g", &[]).unwrap();
    g.set(i64::MAX);
    g.inc(); // would wrap
    assert_eq!(g.get(), i64::MAX);
    g.set(i64::MIN);
    g.dec();
    assert_eq!(g.get(), i64::MIN);
}

#[test]
fn histogram_sum_saturates() {
    let reg = MetricsRegistry::new();
    let h = reg.histogram_with_buckets("h", &[], &[1.0]).unwrap();
    for _ in 0..10 { h.observe(f64::MAX / 2.0); }
    let snap = h.snapshot();
    assert!(snap.sum.is_finite());
    assert_eq!(snap.sum, f64::MAX);
}
```

- [ ] **Step 2: Confirm fail**

- [ ] **Step 3: Implement saturating ops**

Replace `fetch_add` with a CAS loop:

```rust
impl Counter {
    pub fn inc_by(&self, n: u64) {
        let mut cur = self.value.load(Ordering::Relaxed);
        loop {
            let new = cur.saturating_add(n);
            match self.value.compare_exchange_weak(cur, new, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(observed) => cur = observed,
            }
        }
        // last_updated_ms write deferred to A-7 task that defines its semantics.
    }
}

impl Gauge {
    pub fn inc(&self) { self.add_saturating(1); }
    pub fn dec(&self) { self.add_saturating(-1); }
    fn add_saturating(&self, delta: i64) {
        let mut cur = self.value.load(Ordering::Relaxed);
        loop {
            let new = cur.saturating_add(delta);
            match self.value.compare_exchange_weak(cur, new, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(observed) => cur = observed,
            }
        }
    }
}
```

Histogram sum: same CAS pattern over `AtomicU64` carrying `f64::to_bits`, with `f64::MAX` as the saturation cap when `sum + observation > f64::MAX`.

Document each in rustdoc — overflow behavior is now explicit.

- [ ] **Step 4: Run tests; expect pass**

- [ ] **Step 5: Commit**

```
git add crates/telemetry/src/metrics.rs crates/telemetry/tests/overflow.rs
git commit -m "feat(telemetry): saturating Counter/Gauge/Histogram arithmetic

CAS loops replace fetch_add/fetch_sub. Overflow saturates instead of
wrapping; histogram sum saturates at f64::MAX. Documents the contract.
Closes T-006 (J-006); telemetry half of J-010."
```

### Task A-7: `last_updated_ms` semantics and `inc_by(0)` contract

Closes T-019 (J-017) and J-025.

**Files:**
- Modify: `crates/telemetry/src/metrics.rs` (Counter/Gauge/Histogram)

- [ ] **Step 1: Write failing test capturing the chosen contract**

```rust
#[test]
fn inc_by_zero_does_not_touch_last_updated_ms() {
    let reg = MetricsRegistry::new();
    let c = reg.counter("c", &[]).unwrap();
    c.inc();
    let t1 = c.last_updated_ms();
    std::thread::sleep(std::time::Duration::from_millis(5));
    c.inc_by(0); // liveness ping is NOT enough to refresh
    let t2 = c.last_updated_ms();
    assert_eq!(t1, t2, "inc_by(0) must not refresh last_updated_ms");
}
```

- [ ] **Step 2: Confirm fail**

- [ ] **Step 3: Implement**

Skip the timestamp update when `n == 0` for `Counter::inc_by`. Document `last_updated_ms` as "wall-clock time of the most recent **value-changing** operation observed by **any** holder of this handle". This locks in J-025 explicitly: timestamp is shared across cloned handles by design, not by accident.

- [ ] **Step 4: Run tests; expect pass**

- [ ] **Step 5: Commit**

```
git add crates/telemetry/src/metrics.rs
git commit -m "fix(telemetry): inc_by(0) no longer refreshes last_updated_ms

Documents last_updated_ms as last-changed (not last-touched) and
explicit about cross-handle sharing. Closes T-019 (J-017) and J-025."
```

### Task A-8: Frozen `HistogramSnapshot` and kind-tagged `RegistrySnapshot`

Closes T-003 (J-003) and produces the input shape Phase C exporter expects (J-019).

**Files:**
- Create: `crates/telemetry/src/snapshot.rs`
- Modify: `crates/telemetry/src/metrics.rs` (snapshot APIs)
- Modify: `crates/telemetry/src/lib.rs` (re-exports)
- Test: `crates/telemetry/tests/snapshot_consistency.rs` (new)

- [ ] **Step 1: Write failing test**

```rust
// crates/telemetry/tests/snapshot_consistency.rs
use nebula_telemetry::metrics::MetricsRegistry;
use std::sync::Arc;
use std::thread;

#[test]
fn histogram_snapshot_invariants_hold_under_concurrent_observe() {
    let reg = Arc::new(MetricsRegistry::new());
    let h = reg.histogram_with_buckets("h", &[], &[0.1, 0.5, 1.0]).unwrap();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_w = stop.clone();
    let h_w = h.clone();
    let writer = thread::spawn(move || {
        while !stop_w.load(std::sync::atomic::Ordering::Relaxed) {
            h_w.observe(0.2);
            h_w.observe(0.7);
            h_w.observe(2.0);
        }
    });
    for _ in 0..1_000 {
        let snap = h.snapshot();
        let finite_sum: u64 = snap.finite_buckets.iter().sum();
        assert!(snap.count >= finite_sum, "+Inf bucket must dominate");
        assert!(snap.sum.is_finite() || snap.sum == f64::MAX);
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    writer.join().unwrap();
}
```

- [ ] **Step 2: Confirm fail (current snapshot reads atomics independently)**

- [ ] **Step 3: Implement frozen snapshot via seqlock**

Wrap the histogram's mutable state (`buckets[]`, `count`, `sum`) in a per-histogram `parking_lot::RwLock<HistState>`, OR a seqlock pattern. Seqlock is preferred (lock-free reads, no allocation). Implementation outline:

```rust
// crates/telemetry/src/snapshot.rs
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct HistogramSnapshot {
    pub count: u64,
    pub sum: f64,
    pub finite_buckets: Box<[u64]>,
    pub layout: Arc<BucketLayout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricKind { Counter, Gauge, Histogram }

#[derive(Debug)]
pub struct RegistryEntry {
    pub key: MetricKey,
    pub kind: MetricKind,
    pub value: SnapshotValue,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum SnapshotValue {
    Counter(u64),
    Gauge(i64),
    Histogram(HistogramSnapshot),
}

pub struct RegistrySnapshot { /* ... */ }

impl RegistrySnapshot {
    pub fn iter(&self) -> impl Iterator<Item = &RegistryEntry> { /* ... */ }
}
```

In `Histogram::snapshot()`, take the lock once and clone out a frozen value. This eliminates the relaxed-atomics-tear scenario in M-004.

In `MetricsRegistry::snapshot()`, walk the unified `DashMap<MetricKey, MetricEntry>` (from A-4) and emit `RegistryEntry` values. The exporter consumes this iterator in Phase C; live handles are no longer exposed.

- [ ] **Step 4: Remove old `snapshot_counters` / `snapshot_gauges` / `snapshot_histograms`**

Per `feedback_no_shims.md`: do not keep them as deprecated forwarders. Update internal callers (mostly the exporter, but Phase C reshapes that anyway) to use `snapshot()`.

- [ ] **Step 5: Run tests; expect pass**

```
cargo test -p nebula-telemetry
```

- [ ] **Step 6: Commit**

```
git add -A
git commit -m "feat(telemetry): frozen RegistrySnapshot + atomic HistogramSnapshot

Replaces snapshot_* live-handle enumerations with one immutable
RegistrySnapshot tagged by MetricKind. Histograms use a seqlock to
guarantee count >= sum-of-finite-buckets and sum is the value at
snapshot time. Closes T-003 (J-003) and primes J-019."
```

### Task A-9: Drop the retention feature (or fully define it)

Closes T-005, T-012, T-020 (J-011) and J-024.

- [ ] **Step 1: Decision recorded as ADR**

Create `docs/adr/0021-telemetry-retention-decision.md`. Per `feedback_adr_revisable.md` and the joint audit's recommendation: **drop `retain_recent` and `compact_interner` from the public API**. They have no production caller; their `&mut self` contract is incompatible with `Arc<MetricsRegistry>` composition (J-021); their post-call clone-fork (J-023) is a footgun without a use case. Bring retention back behind a feature flag if a real workload appears.

- [ ] **Step 2: Remove the methods**

Delete `MetricsRegistry::retain_recent` and `MetricsRegistry::compact_interner`. Delete `last_updated_ms` from primitive bodies (its only consumer was retention; A-7 already documented that it is otherwise informational). If callers exist outside telemetry tests, fail the compile and resolve in this task — no shim.

- [ ] **Step 3: Update README and rustdoc to reflect the simpler model**

```
crates/telemetry/README.md
crates/telemetry/src/metrics.rs (rustdoc)
```

- [ ] **Step 4: Confirm clean compile and tests pass**

- [ ] **Step 5: Commit**

```
git add -A
git commit -m "feat(telemetry)!: remove retain_recent / compact_interner

ADR-0021 records the decision. Retention had no production callers
and its &mut self contract conflicted with Arc<MetricsRegistry>
composition. Closes T-005 / T-012 / T-020 (J-011), J-024.

BREAKING CHANGE: MetricsRegistry no longer exposes retain_recent or
compact_interner. Future retention work returns behind a feature flag."
```

### Task A-10: Histogram observation validation

Closes T-018 (NaN/Infinity silent drop) and the strict half of J-006.

**Files:**
- Modify: `crates/telemetry/src/metrics.rs` (Histogram::observe)

- [ ] **Step 1: Failing test**

```rust
#[test]
fn observe_nan_is_explicit() {
    let reg = MetricsRegistry::new();
    let h = reg.histogram_with_buckets("h", &[], &[1.0]).unwrap();
    assert!(h.try_observe(f64::NAN).is_err());
    assert!(h.try_observe(f64::INFINITY).is_err());
}
```

- [ ] **Step 2: Confirm fail**

- [ ] **Step 3: Implement**

Add `Histogram::try_observe(&self, value: f64) -> TelemetryResult<()>` returning `InvalidObservation` for non-finite. Keep the infallible `observe(value: f64)` only for the safe path that filters non-finite at the metrics descriptor layer (Phase B). Document both.

- [ ] **Step 4: Pass**

- [ ] **Step 5: Commit**

```
git commit -m "feat(telemetry): try_observe surfaces non-finite observations as errors

Closes T-018."
```

### Task A-11: Allocation-contract docs and sanity benches

Closes T-008.

- [ ] **Step 1: Write benches**

`crates/telemetry/benches/hot_path.rs` with `criterion`:

- `cached_counter_inc`
- `cached_gauge_set`
- `cached_histogram_observe`
- `registry_lookup_then_inc`
- `label_set_construction_then_lookup`

- [ ] **Step 2: Run benches as documentation; no pass/fail gate yet**

```
cargo bench -p nebula-telemetry --no-run
```

- [ ] **Step 3: Update README "hot path" section**

State: the cached-handle update is allocation-free; registry lookup + dynamic label construction allocates. Reference the bench numbers. Remove the "zero-copy label dimensions" claim where it conflates the two.

- [ ] **Step 4: Commit**

```
git commit -m "docs(telemetry): clarify hot-path allocation contract; add benches

Closes T-008."
```

### Task A-12: Phase A gate — clean workspace

- [ ] **Step 1: Run full suite**

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo doc --workspace --no-deps -D warnings
```

- [ ] **Step 2: Confirm Phase A scoreboard**

Map the 12 telemetry findings (T-001..T-021 minus T-013..T-017 which are layered into Phase B) to commits and confirm each closes its target. Update `docs/audits/nebula-telemetry-architecture-audit.md` with status footer per finding.

- [ ] **Step 3: Commit footer update**

```
git commit -m "docs(audit): mark Phase A telemetry findings closed"
```

---

## Phase B — Metrics catalog and safe path

Phase B treats `nebula-metrics` as the only public recording API. Every recording goes through a `MetricDescriptor` and a `SafeLabels` builder. Raw access disappears from the public surface.

### Task B-1: Define `MetricDescriptor`, `LabelSchema`, `BucketSchema`, `MetricName`

Closes the design half of J-008.

**Files:**
- Create: `crates/metrics/src/descriptor.rs`
- Modify: `crates/metrics/src/lib.rs` (re-exports)
- Test: `crates/metrics/tests/descriptor.rs` (new)

- [ ] **Step 1: Failing tests**

```rust
// crates/metrics/tests/descriptor.rs
use nebula_metrics::descriptor::{MetricDescriptor, MetricName, LabelSchema, BucketSchema, Stability};
use nebula_telemetry::metrics::MetricKind;

#[test]
fn metric_name_validates_at_const_time() {
    const N: MetricName = MetricName::new("nebula_workflow_starts_total");
    assert_eq!(N.as_str(), "nebula_workflow_starts_total");
}

#[test]
fn descriptor_carries_full_shape() {
    static D: MetricDescriptor = MetricDescriptor {
        name: MetricName::new("nebula_action_duration_seconds"),
        kind: MetricKind::Histogram,
        unit: "seconds",
        help: "End-to-end action duration in seconds.",
        labels: LabelSchema::keys(&["action_kind", "outcome"]),
        buckets: Some(BucketSchema::DEFAULT_LATENCY),
        stability: Stability::Stable,
    };
    assert_eq!(D.kind, MetricKind::Histogram);
    assert_eq!(D.labels.allowed_keys().len(), 2);
}
```

- [ ] **Step 2: Confirm fail**

- [ ] **Step 3: Implement**

```rust
// crates/metrics/src/descriptor.rs
use nebula_telemetry::metrics::MetricKind;

pub struct MetricName(&'static str);

impl MetricName {
    pub const fn new(s: &'static str) -> Self {
        // const-fn validation: nebula_ prefix, snake_case, ascii.
        // Use index-based loop because const fn is restricted.
        assert!(matches_nebula_prefix(s.as_bytes()));
        assert!(is_snake_case(s.as_bytes()));
        Self(s)
    }
    pub const fn as_str(&self) -> &'static str { self.0 }
}

pub struct LabelSchema { allowed: &'static [&'static str] }

impl LabelSchema {
    pub const fn keys(allowed: &'static [&'static str]) -> Self { Self { allowed } }
    pub const fn empty() -> Self { Self { allowed: &[] } }
    pub fn allowed_keys(&self) -> &[&'static str] { self.allowed }
    pub fn permits(&self, key: &str) -> bool { self.allowed.iter().any(|k| *k == key) }
}

pub struct BucketSchema { boundaries: &'static [f64] }

impl BucketSchema {
    pub const DEFAULT_LATENCY: BucketSchema =
        BucketSchema { boundaries: &[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0] };
    pub const DEFAULT_DURATION_LONG: BucketSchema =
        BucketSchema { boundaries: &[1.0, 5.0, 30.0, 60.0, 300.0, 1800.0, 3600.0] };
    pub fn boundaries(&self) -> &[f64] { self.boundaries }
}

pub enum Stability { Stable, Reserved }

pub struct MetricDescriptor {
    pub name: MetricName,
    pub kind: MetricKind,
    pub unit: &'static str,
    pub help: &'static str,
    pub labels: LabelSchema,
    pub buckets: Option<BucketSchema>,
    pub stability: Stability,
}
```

`matches_nebula_prefix` and `is_snake_case` are `const fn` byte-loops. Compile-time validation removes a class of M-006 / M-021 drift.

- [ ] **Step 4: Pass tests**

- [ ] **Step 5: Commit**

```
git commit -m "feat(metrics): add MetricDescriptor, MetricName, LabelSchema, BucketSchema

MetricName validates nebula_*-snake_case at const time. Foundation
for typed catalog. Closes design half of J-008 (M-006)."
```

### Task B-2: Build the typed `MetricCatalog`

Closes the enforcement half of J-008 and J-016.

**Files:**
- Create: `crates/metrics/src/catalog.rs`
- Delete (no shim): `crates/metrics/src/naming.rs`
- Test: `crates/metrics/tests/catalog_completeness.rs`

- [ ] **Step 1: Failing test**

```rust
use nebula_metrics::catalog::CATALOG;

#[test]
fn every_descriptor_has_unique_name() {
    let mut seen = std::collections::HashSet::new();
    for d in CATALOG {
        assert!(seen.insert(d.name.as_str()), "duplicate: {}", d.name.as_str());
    }
}

#[test]
fn every_histogram_has_buckets() {
    for d in CATALOG {
        if d.kind == nebula_telemetry::metrics::MetricKind::Histogram {
            assert!(d.buckets.is_some(), "{} histogram without buckets", d.name.as_str());
        }
    }
}

#[test]
fn every_total_counter_uses_total_suffix() {
    for d in CATALOG {
        if d.kind == nebula_telemetry::metrics::MetricKind::Counter {
            assert!(d.name.as_str().ends_with("_total"), "{}", d.name.as_str());
        }
    }
}

#[test]
fn duration_metrics_use_seconds_suffix() {
    for d in CATALOG {
        if d.unit == "seconds" {
            assert!(d.name.as_str().ends_with("_seconds"));
        }
    }
}
```

- [ ] **Step 2: Confirm fail**

- [ ] **Step 3: Implement**

```rust
// crates/metrics/src/catalog.rs
use crate::descriptor::*;
use nebula_telemetry::metrics::MetricKind::*;

pub static CATALOG: &[&MetricDescriptor] = &[
    &WORKFLOW_STARTS_TOTAL,
    &WORKFLOW_DURATION_SECONDS,
    // ... migrate every constant in naming.rs to a MetricDescriptor entry,
    // applying these corrections during migration:
    //  - NEBULA_CREDENTIAL_ACTIVE_TOTAL -> nebula_credential_active (gauge, unit=count)
    //    closes M-018.
    //  - NEBULA_EVENTBUS_SENT/DROPPED -> _total suffix, kind=Counter
    //    closes M-019.
    //  - Cache hits/misses/evictions -> _total counters, not gauges
    //    closes M-019.
    //  - NEBULA_ACTION_FAILURES_TOTAL -> ... drop in favor of
    //    nebula_action_outcome_total{outcome,error_class} (closes M-007, M-008).
];

pub static WORKFLOW_STARTS_TOTAL: MetricDescriptor = MetricDescriptor {
    name: MetricName::new("nebula_workflow_starts_total"),
    kind: Counter,
    unit: "count",
    help: "Total number of workflow executions started.",
    labels: LabelSchema::keys(&["workflow_kind"]),
    buckets: None,
    stability: Stability::Stable,
};

pub static WORKFLOW_DURATION_SECONDS: MetricDescriptor = MetricDescriptor {
    name: MetricName::new("nebula_workflow_execution_duration_seconds"),
    kind: Histogram,
    unit: "seconds",
    help: "Duration of completed workflow executions in seconds.",
    labels: LabelSchema::keys(&["workflow_kind", "outcome"]),
    buckets: Some(BucketSchema::DEFAULT_DURATION_LONG),
    stability: Stability::Stable,
};

// ... etc. Migrate every name from naming.rs.
```

Delete `naming.rs` in the same task. The list of constants to migrate is in `crates/metrics/src/naming.rs:11-425`.

- [ ] **Step 4: Update every call site that imported a `NEBULA_*` constant**

Workspace search: `git grep -l NEBULA_`. Replace `NEBULA_X_TOTAL` references with the descriptor (`&catalog::X_TOTAL`) at the recording call site (which becomes the descriptor-based adapter API in B-3).

- [ ] **Step 5: Run tests**

```
cargo test -p nebula-metrics --test catalog_completeness
```

- [ ] **Step 6: Commit**

```
git commit -m "feat(metrics): typed MetricCatalog replaces naming.rs constants

Every operator metric is now one MetricDescriptor entry in CATALOG.
Closes J-008 (M-006), J-016. Fixes M-018 (active_total naming),
M-019 (eventbus _total + cache event semantics), M-007 + M-008
(action outcome taxonomy)."
```

### Task B-3: `SafeLabels` builder filters before interning

Closes M-002 / J-007 / T-016.

**Files:**
- Create: `crates/metrics/src/safe_labels.rs`
- Modify: `crates/telemetry/src/labels.rs` (add `intern_pair` helper if not already present)
- Test: `crates/metrics/tests/safe_labels.rs`

- [ ] **Step 1: Failing tests**

```rust
use nebula_metrics::catalog::WORKFLOW_DURATION_SECONDS;
use nebula_metrics::safe_labels::SafeLabels;
use nebula_telemetry::metrics::MetricsRegistry;
use std::sync::Arc;

#[test]
fn safe_labels_rejects_unknown_keys_in_dev() {
    let reg = Arc::new(MetricsRegistry::new());
    let result = SafeLabels::new(&WORKFLOW_DURATION_SECONDS, &reg)
        .with("workflow_kind", "ingest")
        .with("execution_id", "abc-123") // not in schema
        .build_strict();
    assert!(result.is_err());
}

#[test]
fn safe_labels_does_not_intern_unknown_values_in_prod() {
    let reg = Arc::new(MetricsRegistry::new());
    let before = reg.interner().len();
    let _ = SafeLabels::new(&WORKFLOW_DURATION_SECONDS, &reg)
        .with("workflow_kind", "ingest")
        .with("execution_id", "abc-123")
        .build_lenient();
    let after = reg.interner().len();
    // ingest + workflow_kind interned, execution_id and abc-123 are not.
    assert!(after - before <= 2,
        "execution_id/value must NOT be interned (before={before} after={after})");
}
```

- [ ] **Step 2: Confirm fail**

- [ ] **Step 3: Implement**

```rust
// crates/metrics/src/safe_labels.rs
use crate::descriptor::MetricDescriptor;
use nebula_metrics::diagnostics::record_stripped_label;
use nebula_telemetry::error::TelemetryError;
use nebula_telemetry::labels::LabelSet;
use nebula_telemetry::metrics::MetricsRegistry;
use std::sync::Arc;

pub struct SafeLabels<'reg> {
    descriptor: &'static MetricDescriptor,
    registry: &'reg Arc<MetricsRegistry>,
    pairs: Vec<(&'static str, String)>, // strings stay un-interned until accepted
    rejected: Vec<&'static str>,
}

impl<'reg> SafeLabels<'reg> {
    pub fn new(descriptor: &'static MetricDescriptor, registry: &'reg Arc<MetricsRegistry>) -> Self {
        Self { descriptor, registry, pairs: Vec::new(), rejected: Vec::new() }
    }

    pub fn with(mut self, key: &'static str, value: impl Into<String>) -> Self {
        if self.descriptor.labels.permits(key) {
            self.pairs.push((key, value.into()));
        } else {
            self.rejected.push(key);
        }
        self
    }

    /// Strict mode: returns error if any key was rejected. Use in tests / dev.
    pub fn build_strict(self) -> Result<LabelSet, TelemetryError> {
        if !self.rejected.is_empty() {
            return Err(TelemetryError::InvalidObservation { value: f64::NAN }); // TODO use new variant LabelSchemaViolation
        }
        Ok(self.intern_now())
    }

    /// Lenient mode: rejected keys are dropped silently AND counted by the
    /// nebula_metrics_labels_stripped_total diagnostic counter (closes the
    /// observability half of J-007).
    pub fn build_lenient(self) -> LabelSet {
        for k in &self.rejected {
            record_stripped_label(self.descriptor.name.as_str(), k);
        }
        self.intern_now()
    }

    fn intern_now(self) -> LabelSet {
        let interner = self.registry.interner();
        let pairs: Vec<(&str, &str)> = self.pairs.iter()
            .map(|(k, v)| (*k, v.as_str()))
            .collect();
        interner.label_set(&pairs)
    }
}
```

Add `nebula_metrics_labels_stripped_total{descriptor, label_key}` as a self-observation counter using the same descriptor system. This closes the diagnostic side of M-009 / J-007 (silent strip becomes observable).

Add `LabelSchemaViolation { descriptor: String, key: String }` variant to `TelemetryError` in this task — strict-mode failure must be a typed error, not the placeholder above.

Delete `crates/metrics/src/filter.rs` entirely (the `LabelAllowlist` is replaced by per-descriptor `LabelSchema`). No shim per `feedback_no_shims.md`.

- [ ] **Step 4: Pass**

- [ ] **Step 5: Commit**

```
git commit -m "feat(metrics): SafeLabels filters before interning

Per-descriptor LabelSchema is enforced before LabelInterner sees a
key/value pair, closing the M-002 memory leak. Strict mode for dev,
lenient mode + nebula_metrics_labels_stripped_total for prod.
LabelAllowlist / filter.rs deleted (no shim).
Closes M-002, M-009, J-007."
```

### Task B-4: `TelemetryAdapter` becomes descriptor-only; raw access removed

Closes J-004 / M-001 / M-010.

**Files:**
- Modify: `crates/metrics/src/adapter.rs` (substantial rewrite around lines 30-260)
- Modify: `crates/metrics/src/prelude.rs:7-15`
- Modify: `crates/metrics/src/lib.rs:45-76`

- [ ] **Step 1: Failing test**

```rust
#[test]
fn prelude_does_not_re_export_metrics_registry() {
    // Compile-time check via doc-test:
    //
    // ```compile_fail
    // use nebula_metrics::prelude::MetricsRegistry;
    // ```
    //
    // Asserts the symbol is not in scope.
}

#[test]
fn adapter_record_is_only_recording_path() {
    use nebula_metrics::adapter::TelemetryAdapter;
    use nebula_metrics::catalog::WORKFLOW_STARTS_TOTAL;
    use nebula_metrics::safe_labels::SafeLabels;
    let reg = Arc::new(MetricsRegistry::new());
    let adapter = TelemetryAdapter::new(reg.clone());
    let labels = SafeLabels::new(&WORKFLOW_STARTS_TOTAL, &reg)
        .with("workflow_kind", "ingest")
        .build_lenient();
    adapter.record_counter(&WORKFLOW_STARTS_TOTAL, &labels, 1).unwrap();
}
```

- [ ] **Step 2: Confirm fail**

- [ ] **Step 3: Implement**

Replace `TelemetryAdapter` body with:

```rust
pub struct TelemetryAdapter {
    registry: Arc<MetricsRegistry>,
}

impl TelemetryAdapter {
    pub fn new(registry: Arc<MetricsRegistry>) -> Self { Self { registry } }

    pub fn record_counter(
        &self,
        descriptor: &'static MetricDescriptor,
        labels: &LabelSet,
        n: u64,
    ) -> Result<(), TelemetryError> {
        debug_assert_eq!(descriptor.kind, MetricKind::Counter);
        let counter = self.registry.counter(descriptor.name.as_str(), labels)?;
        counter.inc_by(n);
        Ok(())
    }

    pub fn set_gauge(
        &self,
        descriptor: &'static MetricDescriptor,
        labels: &LabelSet,
        value: i64,
    ) -> Result<(), TelemetryError> {
        debug_assert_eq!(descriptor.kind, MetricKind::Gauge);
        let g = self.registry.gauge(descriptor.name.as_str(), labels)?;
        g.set(value);
        Ok(())
    }

    pub fn observe_histogram(
        &self,
        descriptor: &'static MetricDescriptor,
        labels: &LabelSet,
        value: f64,
    ) -> Result<(), TelemetryError> {
        debug_assert_eq!(descriptor.kind, MetricKind::Histogram);
        let buckets = descriptor.buckets.as_ref()
            .expect("histogram descriptor must carry BucketSchema").boundaries();
        let h = self.registry.histogram_with_buckets(descriptor.name.as_str(), labels, buckets)?;
        h.observe(value); // value-validity is descriptor's job; non-finite filtered upstream
        Ok(())
    }

    /// Read-only registry handle for the exporter ONLY.
    pub(crate) fn registry(&self) -> &Arc<MetricsRegistry> { &self.registry }
}
```

Removed (deleted, not deprecated):
- `LabelAllowlist`-based methods.
- `counter`/`gauge`/`histogram` raw methods.
- Public `registry()` / `interner()` accessors.

In `lib.rs`, remove `pub use nebula_telemetry::metrics::MetricsRegistry`. The metrics crate prelude exports only descriptors, `TelemetryAdapter`, `SafeLabels`, and `MetricDescriptor` types.

For the rare custom-metric escape hatch, add `pub mod unsafe_telemetry { pub use nebula_telemetry::*; }` with `#[doc(hidden)]` and a doc comment stating: "use only when a non-cataloged custom metric is genuinely required; review for cardinality and naming manually".

- [ ] **Step 4: Touch every production call site**

Search: `git grep -nE 'MetricsRegistry::|\\.counter\(|\\.gauge\(|\\.histogram\('` in `crates/engine`, `crates/api`, `crates/resource`, `crates/eventbus`. Each call site:
- Imports the relevant descriptor from `nebula_metrics::catalog`.
- Builds `SafeLabels`.
- Calls `adapter.record_counter` / `set_gauge` / `observe_histogram`.

The audit lists the exact files: `crates/engine/src/engine.rs:47`, `crates/engine/src/runtime/runtime.rs:22`, `crates/api/src/services/webhook/transport.rs:39`, `crates/resource/src/metrics.rs:18`, `crates/engine/src/control_consumer.rs:213, 237`. None should remain after this task.

- [ ] **Step 5: Test workspace + clippy**

```
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- [ ] **Step 6: Commit**

```
git commit -m "feat(metrics)!: descriptor-only TelemetryAdapter; raw API removed

- TelemetryAdapter::new(Arc<MetricsRegistry>); no LabelAllowlist default.
- record_counter/set_gauge/observe_histogram are the only public methods.
- LabelAllowlist + filter.rs deleted.
- MetricsRegistry / Counter / Gauge / Histogram dropped from prelude.
- All production call sites migrated to descriptor + SafeLabels.

BREAKING CHANGE: Closes J-004 (M-001, M-010, M-021)."
```

### Task B-5: Reject invalid catalog names at registration; sanitization stays only for the unsafe escape hatch

Closes M-011 / J-013.

**Files:**
- Modify: `crates/metrics/src/export/prometheus.rs:20-150` (sanitization paths)
- Test: `crates/metrics/tests/descriptor.rs`

- [ ] **Step 1: Failing test**

```rust
#[test]
fn invalid_catalog_name_fails_at_compile_time() {
    // Documented as compile_fail doc-test on MetricName::new
    // for "x" (no nebula_ prefix).
}

#[test]
fn exporter_does_not_sanitize_catalog_names() {
    // After B-2 every name comes from MetricName, which is already
    // validated. Exporter must NOT re-sanitize (so any drift fails loud).
    // Confirm by injecting a bad name through unsafe_telemetry path
    // and checking the exporter returns ExportError::InvalidName.
}
```

- [ ] **Step 2-3: Implement**

Remove `sanitize_metric_name` / `sanitize_label_key` from the catalog path. Keep them callable only when consuming the `unsafe_telemetry` escape-hatch entries. Make `snapshot()` return `Result<String, ExportError>` with `ExportError::InvalidName { raw: String }` when a non-catalog name reaches the exporter.

- [ ] **Step 4: Pass**

- [ ] **Step 5: Commit**

```
git commit -m "feat(metrics): exporter no longer sanitizes catalog names

Catalog names are validated at MetricName::new (compile time).
Exporter returns ExportError::InvalidName for unrecognized raw
names. Closes J-013 (M-011)."
```

### Task B-6: Phase B gate

- [ ] **Step 1: Run full suite**

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo doc --workspace --no-deps -D warnings
```

- [ ] **Step 2: Update `docs/audits/nebula-metrics-architecture-audit.md` status footers**

- [ ] **Step 3: Commit footer update**

---

## Phase C — Boundary contracts

### Task C-1: Exporter consumes kind-tagged `RegistrySnapshot`

Closes J-019 / M-003.

**Files:**
- Modify: `crates/metrics/src/export/prometheus.rs:250-343`
- Test: `crates/metrics/tests/export_validity.rs` (new)

- [ ] **Step 1: Failing test**

```rust
#[test]
fn exporter_uses_single_iterator_no_kind_split() {
    // Smoke test: the exporter iterates RegistrySnapshot once,
    // groups by name, and any duplicate name with mismatched kind
    // is impossible because the registry rejected it at registration
    // (T-002). The exporter only needs to render.
}
```

- [ ] **Step 2-3: Replace** the three `snapshot_counters` / `snapshot_gauges` / `snapshot_histograms` calls with one loop over `registry.snapshot().iter()`. Per-name ordering remains via `BTreeMap` grouping.

- [ ] **Step 4: Pass**

- [ ] **Step 5: Commit**

```
git commit -m "feat(metrics): exporter consumes kind-tagged RegistrySnapshot

Single iterator pass; type conflicts are caught at registration
(T-002), exporter only renders. Closes J-019."
```

### Task C-2: Defensive type-conflict assertion in exporter

Closes J-009 (the metrics half).

- [ ] **Step 1: Failing test**

```rust
#[test]
fn exporter_returns_error_on_duplicate_kind_for_one_name() {
    // Inject through the unsafe_telemetry escape hatch (the only
    // way to reach this state after T-002). Confirm exporter
    // returns ExportError::TypeConflict instead of writing both
    // # TYPE families.
}
```

- [ ] **Step 2-3: Implement** — track seen `(name, kind)` pairs while rendering. On second kind for one name, return `ExportError::TypeConflict { name }`.

- [ ] **Step 4: Pass**

- [ ] **Step 5: Commit**

```
git commit -m "feat(metrics): exporter rejects same-name multi-kind families

Defensive belt-and-braces against the unsafe_telemetry escape
hatch. Closes J-009 metrics half."
```

### Task C-3: Prometheus numeric encoder for `+Inf`, `-Inf`, `NaN`

Closes J-010 (the metrics half).

**Files:**
- Modify: `crates/metrics/src/export/prometheus.rs` (add `format_prom_float`)

- [ ] **Step 1: Failing test**

```rust
use nebula_metrics::export::prometheus::format_prom_float;

#[test]
fn formats_finite() {
    assert_eq!(format_prom_float(0.0), "0");
    assert_eq!(format_prom_float(1.5), "1.5");
}
#[test]
fn formats_pos_inf_as_token() {
    assert_eq!(format_prom_float(f64::INFINITY), "+Inf");
}
#[test]
fn formats_neg_inf_as_token() {
    assert_eq!(format_prom_float(f64::NEG_INFINITY), "-Inf");
}
#[test]
fn formats_nan_as_token() {
    assert_eq!(format_prom_float(f64::NAN), "NaN");
}
```

- [ ] **Step 2-3: Implement** `pub(crate) fn format_prom_float(v: f64) -> String` and route every histogram `_sum` and bucket boundary through it.

- [ ] **Step 4: Pass**

- [ ] **Step 5: Commit**

```
git commit -m "feat(metrics): Prometheus numeric encoder for +Inf/-Inf/NaN

Histogram sum saturated at f64::MAX (A-6) but tokens still need
the Prometheus spelling. Closes J-010 metrics half."
```

### Task C-4: Output format selector (no OpenMetrics yet, but unblocked)

Closes J-026 (deferred but architecturally unblocked).

- [ ] **Step 1-3: Add** `pub enum Format { Text004, OpenMetricsV1 }` and parameterize `snapshot()` as `snapshot_with(format: Format)`. Implement only `Text004` today; `OpenMetricsV1` returns `ExportError::Unsupported`. Document the split as a feature roadmap; descriptor `unit` field is now ready to be emitted as `# UNIT` lines when V1 lands.

- [ ] **Step 4: Pass**

- [ ] **Step 5: Commit**

```
git commit -m "feat(metrics): Format enum primes OpenMetrics v1 path

Catalog descriptors carry unit + stability ready for # UNIT lines.
J-026 deferred but no longer architecturally blocked."
```

---

## Phase D — Joint tests and benches

### Task D-1: End-to-end safe-path test exercising every workflow descriptor

Closes the workflow / action half of J-018.

**File:** `crates/metrics/tests/safe_path_workflow.rs`

- [ ] **Step 1-2: Test**

For each workflow / action descriptor in `CATALOG`:

- Build adapter on a shared `Arc<MetricsRegistry>`.
- Build `SafeLabels` with valid keys.
- Record per-kind operation.
- Snapshot exporter.
- Assert each descriptor's series appears with correct `# TYPE`, `# HELP`, finite `_sum`, `+Inf == _count == sum-of-finite-buckets`.

- [ ] **Step 3: Pass**

- [ ] **Step 4: Commit**

### Task D-2: Concurrent observe + scrape

Closes the concurrency half of J-018 (M-004 / T-003 verification).

**File:** `crates/metrics/tests/concurrent_observe_scrape.rs`

- [ ] Test with N=8 writers + 1 scraper for 1000 iterations; assert histogram invariants and no `inf` literal in the output. Run under `--release` and `--test-threads=1` to surface tearing in CI.

### Task D-3: High-cardinality misuse simulation

Closes M-002 / M-009 / J-012 verification.

**File:** `crates/metrics/tests/cardinality_misuse.rs`

- [ ] Attempt to record `execution_id` through:
  - `SafeLabels::build_strict` — must error.
  - `SafeLabels::build_lenient` — must drop and increment `nebula_metrics_labels_stripped_total`.
  - `unsafe_telemetry` path — must succeed but interner-grow is on the caller's head.
  Assert that interner length stays bounded for the safe paths.

### Task D-4: Catalog round-trip test

Closes M-018, M-019, M-024 verification.

**File:** `crates/metrics/tests/descriptor_round_trip.rs`

- [ ] For every descriptor: register, snapshot, render, parse text, assert `# TYPE` matches `descriptor.kind`, `# HELP` matches `descriptor.help`, label set matches `descriptor.labels.allowed_keys()`.

### Task D-5: Hot-path benches

**File:** `crates/metrics/benches/record_hot_path.rs`

- [ ] Bench `adapter.record_counter` with cached descriptor. Assert no allocation per call (use `dhat` or count via a custom global allocator). Numbers go in the README "performance" section.

### Task D-6: Phase D gate

- [ ] Full workspace `cargo test --release`, clippy, fmt, doc. Update audit docs.

---

## Phase E — Canon and docs

### Task E-1: Output vs decision-input dataflow canon

Closes J-014.

**File:** `docs/canon/PRODUCT_CANON.md` (extend) and `docs/canon/observability.md` (new).

- [ ] **Step 1: Add canon section**

```markdown
## Observability dataflow

Two distinct data paths:

1. **Observability output path.** Runtime components record observations
   through `nebula-telemetry` primitives; `nebula-metrics` shapes them
   into the operator-facing catalog; `nebula-api` serves `/metrics`;
   Prometheus / Grafana scrape it.

2. **Decision input path.** Engine and scheduler decide from
   `nebula-system` host facts, configuration, policy, and internal state
   (queues, leases, cancellation, resources, execution state).

   The engine MUST NOT consume `/metrics` for scheduling decisions.
   Direct primitive reads on a `Counter` / `Gauge` / `Histogram` handle
   are allowed for self-observation when documented at the call site,
   but they are explicitly NOT exporter feedback.
```

- [ ] **Step 2: Cross-link from `crates/telemetry/README.md` and `crates/metrics/README.md`**

- [ ] **Step 3: Commit**

### Task E-2: Interner non-guarantee + reverse-flow contract

Closes J-012 / J-022.

- [ ] In `docs/canon/observability.md`, state explicitly:
  - "`LabelInterner` is append-only and is NOT a cardinality guard. Cardinality safety lives in `nebula-metrics::SafeLabels` per descriptor."
  - "If a future scheduler self-throttles on its own counters, it caches the `Counter` handle once at startup and calls `.get()` periodically. It does not parse `/metrics`."

- [ ] Commit.

### Task E-3: ADR-0021 (retention drop) + ADR-0022 (descriptor-only metrics API)

- [ ] Two ADRs covering Phase A retention drop (Task A-9) and Phase B descriptor-only API (Task B-4). Per `feedback_adr_revisable.md`, both supersede any prior wave-1 ADR that recommended the older shape (search `docs/adr/` for affected entries — typical candidates: telemetry primitives ADR, metrics adapter ADR).

- [ ] Commit.

### Task E-4: Lefthook + CI mirror

Per `feedback_lefthook_mirrors_ci.md`: every CI required job that this refactor introduces (e.g. new `cargo test -p nebula-metrics --test descriptor_round_trip`, doc-test for `MetricName::new` compile-fail) must also fire from `lefthook.yml` `pre-push`.

- [ ] **Step 1:** Diff `.github/workflows/*.yml` against `lefthook.yml` after the refactor.
- [ ] **Step 2:** Update `lefthook.yml` to mirror.
- [ ] **Step 3:** Commit.

### Task E-5: Final scoreboard and audit close

- [ ] Update each audit's findings table (T-001..T-021, M-001..M-026, J-001..J-026) with `status: closed` + commit SHA.
- [ ] Confirm the joint audit's 7-Point answer flips to seven Yeses. Add the new section "Resolution status (2026-..)" to `nebula-telemetry-metrics-joint-audit.md` recording the flip.
- [ ] Commit.

---

## Self-review checklist (executed before declaring the plan complete)

- **Spec coverage** — every J-### / T-### / M-### finding mapped to a phase task above. Gaps:
  - T-013..T-017 (label-related) folded into Task A-2 / A-3 / B-3.
  - T-021 (default buckets) closed by Task B-2 (each histogram descriptor names its own `BucketSchema`).
  - M-005 (label-key collision across series) folded into Task C-1 (exporter operates on descriptor-keyed schema, so cross-series collisions become impossible by construction).
  - M-012 (RED/USE catalog gaps) — partially addressed in Task B-2 by adding the missing descriptors (queue backlog, circuit breaker state, retry, fallback). If reviewing later finds gaps, they are added as Task B-2.X subtasks per descriptor.
  - M-013 / M-018 / M-019 closed by Task B-2 catalog migration.
  - M-023 closed by Task A-4 dropping `MetricsRegistry: Clone`.
  - M-025 (unit-typed adapter) — Task B-4's `record_counter` / `set_gauge` / `observe_histogram` enforce kind, but unit-typing (`Seconds(f64)`, `Bytes(u64)`) is intentionally NOT in this plan; deferred as a follow-up because it is additive and not a correctness blocker. Tracked as a "future" note in Task E-1.
  - M-026 (`MetricsRegistry: Clone` and `LabelInterner: Clone` cross-fork) closed by Task A-4 + A-2 respectively.

- **Placeholder scan** — no "TBD" / "implement later" / "similar to Task N" patterns. Each task names exact files and includes complete code for new APIs.

- **Type consistency** — `MetricKind` lives in `nebula-telemetry::metrics`; `MetricDescriptor`, `MetricName`, `LabelSchema`, `BucketSchema`, `Stability`, `SafeLabels` live in `nebula-metrics`. `BucketLayout` (the runtime, owned form) lives in `nebula-telemetry`; `BucketSchema` (the static catalog form) lives in `nebula-metrics` and is a `&'static [f64]` view consumed when calling `histogram_with_buckets`. The two are not the same type and are not interchangeable — the boundary is Task A-5 + B-1.

- **Layering check** — every task is owned by exactly one crate per the joint audit's layering matrix. No metrics policy slips into telemetry; no primitive correctness slips into metrics. The `unsafe_telemetry` re-export in `nebula-metrics` is an escape hatch with the doc-hidden visibility rule, not a layering violation.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-05-telemetry-metrics-stack-refactor.md`. Two execution options:

1. **Subagent-Driven (recommended)** — fresh subagent per task with two-stage review. Phase A and Phase B are roughly 12 + 6 = 18 subagent dispatches, mostly sequential within a phase but Phase A tasks A-1, A-2, A-7, A-10, A-11 can parallelize once A-4 lands. Use `superpowers:subagent-driven-development`.

2. **Inline Execution** — execute through the plan in this session with batch checkpoints at the end of each phase. Use `superpowers:executing-plans`.

Which approach?
