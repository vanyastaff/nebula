# `nebula-metrics` ↔ `nebula-telemetry` Merge Audit

> **Scope.** Decide whether to merge `nebula-telemetry` (primitives) and `nebula-metrics` (naming policy + Prometheus export) into a single crate, keep the split, or adopt a hybrid layout. Outcome lands as ADR-0046.
>
> **Working hypothesis.** Flat merge into `nebula-metrics`, delete `nebula-telemetry`. Subject to revision by §6 / §7 evidence. Plan: `.ai-factory/plans/telemetry-merge-audit.md`.
>
> **Out of scope.** The three pre-existing audits in `docs/audits/nebula-{metrics,telemetry,telemetry-metrics-joint}-*.md` are explicitly superseded for this branch's purpose; a fresh audit lands as a follow-up after ADR-0046. This document does not reference T-* / M-* / J-* findings.

## §1 Inventory

### Side-by-side comparison

| Column | `nebula-telemetry` | `nebula-metrics` |
|---|---|---|
| **Cargo description** | "In-memory metrics primitives for the Nebula workflow engine" | "Unified metric naming and export adapters for the Nebula workflow engine" |
| **README role tag** | "Metric Primitives (lock-free counters, gauges, histograms, label interning)" | "Metric Export and Label-Safety (Prometheus-style naming, adapter, cardinality guard)" |
| **Module count** (`src/*.rs` files) | 4 (`error.rs`, `labels.rs`, `lib.rs`, `metrics.rs`) | 6 (`adapter.rs`, `export/{mod.rs, prometheus.rs}`, `filter.rs`, `lib.rs`, `naming.rs`, `prelude.rs`) |
| **LOC** (`wc -l` on `src/`) | 1862 | 2172 |
| **Direct deps** (workspace + external) | 5: `nebula-error`, `thiserror`, `tracing`, `dashmap`, `lasso` | 2: `nebula-eventbus`, `nebula-telemetry` |
| **Dep tree depth** | Deeper (lasso → dashmap → hashbrown / lock_api) | Shallower; entirely transitive through `nebula-telemetry` |
| **Dev-deps** | 3: `insta`, `pretty_assertions`, `rstest` | 4: `tracing`, `insta`, `pretty_assertions`, `rstest` |
| **Feature flags** | None | None |
| **Typed errors** | `TelemetryError`, `TelemetryResult`, `MetricKind` | None of its own — re-exports `TelemetryError`, `TelemetryResult`, `MetricKind` from `nebula-telemetry` |
| **OpenTelemetry coupling** | None | None |
| **Prometheus coupling** | None | Yes — `export/prometheus.rs` (696 LOC, text format) |
| **Eventbus coupling** | **Zero references** (`grep -rn nebula_eventbus crates/telemetry/src/` returns empty) | Active: imports `EventBusStats` (`adapter.rs:8`), defines 4 `NEBULA_EVENTBUS_*` constants, `record_eventbus_stats(EventBusStats)` method, integration test |
| **Public types/traits/fns** | `MetricsRegistry`, `Counter`, `Gauge`, `Histogram`, `HistogramSnapshot`, `LabelInterner`, `LabelSet`, `MetricKey`, `MetricKind`, `TelemetryError`, `TelemetryResult` | `TelemetryAdapter`, `PrometheusExporter`, `snapshot()`, `content_type()`, `LabelAllowlist`, ~47 `NEBULA_*` constants, ~7 label helper fns (`control_reclaim_outcome`, `rotation_outcome`, etc.); + 8 re-exports from `nebula-telemetry` (`Counter`, `Gauge`, `Histogram`, `HistogramSnapshot`, `MetricsRegistry`, `MetricKind`, `TelemetryError`, `TelemetryResult`) |
| **Doc-comment density** (`/// + //!` lines) | ~146 | ~280 |
| **Runnable doctest fences** (count of triple-backtick lines via grep on `src/`) | 14 fence lines (~7 blocks) | 10 fence lines (~5 blocks) |
| **Workspace lints** | `[lints] workspace = true` | `[lints] workspace = true` |

**Combined:** 4034 LOC src, 10 modules across 2 crates, ~12 runnable doctests, ~426 doc-comment lines.

### Required line items

#### 1.1 Eventbus coupling

`nebula-metrics` defines four named gauges in `src/naming.rs`:

```rust
pub const NEBULA_EVENTBUS_SENT: &str = "nebula_eventbus_sent";
pub const NEBULA_EVENTBUS_DROPPED: &str = "nebula_eventbus_dropped";
pub const NEBULA_EVENTBUS_SUBSCRIBERS: &str = "nebula_eventbus_subscribers";
pub const NEBULA_EVENTBUS_DROP_RATIO_PPM: &str = "nebula_eventbus_drop_ratio_ppm";
```

And exposes the recording method on `TelemetryAdapter`:

```rust
// crates/metrics/src/adapter.rs:8
use nebula_eventbus::EventBusStats;

// crates/metrics/src/adapter.rs:238
/// Records an `EventBusStats` snapshot under standard `nebula_eventbus_*` gauges.
pub fn record_eventbus_stats(&self, stats: &EventBusStats) { ... }
```

`nebula-telemetry` has **zero references** to `nebula_eventbus` (verified by `grep -rn "nebula_eventbus" crates/telemetry/`). The coupling is **unidirectional and observability-only**: metrics observes eventbus state via a snapshot type; eventbus is unaware of metrics. No publish/subscribe involvement on either side.

**Implication for merge decision.** The `nebula-eventbus` dep moves to whichever crate carries naming/export. If merged, the dep is unchanged in shape; it does not constrain the boundary decision.

#### 1.2 Typed-error asymmetry

`nebula-telemetry::error` exposes `TelemetryError` and `TelemetryResult` (typed `thiserror`-based). `nebula-metrics` defines no error type of its own — it re-exports both via `pub use nebula_telemetry::{MetricKind, TelemetryError, TelemetryResult};` (`crates/metrics/src/lib.rs:79`). The Prometheus exporter and label allowlist paths are non-fallible (`String` outputs, panic-free strip semantics). The `TelemetryAdapter::record_eventbus_stats` is also infallible.

**Structural signal.** `nebula-metrics` is a **thin policy/naming layer** — it has no independent failure surface. The "two crates" pattern is asymmetric: one carries primitives + errors, the other carries name constants + a single bridge type (`TelemetryAdapter`).

#### 1.3 Doc-comment / doctest count

| Crate | `///` + `//!` lines | Triple-backtick fence lines | Approx. runnable doctest blocks |
|---|---|---|---|
| `nebula-telemetry` | 146 | 14 | ~7 |
| `nebula-metrics` | 280 | 10 | ~5 |
| **Combined** | **426** | **24** | **~12** |

The 426 doc-comment lines include both `//!` crate/module-level docs and `///` item docs. The runnable-block count is approximate (each triple-backtick `rust` block has an opening and closing fence).

**Implication for migration cost (input to §6 Option 2).** A merge involves rewriting any doctest example that imports `nebula_telemetry::*` to use `nebula_metrics::*`. With ~12 runnable blocks, the rewrite surface is **small and mechanical** — sed/regex-grade work, not semantic redesign.

#### 1.4 Root `examples/` workspace member

`grep -rn "nebula_metrics\|nebula_telemetry" examples/` returns **zero matches**. The root-level `examples/` workspace member contains no usage of either crate.

**Implication.** The README "Why Nebula" embed-any-crate-independently claim is **not exercised by examples** for the observability surface. Any embeddability argument for either crate must rest on README intent rather than demonstrated usage.

### Re-export surface (boundary erosion signal)

`nebula-metrics::lib.rs:42-79` re-exports the following from `nebula-telemetry`:

| Re-exported item | Origin | Note |
|---|---|---|
| `Counter` | `nebula_telemetry::metrics::Counter` | Primitive |
| `Gauge` | `nebula_telemetry::metrics::Gauge` | Primitive |
| `Histogram` | `nebula_telemetry::metrics::Histogram` | Primitive |
| `HistogramSnapshot` | `nebula_telemetry::metrics::HistogramSnapshot` | Primitive |
| `MetricsRegistry` | `nebula_telemetry::metrics::MetricsRegistry` | Primitive |
| `MetricKind` | `nebula_telemetry` | Enum |
| `TelemetryError` | `nebula_telemetry` | Error type |
| `TelemetryResult` | `nebula_telemetry` | Result alias |

Eight types — including the entire primitive surface — are pulled through `nebula-metrics`. The README explicitly recommends this path: *"Consumers should import this crate, which re-exports `Counter`, `Gauge`, `Histogram`, and `MetricsRegistry` from `nebula-telemetry` so only one import is needed."*

**Implication.** The cross-crate boundary is publicly **encouraged to be ignored** by consumers. The split exists at the Cargo level but not at the API-surface level for the canonical use case.

### Deny / wrappers status

`grep -nE 'nebula-metrics|nebula-telemetry' deny.toml` finds **no `[[bans.wrappers]]` rules** for either crate. Both are cross-cutting (per `ARCHITECTURE.md` and `deny.toml [[bans]]` allow-list). The split therefore carries **zero boundary-enforcement load** — there is no Cargo-deny rule whose validation depends on the two crates being separate.

### Canon invariant note (preserved for §3)

`crates/telemetry/README.md` references `[L1-§3.10]`:

> "This crate is the primitive layer. Naming conventions (`nebula_*` prefix), adapters, and export formats belong in `nebula-metrics` — not here. If naming helpers appear in this crate, that is a layering violation."

This is the **canonical justification** for the split. Surfaced here as input; §3 ("Why split today") will engage with it. If the merge recommendation is adopted in ADR-0046, this canon entry must be superseded explicitly (not silently ignored).

### Appendix — exact command outputs

```text
$ cargo tree -p nebula-metrics --edges normal --depth 2
nebula-metrics v0.1.0
├── nebula-eventbus v0.1.0
│   ├── futures-core v0.3.32
│   ├── parking_lot v0.12.5
│   ├── tokio v1.52.1
│   └── tokio-stream v0.1.18
└── nebula-telemetry v0.1.0
    ├── dashmap v6.1.0
    ├── lasso v0.7.3
    ├── nebula-error v0.1.0
    ├── thiserror v2.0.18
    └── tracing v0.1.44
```

```text
$ cargo tree -p nebula-telemetry --edges normal --depth 2
nebula-telemetry v0.1.0
├── dashmap v6.1.0
│   ├── cfg-if, crossbeam-utils, hashbrown 0.14, lock_api,
│   │   once_cell, parking_lot_core
├── lasso v0.7.3
│   ├── dashmap (*), hashbrown 0.14 (*)
├── nebula-error v0.1.0
│   └── nebula-error-macros (proc-macro)
├── thiserror v2.0.18
│   └── thiserror-impl (proc-macro)
└── tracing v0.1.44
    ├── pin-project-lite, tracing-attributes (proc-macro), tracing-core
```

```text
$ find crates/metrics/src crates/telemetry/src -name '*.rs' | xargs wc -l
   431 crates/metrics/src/adapter.rs
     5 crates/metrics/src/export/mod.rs
   696 crates/metrics/src/export/prometheus.rs
   208 crates/metrics/src/filter.rs
    79 crates/metrics/src/lib.rs
   738 crates/metrics/src/naming.rs
    15 crates/metrics/src/prelude.rs
    70 crates/telemetry/src/error.rs
   414 crates/telemetry/src/labels.rs
    39 crates/telemetry/src/lib.rs
  1339 crates/telemetry/src/metrics.rs
  4034 total
```

```text
$ rg "nebula_eventbus|nebula-eventbus" crates/telemetry/src/ crates/telemetry/Cargo.toml
(no matches)
```

```text
$ rg "nebula_metrics|nebula_telemetry|nebula-metrics|nebula-telemetry" examples/
(no matches)
```

## §2 Call-site map

### 2.1 Cargo.toml dependencies (workspace-wide)

| Consumer crate | `nebula-metrics` | `nebula-telemetry` |
|---|---|---|
| `nebula-api` | normal dep | normal dep |
| `nebula-engine` | normal dep + `[dev-dependencies]` | normal dep |
| `nebula-resource` | normal dep | normal dep |
| `nebula-metrics` (self) | n/a | normal dep |

No other workspace crate depends on either. SDK / plugin-sdk / sandbox / action / credential / workflow / execution / core / log / system / eventbus / validator / expression / schema / metadata / storage / storage-loom-probe / resilience / error — all do not import either crate.

### 2.2 Categorized import map

For every workspace file outside `crates/metrics/src/` and `crates/telemetry/src/` that imports either crate, classify by which crate(s) the imports come from.

#### Category A — Both crates imported in the same file (dual-import friction)

| File | `use nebula_metrics::*` | `use nebula_telemetry::*` | Use case |
|---|---|---|---|
| `crates/api/src/services/webhook/transport.rs` | `NEBULA_WEBHOOK_SIGNATURE_FAILURES_TOTAL`, `webhook_signature_failure_reason` | `metrics::MetricsRegistry` | Records named counter via Registry |
| `crates/api/tests/webhook_transport_integration.rs` (×2 sites) | naming constants | `metrics::MetricsRegistry` | Webhook integration test wiring |
| `crates/engine/src/engine.rs` | `naming::*` | `metrics::{Counter, Histogram, MetricsRegistry}` | Engine wiring + recording |
| `crates/engine/src/runtime/runtime.rs` | `naming::*` | `nebula_telemetry::*` (multi-line) | Runtime metrics recording |
| `crates/engine/tests/integration.rs` | `naming::*` | `metrics::MetricsRegistry` | Integration test wiring |
| `crates/metrics/examples/cardinality_guard.rs` | `adapter::TelemetryAdapter`, `filter::LabelAllowlist` | `metrics::MetricsRegistry` | Example program |
| `crates/metrics/examples/prometheus_export.rs` | `adapter::TelemetryAdapter`, `export::prometheus` | `metrics::MetricsRegistry` | Example program |
| `crates/resource/src/metrics.rs` | naming constants | `metrics::{Counter, MetricsRegistry}` | Resource metric recording |

**Classification.** Eight files (3 production, 1 integration test, 2 examples, 2 wiring test files) import both crates. Each of these could collapse to a single `use nebula_metrics::*` block if `MetricsRegistry`, `Counter`, `Histogram` were imported through the re-export in `crates/metrics/src/lib.rs:76`. The dual-import is a **stylistic choice**, not a semantic requirement — `nebula-metrics` already re-exports the primitives the file is reaching across the boundary for.

#### Category B — Telemetry-only direct imports (the load-bearing question)

Pre-known consumers (per plan Inputs) plus any newly discovered:

| File | Import | Production / test | Classification |
|---|---|---|---|
| `crates/api/src/state.rs:18` | `nebula_telemetry::metrics::MetricsRegistry` | Production (holds `MetricsRegistry` in `AppState`) | **Incidental** — field type is a primitive; no naming/policy involved at this layer. Could equally use `nebula_metrics::MetricsRegistry` (re-export). No semantic reason to bypass `-metrics`. |
| `crates/engine/tests/control_dispatch.rs:32` | `nebula_telemetry::metrics::MetricsRegistry` | Integration test | **Incidental** — test wires up a `MetricsRegistry` instance for engine setup. No naming constants used. |
| `crates/engine/tests/end_to_end_pipeline.rs:55` | `nebula_telemetry::metrics::MetricsRegistry` | Integration test | **Incidental** — same shape as above. |
| `crates/engine/tests/lease_takeover.rs:34` | `nebula_telemetry::metrics::MetricsRegistry` | Integration test | **Incidental** — same shape. |
| `crates/engine/tests/resource_integration.rs:27` | `nebula_telemetry::metrics::MetricsRegistry` | Integration test | **Incidental** — same shape. |
| `crates/engine/tests/retry.rs:32` | `nebula_telemetry::metrics::MetricsRegistry` | Integration test | **Incidental** — same shape. |

**Classification verdict.** All 6 telemetry-only consumers are **incidental**. None of them require primitives without naming policy as a semantic constraint. Each is reaching for `MetricsRegistry` (which is re-exported by `nebula-metrics`) and could substitute the `-metrics` import path with no behavioural change.

**No load-bearing telemetry-only consumer exists** in workspace production or test code.

#### Category C — Metrics-only direct imports (working as designed)

| File | Imports | Notes |
|---|---|---|
| `crates/engine/src/control_consumer.rs:34` | naming constants + re-exported primitives | Uses `nebula-metrics` as the single import boundary |
| `crates/engine/src/credential/refresh/coordinator.rs` (3 test sites at lines 1349, 1389, 1517) | `MetricsRegistry` (re-exported from -telemetry through -metrics) | Test code inside production file |
| `crates/engine/src/credential/refresh/metrics.rs:26` | naming constants | Production |
| `crates/engine/src/credential/refresh/reclaim.rs` (2 test sites at lines 513, 572) | `MetricsRegistry` (re-exported) | Test code inside production file |
| `crates/engine/tests/control_consumer_wiring.rs:23` | naming constants | Integration test |
| `crates/engine/tests/refresh_coordinator_chaos.rs:69` | naming constants | Integration test |

These six consumers prove the canonical "consumers should import this crate" path from the README is in active use. They demonstrate that the `-metrics` re-export layer is **adequate** for both naming-policy work and primitive-type work.

#### Category D — Doc-comment / example references inside `nebula-metrics`

| File | Reference |
|---|---|
| `crates/metrics/src/adapter.rs:53, 153, 271` | doctest examples showing how to construct a `MetricsRegistry` |
| `crates/metrics/src/export/prometheus.rs:408` | test |
| `crates/metrics/src/filter.rs:18, 150` | doc + test |
| `crates/metrics/src/naming.rs:431` | test |
| `crates/metrics/tests/integration.rs` | integration test (count of import lines suppressed for brevity) |
| `crates/telemetry/examples/basic_metrics.rs:13` | telemetry's own example |

Internal to either crate; not consumer code. Counted only for migration-cost input (≈10 internal references that change shape on rename).

### 2.3 Implication for the merge decision (input to §6)

Two findings dominate:

1. **No load-bearing telemetry-only consumer exists.** The 6 files in Category B all use `MetricsRegistry` as a primitive type, none use it in a context that excludes naming policy. Each could trivially switch to `nebula_metrics::MetricsRegistry` (re-export). The "embed primitives without policy" use case is theoretical, not load-bearing.

2. **Dual-import friction is real and affects 8 files.** Category A files all import the same conceptual surface from two crates because the re-export covers most-but-not-all paths. A single `use nebula_metrics::*` boundary would simplify each.

These two patterns argue **in favour of merge**: the cost of the split (8 dual-import call-sites) is paid daily; the value of the split (load-bearing primitive-only embedding) is unrealized in the codebase.

**Open Question 2 — finalized.** The split has no load-bearing consumer. Every telemetry-only direct import is a stylistic choice; none would break if the `-telemetry` crate were absorbed into `-metrics`.

## §3 Why split today

### 3.1 Stated rationale

The split has an **explicit public justification** in three places. Quoted verbatim:

**`crates/telemetry/README.md` Purpose section:**
> "Every crate that records metrics needs the same primitive building blocks: a thread-safe counter, a gauge, a histogram, and a way to attach label dimensions without heap allocation on the hot path. `nebula-telemetry` provides these primitives — and only these primitives. Naming conventions, export adapters, and Prometheus text generation are deliberately out of scope; they live in `nebula-metrics` one layer above. **This boundary ensures that the low-level metric types stay minimal, with no accidental coupling to naming policy or export format.**"

**`crates/telemetry/README.md` Contract section, citing canon `[L1-§3.10]`:**
> "**[L1-§3.10]** This crate is the primitive layer. Naming conventions (`nebula_*` prefix), adapters, and export formats belong in `nebula-metrics` — not here. **If naming helpers appear in this crate, that is a layering violation.**"

**`crates/metrics/src/lib.rs` crate-level docstring:**
> "Sits on top of `nebula-telemetry` primitives and adds what operators need: consistent `nebula_*` naming, a cardinality guard … In-memory primitives (`Counter`, `Gauge`, `Histogram`) remain in `nebula-telemetry`; this crate adds naming convention, a thin adapter, Prometheus text export, and label safety."

The intent is clear: *primitives stay minimal; policy/naming/export live one layer up*. This is canon-grade language ("layering violation") and would need to be **superseded explicitly** if the merge recommendation is adopted — silent violation of the L1 invariant is not on the table.

### 3.2 History — what was the split a reaction to?

Reverse-chronological commits touching either crate:

| Commit | Subject | Era |
|---|---|---|
| `fb640566` | `feat(telemetry)!: fallible registry and histogram snapshots (#645)` | Recent, breaking — refining primitive layer correctness |
| `f5d781c4` | `fix(metrics): enforce allowlist and safe Prometheus export (#403)` | Hardening policy layer |
| `dff62234` | `fix(telemetry): label dedupe, histogram bucket conflict warn, interner caveats (#402)` | Hardening primitive layer correctness |
| `05df9a0f` | `refactor(telemetry): strip to pure metrics primitives` | **Current split established** — telemetry pruned to primitives only |
| `fc6ea1ef` | `refactor(telemetry): remove unused recorder module` | Continued pruning |
| `b7659f8c` | `chore(telemetry): remove stale design/ directory` | Continued pruning |
| `ac899420` | `refactor: unify three metrics systems into single telemetry registry path (#206)` | **Pre-history** — workspace had *three* metrics systems, unified into one |

**Story.** The codebase originally had **three** ad-hoc metrics systems (#206, pre-split era). They were unified into a single telemetry registry. Then `nebula-telemetry` was stripped down to pure primitives (`05df9a0f`), and `nebula-metrics` emerged above it as the naming/policy/export layer. The split is **recent** (post-unification), and the rationale was "make the primitive layer minimal, don't repeat the multi-system mistake".

This frames the merge question as: *would re-merging primitives + policy revert toward the multi-system mess, or toward a cleaner one-system layout?* The answer depends on whether *layering* alone (without crate-level enforcement) suffices — which §3.3-3.5 below test.

### 3.3 Structural signal — `deny.toml` status

`grep -nE 'nebula-metrics|nebula-telemetry' deny.toml` returns **no matches**.

There is **no `[[bans.wrappers]]` rule** that constrains either crate. Both are members of the cross-cutting layer per `ARCHITECTURE.md` and the `deny.toml [[bans]]` allow-list, and any workspace crate may depend on them.

**Implication.** The split carries **zero boundary-enforcement load** at the workspace's mechanical-architecture layer. Whatever discipline `[L1-§3.10]` exerts is **doc-level only**. A file in `nebula-telemetry/src/` that defined `pub const NEBULA_FOO: &str = "..."` would compile, lint clean, and ship — `cargo deny check bans` would not flag it. The "layering violation" is enforced only by code review, not CI.

### 3.4 Structural signal — public-surface exclusion

`crates/sdk/Cargo.toml` workspace-internal dependencies:
```text
nebula-core, nebula-action, nebula-workflow, nebula-schema,
nebula-credential, nebula-plugin, nebula-resource, nebula-validator
```
No `nebula-metrics`, no `nebula-telemetry`.

`crates/plugin-sdk/Cargo.toml` workspace-internal dependencies:
```text
nebula-metadata, nebula-schema
```
No `nebula-metrics`, no `nebula-telemetry`.

Per `ARCHITECTURE.md`: *"Public extension surface = `nebula-sdk` + `nebula-plugin-sdk`. Third-party integrators depend on these two crates only."* Since neither sdk re-exports nor depends on the observability surface, **plugin authors and external integrators are blind to whether `-telemetry` and `-metrics` are one crate or two**.

**Implication.** Removes "external API stability" as justification for either option. The boundary decision is purely an internal-architecture concern. Any breakage scope quantified in §6 affects workspace-internal call-sites only.

### 3.5 Structural signal — typed-error asymmetry

Per §1, `nebula-telemetry` exposes `TelemetryError`/`TelemetryResult`/`MetricKind`. `nebula-metrics` defines **no error type of its own** — the Prometheus exporter, label allowlist, naming constants, and `record_eventbus_stats` are all infallible. `nebula-metrics::lib.rs:79` re-exports `TelemetryError`, `TelemetryResult`, `MetricKind` so consumers don't need to import the lower crate.

**Two readings:**

1. **Architectural reading.** Primitives have a meaningful failure surface (registry conflicts, histogram bucket validation); policy is naming/string-formatting and is correctly non-fallible. The asymmetry is principled.

2. **Vestigial reading.** `nebula-metrics` is too thin to have its own error type because it's **not actually a layer in the architectural sense** — it's a configuration cluster around `-telemetry`. The error symmetry would naturally appear if the layers were truly disjoint; its absence is a signal that the split is more file-organization than module-architecture.

The audit notes both readings. §6 must engage with them when comparing options.

### 3.6 The Telemetry-Adapter signal

`crates/metrics/src/adapter.rs` (431 LOC) defines `TelemetryAdapter` — a struct that wraps `nebula_telemetry::MetricsRegistry` and exposes "labeled record" methods for the standard `nebula_*` constants. Its only reason to exist is **the cross-crate boundary**: the adapter mediates between the naming policy in `-metrics` and the registry primitives in `-telemetry`. Inside a single crate, this adapter would be a couple of free functions on `MetricsRegistry`, not a 431-LOC type.

The bridge type is **a quantifiable cost of the split** — 431 lines of boundary maintenance code that has no purpose except keeping the two crates wired together.

### 3.7 Bottom line — does the rationale still hold?

**The stated rationale (`-telemetry` minimal, `-metrics` policy/naming) is internally coherent.** What §1, §2, and §3.3-3.6 together undermine is whether the *Cargo-level split* is the right way to express it:

- The boundary is **doc-enforced, not deny.toml-enforced** (§3.3).
- The boundary is **invisible to plugin authors** (§3.4).
- The boundary forces **a 431-LOC adapter** to bridge it (§3.6).
- The boundary causes **8 dual-import call-sites** (§2.2 Category A).
- **No production or test consumer needs primitives without policy** (§2.2 Category B verdict).
- The asymmetric error model is plausibly architectural, plausibly vestigial — readings split (§3.5).

The rationale could be expressed as `pub` / `pub(crate)` discipline inside one merged crate's `lib.rs` with comment-separated sections (per Working Hypothesis), achieving the same minimality + policy-isolation without the Cargo-level overhead. This is the structural argument §6 must evaluate.

**Open Question 1 — answered.** The documented split rationale ("primitives below, policy above") is *internally coherent but expressible without two crates*. The mechanism (Cargo split) is heavier than the constraint (don't co-locate naming with primitive types) requires.

## §4 Ecosystem reference

Three Rust observability stacks are reviewed for layering pattern. Each docs.rs landing page was fetched on **2026-05-06**.

### 4.1 `metrics-rs` — facade pattern

**Crate topology:**
- **`metrics`** (facade, 3 fundamental types: Counter / Gauge / Histogram, plus `Recorder` trait + `Key`, `Metadata`). **No registry, no exporters, no concrete collection logic.**
- **`metrics-util`** — helpers (`AtomicBucket`, `Handle`) for exporter authors.
- **`metrics-exporter-prometheus`** — Prometheus scrape endpoint.
- **`metrics-exporter-tcp`** — TCP output.

**Layering axis:** *facade vs. recorder utilities vs. exporters per backend* (3+ crates).

**Primitive ownership:** the `metrics` *facade* crate owns the primitive *types* (Counter, Gauge, Histogram). Concrete *storage* lives in whichever Recorder the application installs. Library authors emit via `metrics::counter!(name, labels)` without depending on any specific recorder.

**Embeddability:** very high. A library can depend on `metrics` only. The application chooses the recorder at composition time.

**Mapping to Nebula:** `metrics-rs::metrics` is roughly *facade + primitive types*; Nebula's `nebula-telemetry` is *concrete registry + primitive types* (more like a fixed `Recorder` impl baked into the facade). Nebula's split is **not facade-style**.

### 4.2 `opentelemetry-rust` — API/SDK/exporter pattern

**Crate topology:**
- **`opentelemetry`** (API) — trait definitions, instrument abstractions (e.g., `u64_counter`, `u64_observable_counter`).
- **`opentelemetry_sdk`** — aggregation, export intervals, configuration, concrete `MeterProvider`.
- Per-protocol exporters: **`opentelemetry-otlp`**, **`opentelemetry-prometheus`**, **`opentelemetry-zipkin`**, **`opentelemetry-stdout`**.
- **`opentelemetry-http`** — context propagation utilities.

**Layering axis:** *API vs. SDK vs. exporters per backend* (multi-axis: signal × layer; signals = traces / metrics / logs).

**Primitive ownership:** the API crate exposes *trait-level* abstractions (instrument constructors). Concrete primitives (atomic-backed counters, histograms with bucket logic) live in `opentelemetry_sdk`.

**Embeddability:** very high. Library authors depend on `opentelemetry` (API) only. Heavy, but cleanly modularized.

**Mapping to Nebula:** Nebula's split is **not API/SDK-style** either — `nebula-telemetry` carries concrete atomic-backed implementations, not API traits. The closest analog would treat `nebula-telemetry` as the SDK (concrete primitives) and `nebula-metrics` as the *exporter + naming catalog*. But there's no "API crate" above `nebula-telemetry`.

### 4.3 `prometheus` Rust crate — monolithic single-crate pattern

**Crate topology:** **One crate** ships everything:
- Metrics: `Counter`, `Gauge`, `Histogram`, `IntCounter`, `IntGauge`
- Labeled variants: `CounterVec`, `GaugeVec`, `HistogramVec`
- Core: `Registry`, `Opts`
- Export: `TextEncoder`, `ProtobufEncoder`

Optional features enable protobuf and push-gateway support, but everything is in the one crate.

**Layering axis:** *none* — single crate.

**Primitive ownership:** monolithic.

**Embeddability:** lower than `metrics-rs` / `opentelemetry-rust`. A library that depends on `prometheus` exposes the Registry shape and exporter machinery transitively.

**Mapping to Nebula:** the closest direct analog of the Working Hypothesis. A merged `nebula-metrics` crate (per Working Hypothesis flat layout) would resemble `prometheus`-the-crate in shape, scope, and audience.

### 4.4 Comparison table

| Stack | Crate count | Primary split axis | Where primitives live | Where naming/catalog lives | Where Prometheus export lives |
|---|---|---|---|---|---|
| `metrics-rs` | 3+ (`metrics`, `metrics-util`, `metrics-exporter-prometheus`, …) | Facade ↔ Recorder utils ↔ Exporter per backend | `metrics` (facade types; concrete in active Recorder) | Facade key + caller convention | Separate exporter crate |
| `opentelemetry-rust` | 5+ per signal | API ↔ SDK ↔ Exporter per backend (× signal) | `opentelemetry_sdk` | API + SDK conventions | Separate `-prometheus` exporter crate |
| `prometheus` (Rust) | **1** | None | Same crate | Same crate | Same crate |
| **Nebula today** | **2** (`nebula-telemetry`, `nebula-metrics`) | **Primitives ↔ Naming/policy/export** | `nebula-telemetry` | `nebula-metrics` | `nebula-metrics` |
| **Nebula per Working Hypothesis** | **1** (`nebula-metrics`) | None | Same crate | Same crate | Same crate |

### 4.5 Findings — does Nebula's axis match any ecosystem?

**No.** Nebula's primitives-vs-policy axis is **not represented** in any of the three reviewed stacks:

- `metrics-rs` splits along **facade vs. recorder vs. exporter** — the recorder (storage) is pluggable, not a separate primitive crate.
- `opentelemetry-rust` splits along **API vs. SDK vs. exporter** — the API exposes traits, not concrete primitives.
- `prometheus` does not split at all.

Two of the three (`metrics-rs`, `opentelemetry-rust`) achieve embeddability through the **facade/API pattern**, where library code depends on a thin abstraction and never on concrete primitives. Nebula's `nebula-telemetry` is *concrete*, not a facade — so its embeddability claim from the README differs structurally from the ecosystem's.

The remaining stack (`prometheus`) is a **single-crate monolith** that fits the Working Hypothesis's target shape.

### 4.6 Implication for the merge decision

Two ecosystem-aligned paths exist:

- **Path A — Single crate (Working Hypothesis)**, modeled on `prometheus`. Smallest refactor, simplest dep graph, idiomatic for the "registry + naming + exporter together" use case. **In scope for ADR-0046.**
- **Path B — Facade pattern**, modeled on `metrics-rs`. `nebula-telemetry` becomes a facade with `Recorder` trait; concrete Registry moves to `nebula-metrics-recorder`; exporter to `nebula-metrics-exporter-prometheus`. Decouples Nebula from a specific recorder, enables future OpenTelemetry SDK as alternative recorder. **Out of scope for ADR-0046** — it is more aggressive than the merge being decided here, and is an orthogonal future direction worth its own ADR if pursued.

The current Nebula split (primitives ↔ naming/export) is **the worst of both worlds vs. the ecosystem**: it does not provide facade-style embeddability *and* it does not consolidate the operator-facing surface. It is internally coherent but ecosystem-orphaned.

**Open Question 4 — answered.** Three reviewed stacks: one is single-crate (`prometheus`), two split along facade/API axes (`metrics-rs`, `opentelemetry-rust`). Nebula's primitive-vs-policy axis is unique. If the merge is adopted, the result fits the `prometheus` single-crate pattern. If the codebase ever wants ecosystem-grade embeddability, that is a separate (Path B) future refactor and is **not** what ADR-0046 decides.

## §5 Friction observed

Five concrete friction patterns are present in the current codebase. Quoted line numbers refer to the `docs/metrics-telemetry-merge-audit` branch state.

### 5.1 Re-exports for ergonomics — "one import is enough" pattern

`crates/metrics/src/lib.rs:42-79` re-exports 8 types from `nebula-telemetry`:
```rust
pub use adapter::TelemetryAdapter;
pub use export::prometheus::{PrometheusExporter, content_type, snapshot};
pub use filter::LabelAllowlist;
pub use naming::{ ... ~50 NEBULA_* constants ... };
// Re-export for convenience so callers can use nebula_metrics::Counter etc.
pub use nebula_telemetry::metrics::{Counter, Gauge, Histogram, HistogramSnapshot, MetricsRegistry};
pub use nebula_telemetry::{MetricKind, TelemetryError, TelemetryResult};
```

The README explicitly recommends consumers import via `-metrics`: *"Consumers should import this crate, which re-exports `Counter`, `Gauge`, `Histogram`, and `MetricsRegistry` from `nebula-telemetry` so only one import is needed."* (`crates/metrics/README.md:27-29`).

`crates/metrics/src/prelude.rs:9` does the same: `pub use nebula_telemetry::metrics::{Counter, Gauge, Histogram, MetricsRegistry};`.

**Friction.** The recommended consumer path actively bypasses the boundary the split was created to enforce. Every re-export is a maintenance cost (when `-telemetry` adds a public type, `-metrics` must mirror it or break the "one import" promise) and a structural admission that consumers want a single import surface.

**Quantification.** 8 type re-exports + ~50 constant re-exports + a dedicated `prelude` module = boundary maintenance code that exists *only* because the boundary exists.

### 5.2 Deep-path internal imports from `-metrics` into `-telemetry`'s submodules

`nebula-metrics` internals do not import `-telemetry` through its top-level re-exports — they reach into nested module paths:

| File | Import | Depth |
|---|---|---|
| `crates/metrics/src/adapter.rs:9` | `use nebula_telemetry::{LabelInterner, LabelSet, MetricKey, MetricKind, MetricsRegistry, ...}` | 1 (top-level types) |
| `crates/metrics/src/export/prometheus.rs:18` | `use nebula_telemetry::{labels::LabelInterner, metrics::MetricsRegistry};` | 2 (nested) |
| `crates/metrics/src/filter.rs:36` | `use nebula_telemetry::labels::{LabelInterner, LabelKey, LabelSet};` | 2 (nested) |
| `crates/metrics/src/naming.rs:431` | `use nebula_telemetry::metrics::MetricsRegistry;` | 2 (nested) |
| `crates/metrics/src/prelude.rs:9` | `pub use nebula_telemetry::metrics::{Counter, Gauge, Histogram, MetricsRegistry};` | 2 (nested) |

**Friction.** `nebula-metrics` is *deeply* coupled to `nebula-telemetry`'s internal module layout (`labels::`, `metrics::`). Any reorganization of `-telemetry`'s submodules cascades into compile errors across `-metrics`. This is the opposite of "minimal coupling between layers" — it's coupling on the inner shape, not just the public API.

**In a flat-merged crate** (Working Hypothesis), these imports become trivial `use crate::*` paths or are subsumed by `lib.rs` re-exports. The internal cohesion is expressible as Rust modules without crate-level boundaries.

### 5.3 `TelemetryAdapter` exists only because the split exists

`crates/metrics/src/adapter.rs` (431 LOC) defines `TelemetryAdapter` — a struct that:
1. Wraps a `nebula_telemetry::MetricsRegistry`.
2. Exposes "labeled record" methods using `nebula_*` named constants from `crates/metrics/src/naming.rs`.
3. Provides the `record_eventbus_stats(EventBusStats)` instrumentation method.
4. Carries the `LabelAllowlist` configuration.

Inside a single crate, this adapter would be **a few free functions on `MetricsRegistry`** plus an inherent `impl` block — not a 431-line type with its own constructor, builder, and tests.

**Friction quantified.** 431 LOC of boundary-bridging code with no semantic role beyond connecting two crates. Doctest examples in `adapter.rs:53, 153, 271` consist mostly of `use nebula_telemetry::metrics::MetricsRegistry; use nebula_metrics::adapter::TelemetryAdapter;` — boilerplate the merged crate would not need.

This file is the largest single concrete cost of the split.

### 5.4 Examples and doctests import from both crates

Both `crates/metrics/examples/cardinality_guard.rs` and `crates/metrics/examples/prometheus_export.rs` (lines 18-19 and 13-14 respectively) import from both:
```rust
use nebula_metrics::{adapter::TelemetryAdapter, filter::LabelAllowlist};
use nebula_telemetry::metrics::MetricsRegistry;
```

Doctest examples in `crates/metrics/src/adapter.rs:53, 153`, `crates/metrics/src/filter.rs:18`, and `crates/resource/src/metrics.rs:33` all show the same pattern — the *example code authors* wrote a dual-import as the canonical demo, even though `MetricsRegistry` is re-exported from `-metrics`.

**Friction.** Even the maintainers (writing examples and docs) reach across the boundary. The dual-import is not a careless mistake by external consumers; it is the *idiomatic example pattern in the crate's own documentation*.

### 5.5 Version-skew risk if `-telemetry` is ever published independently

Today `-metrics` declares `nebula-telemetry = { path = "../telemetry" }` (workspace-internal path dependency). The two crates ship in lockstep with the workspace, so version skew is impossible.

The README of `-telemetry` opens with embeddability framing: *"Every crate that records metrics needs the same primitive building blocks…"* — the wording suggests the crate is intended to be reusable beyond Nebula. If `-telemetry` were ever published independently to crates.io as a primitives library, the version-skew matrix becomes:

- `nebula-metrics 0.1.x` pinning `nebula-telemetry = "0.1"` would need to track every minor `-telemetry` release.
- Any breaking change in `-telemetry` (per the recent `feat(telemetry)!: fallible registry and histogram snapshots (#645)`) becomes a coordinated release.
- Consumer projects pinning both directly hit the same pin/skew matrix.

**Friction (latent).** Today this is hypothetical; root `examples/` exercises no embeddability path (§1.4) and no external publication is planned in `Cargo.toml` workspace metadata. But the README's framing suggests the option is reserved. If reserved, the friction is real-when-realized.

### 5.6 "Where does this go?" — concrete cases of cross-cutting helpers

Three helpers touch both naming and primitive concerns:

1. **`record_eventbus_stats(EventBusStats)`** (`adapter.rs:238`) — needs the 4 `NEBULA_EVENTBUS_*` constants (naming) and a registry handle (primitive). Forced into `-metrics` because it touches naming, but it pulls a registry through `TelemetryAdapter`. Currently coherent placement, but the coherence is achieved by routing through the bridge type.

2. **`LabelAllowlist`** (`crates/metrics/src/filter.rs`, 208 LOC) — strips dangerous labels before insertion. Imports `LabelInterner`, `LabelKey`, `LabelSet` from `nebula_telemetry::labels`. Conceptually it is "a primitive-layer filter applied at insertion time" — equally placeable at either side. Currently in `-metrics` per the stated rationale ("policy lives above"), but its dependency on `-telemetry`'s `labels::` submodule shape is structural, not nominal.

3. **`PrometheusExporter`** (`crates/metrics/src/export/prometheus.rs`, 696 LOC) — needs `MetricsRegistry` to read state and `LabelInterner` to resolve labels. Pure bridge from primitive state to text format. Lives in `-metrics` because Prometheus is "policy", but the function body is mostly atomic reads and string formatting. Same trade as `LabelAllowlist`.

**Friction.** The three largest pieces of "policy" code in `-metrics` (1335 LOC combined: 431 + 208 + 696) all reach into `-telemetry`'s internal module shape. They are not "policy decisions on top of stable primitive contracts"; they are tightly coupled implementations that happen to live in a different crate.

### 5.7 Aggregate friction cost

| Friction kind | Cost (LOC / call-sites) |
|---|---|
| Re-exports in `-metrics::lib.rs` and `prelude.rs` | ~60 lines + recurring sync work when `-telemetry` adds public types |
| Deep-path internal imports from `-metrics` into `-telemetry::{metrics,labels}` submodules | 5+ files |
| `TelemetryAdapter` bridge type | 431 LOC (no purpose without split) |
| Dual-import call-sites in workspace consumers | 8 files (per §2.2 Category A) |
| Maintainer-authored dual-import examples / doctests | 5+ documented examples |
| Version-skew risk (latent) | Realized only on independent publication |

**Total visible cost today: ~500 LOC of boundary-maintenance code + 8 dual-import sites + 5+ documented dual-import examples.**

This is what the codebase pays daily for the current split.

## §6 Options

Three options are considered in full. Each is described against the same axes: API delta, `deny.toml` delta, migration cost, embeddability impact, ecosystem fit (per §4), breakage classification.

**Breakage classification rules (apply consistently per plan).** Public contract = `nebula-sdk` + `nebula-plugin-sdk` re-exports + `nebula-api` HTTP surface. Per §1 / §3.4: none of these expose `nebula-metrics` or `nebula-telemetry` types. Therefore *public-contract* breakage is not in play for any option below. Workspace-internal rename of `nebula_telemetry::*` → `nebula_metrics::*` paths is a mechanical change. Tag accordingly: "internal-only breaking, public NON-breaking" — *not* "semver-major".

### 6.1 Option 1 — Keep split as-is

**Description.** Keep two crates. Optionally tighten `[L1-§3.10]` enforcement (e.g., add a CI lint that rejects `pub const NEBULA_*` in `-telemetry` source). No file-level changes; documentation is the only update.

**API delta.** None.

**`deny.toml` delta.** None. Optional addition of a CI grep-check (not a `[[bans.wrappers]]` rule).

**Migration cost.** Zero on this branch. No rename.

**Embeddability impact.** Unchanged — preserves the (today-unrealized) option to publish `nebula-telemetry` as a primitives library.

**Ecosystem fit.** Unique axis (per §4.5). Does not match `metrics-rs` facade pattern, does not match `opentelemetry-rust` API/SDK pattern, does not match `prometheus`-the-crate monolithic pattern.

**Breakage classification.** None.

**Trade-offs in favour.**
- Honors the explicit `[L1-§3.10]` canon invariant without superseding work.
- Preserves the future option to publish `-telemetry` independently as a primitives library.
- Zero-risk change; no rebase friction across the workspace.
- The 6 telemetry-only consumers (per §2.2 Category B — engine integration tests + `crates/api/src/state.rs`) keep their current imports; no test churn.

**Trade-offs against.**
- Pays the full daily friction cost: ~500 LOC boundary maintenance, 8 dual-import sites, recurring re-export sync work (per §5.7).
- The friction does not buy anything load-bearing — *no consumer needs primitives without policy* (per §2.3 finalized verdict on Open Q2).
- The `[L1-§3.10]` invariant is doc-enforced only (per §3.3 — no `deny.toml` rule). It is not a structural protection; a single careless commit can violate it without CI noticing.
- The `TelemetryAdapter` (431 LOC) exists only because of the split (per §5.3) and would not survive a merge.
- Ecosystem-orphaned (per §4.5): unique axis, not matched by `metrics-rs` / `opentelemetry-rust` / `prometheus`.

**When this is the right choice.** Reserved for a clear, near-term plan to publish `-telemetry` independently as a primitives library, OR if `[L1-§3.10]` is defensible as a hard architectural invariant that the workspace will enforce mechanically (not just doc-ically).

### 6.2 Option 2 — Full merge per Working Hypothesis

**Description.** Absorb `nebula-telemetry` into `nebula-metrics` with the flat module layout from the Working Hypothesis. Delete crate `nebula-telemetry`. Rename `TelemetryError` → `MetricsError`, `TelemetryResult` → `MetricsResult`. Drop the `TelemetryAdapter` bridge type (its methods become inherent `impl MetricsRegistry { ... }` blocks). Module structure: `lib.rs` + 11 flat modules (`counter.rs`, `gauge.rs`, `histogram.rs`, `registry.rs`, `labels.rs`, `naming.rs`, `filter.rs`, `prometheus.rs`, `eventbus.rs`, `error.rs`, `prelude.rs`).

**API delta.** Workspace-internal:
- All `use nebula_telemetry::X` paths become `use nebula_metrics::X` (top-level — flat layout means no module path segment shifts beyond the crate name).
- `TelemetryError` / `TelemetryResult` → `MetricsError` / `MetricsResult` (rename). Available everywhere `MetricsRegistry` is used.
- `TelemetryAdapter`'s methods become inherent methods on `MetricsRegistry` or free functions in the appropriate flat module.

Public contract (sdk / plugin-sdk / api HTTP): unchanged.

**`deny.toml` delta.** Remove the `[[bans.wrappers]]` (none exist for either crate today, so this is a no-op verification step). Workspace `[workspace] members` array drops `crates/telemetry`.

**Migration cost (quantified).**
- **8 files** in Category A (per §2.2 — dual-import sites): collapse two import lines into one.
- **6 files** in Category B (per §2.2 — telemetry-only): mechanical path rename.
- **1 file** in `crates/telemetry/examples/basic_metrics.rs`: deleted along with `crates/telemetry/`, content rehomed if still useful.
- **~12 doctest blocks** (per §1): rewrite import lines.
- **Deep-path internal imports** in `-metrics` (per §5.2, 5+ files): become `use crate::*`.
- **8 type re-exports** in `-metrics::lib.rs:42-79`: deleted (types are crate-local now).
- **`-metrics` `Cargo.toml`**: drop `nebula-telemetry` dep; keep `nebula-eventbus`.
- **3 consumer `Cargo.toml`s** (`api`, `engine`, `resource`): drop `nebula-telemetry` line.
- **`workspace.members`** in root `Cargo.toml`: remove `crates/telemetry`.

**Approximate file-touch count: ~25 files** (8 dual-import collapses + 6 path renames + ~5 deep-path internal imports + ~3 Cargo.toml edits + ~3 doctest edits across both crates + 1 workspace edit). Mechanical, no semantic redesign.

**Embeddability impact.** Loses the "embed `-telemetry` as primitives-only library" option. Per §1.4 / §3.4, this option is unrealized today; per §4.1-4.2, ecosystem-grade primitives embeddability is a *facade* pattern (not a separate-crate-of-concrete-primitives pattern), so the loss is not what it appears to be — it's loss of an *unrealized* option that wouldn't be ecosystem-idiomatic if pursued anyway.

**Ecosystem fit.** Matches `prometheus`-the-Rust-crate pattern (per §4.3): single crate ships Registry + naming + exporter. Most aligned of the three options to a real ecosystem precedent.

**Breakage classification.** **Internal-only breaking, public NON-breaking.** No semver bump for plugin authors or HTTP API consumers. Workspace-internal rename ~25 files, mechanical.

**Trade-offs in favour.**
- Eliminates ~500 LOC of boundary maintenance (per §5.7).
- Collapses 8 dual-import sites into single-import.
- Drops the 431-LOC `TelemetryAdapter` (per §5.3).
- Aligns with `prometheus`-the-crate ecosystem precedent (per §4.3).
- Removes the `[L1-§3.10]` doc-only invariant in favour of a structural property: there is one observability crate.
- Public contract untouched (per §3.4).
- Quantified cost is mechanical (~25 files, ~12 doctests).

**Trade-offs against.**
- Supersedes `[L1-§3.10]` — this is a deliberate canon revision and must be acknowledged in ADR-0046, not silently violated.
- The 6 engine integration tests in §2.2 Category B need a path rename. Mechanical, but their authors chose `nebula_telemetry` for a reason; that reason should be documented (§2.2 verdict: incidental, not load-bearing).
- Forecloses the option of publishing `-telemetry` as a standalone primitives library (already not on roadmap; §1.4 confirms it's not exercised).

**When this is the right choice.** When the audit finds that the split's daily friction is real and the split's stated benefit is theoretical — which is what §1-§5 collectively establish.

### 6.3 Option 3 — Hybrid

**Description.** Two flavors:

- **3a) Re-export module pattern.** Keep two crates. Add `nebula_metrics::primitives` module that pub-uses everything from `-telemetry`. Update README and prelude to make `nebula_metrics::primitives::Counter` the canonical path; deprecate the `nebula_metrics::Counter` (top-level re-export) in favour of the namespaced one.

- **3b) Feature-gated policy.** Keep two crates. Add a `policy` Cargo feature on `-metrics` that gates the naming catalog, filter, and exporter. Default-on. Consumers wanting primitives-only depend on `nebula-metrics` with `default-features = false` and get the bare re-exports.

**API delta.** Each variant changes the recommended consumer path but keeps both crates compiled and shipped.

**`deny.toml` delta.** None.

**Migration cost.**
- 3a: small. Update prelude, README, and add the `primitives` module. ~5 LOC + docs.
- 3b: medium. Add feature gating, restructure `Cargo.toml`, audit which call-sites need which feature set. Adds compile-time matrix overhead.

**Embeddability impact.** 3a — same as Option 1. 3b — *adds* a primitives-only consumption path, partially restoring an embeddability story by Cargo-feature gating.

**Ecosystem fit.** Closer to `metrics-rs`'s pattern in spirit (3b adds something facade-like), but neither variant matches an ecosystem precedent cleanly.

**Breakage classification.** 3a non-breaking. 3b non-breaking but introduces feature-flag complexity (per `feedback_idiom_currency`: feature flags should justify their existence).

**Trade-offs in favour.**
- 3a: Reduces dual-import friction (Category A) by giving everyone a single canonical import path.
- 3b: Restores a primitives-only consumption story without merging crates.
- Preserves `[L1-§3.10]` if maintainers value the invariant.

**Trade-offs against.**
- 3a is the *worst of both worlds* per memory `feedback_no_shims`: keeps the split (with its costs from §5.7) AND adds an additional re-export layer. Solves a problem that wouldn't exist if the crates merged.
- 3b adds feature-flag complexity for a use case (§1.4 / §3.4) that is unrealized in the workspace.
- Neither variant drops the `TelemetryAdapter` bridge (per §5.3) — both keep the 431-LOC cost.
- Neither aligns with an ecosystem precedent (per §4).

**When this is the right choice.** Only if Option 2 is rejected for a hard external reason (e.g., a near-term decision to publish `-telemetry` independently) AND the maintainers want to *also* improve the consumer DX without merging. Per memory `feedback_no_shims`: bridges that solve symptoms of the wrong split are a smell.

### 6.4 Side-by-side comparison

| Axis | Option 1 (keep) | Option 2 (merge) | Option 3a (re-export) | Option 3b (feature gate) |
|---|---|---|---|---|
| `Cargo.toml` workspace members | 2 | **1** | 2 | 2 |
| `TelemetryAdapter` (431 LOC) | retained | **dropped** | retained | retained |
| Re-exports in `lib.rs` (~60 LOC) | retained | **dropped** | retained + new `primitives` module | retained + feature attrs |
| `[L1-§3.10]` canon | unchanged | **superseded by ADR-0046** | unchanged | unchanged |
| Public contract breakage | none | **none** | none | none |
| Workspace-internal file touches | 0 | ~25 | ~3 (docs) | ~10 (feature attrs) |
| Doctest block edits | 0 | ~12 | ~2 | ~4 |
| Embeddability ("primitives-only library" path) | retained (unrealized) | dropped (unrealized) | retained | restored via feature flag |
| Ecosystem analog | none | **`prometheus` Rust crate** | none | weak (towards `metrics-rs`) |
| Daily friction cost (per §5.7) | retained (~500 LOC) | **eliminated** | retained + small surcharge | retained + feature-matrix surcharge |

## §7 Decision criteria

### 7.1 Tie-breakers (rule of 3)

Three criteria order the options. Each is anchored to evidence collected in §1-§5.

#### 7.1.1 Load-bearing layering

**Question.** Does the cross-crate boundary protect a property that cannot be expressed inside a single crate?

**Evidence.**
- `deny.toml` has no `[[bans.wrappers]]` rule for either crate (§3.3) — Cargo-deny is not enforcing the boundary.
- The `[L1-§3.10]` invariant ("no naming helpers in `-telemetry`") is doc-only — no CI mechanism prevents violation.
- Inside one merged crate, the same constraint is expressible as `pub`/`pub(crate)` discipline plus comment-separated `mod` declarations in `lib.rs`. Not weaker.

**Verdict.** **Not load-bearing.** The boundary protects a property the language already supports without crate separation. Favours Option 2.

#### 7.1.2 Embeddability requirement strength

**Question.** Does any consumer (today or planned) require depending on primitives without policy?

**Evidence.**
- §2.3 / Open Question 2 finalized: zero load-bearing telemetry-only consumers in workspace.
- §1.4: root `examples/` does not exercise `-telemetry` independently.
- §3.4: `nebula-sdk` and `nebula-plugin-sdk` (the public extension surface) do not depend on either crate. External integrators see neither.
- README of `-telemetry` framing suggests embeddability is *reserved as an option*, but no roadmap commitment.
- Ecosystem (per §4.5): *facade* pattern is the canonical embeddability route, not separate-primitive-crate.

**Verdict.** **Embeddability argument is unrealized and ecosystem-suboptimal.** If embeddability were a real requirement, Option 1 would still be the wrong shape (facade pattern is). Favours Option 2.

#### 7.1.3 Ecosystem inertia

**Question.** Which option matches Rust observability conventions?

**Evidence (per §4).**
- `metrics-rs`: facade ↔ exporter. Nebula's primitive-vs-policy axis does not match.
- `opentelemetry-rust`: API ↔ SDK ↔ exporter. Multi-axis. Does not match.
- `prometheus`-the-crate: single crate. **Matches Option 2 directly.**

**Verdict.** Option 2 matches one ecosystem precedent (`prometheus`); Options 1 and 3 are ecosystem-orphaned. Favours Option 2.

### 7.2 Decision matrix

| Tie-breaker | Option 1 | Option 2 | Option 3a | Option 3b |
|---|---|---|---|---|
| Load-bearing layering | ❌ no | ✅ same property, lighter mechanism | ❌ no | ❌ no |
| Embeddability requirement strength | ⚠️ retained but unrealized | ✅ unrealized → drop | ⚠️ retained but unrealized | ⚠️ partially restored, still unrealized |
| Ecosystem inertia | ❌ orphaned | ✅ matches `prometheus` | ❌ orphaned | ⚠️ weak fit |

**Three out of three tie-breakers favour Option 2.** No tie-breaker favours Options 1 or 3.

### 7.3 Recommendation

**Option 2 — full merge per Working Hypothesis.**

The audit recommends this option to ADR-0046 because:
1. The split's stated benefit (§3) is internally coherent but **not load-bearing**; it can be expressed inside one crate without boundary cost.
2. The split's daily cost (§5) is real and measurable: ~500 LOC of boundary maintenance, 8 dual-import sites, 431-LOC `TelemetryAdapter`, deep-path internal coupling that defeats the "minimal coupling between layers" goal of the split itself.
3. No production or test consumer needs primitives without policy (§2.3); the split has no load-bearing client.
4. Public contract is unaffected (§3.4); merge is **internal-only breaking, public NON-breaking**.
5. Option 2 matches `prometheus`-the-Rust-crate ecosystem precedent (§4.3); Options 1 and 3 are ecosystem-orphaned.

**Recommendation strength: strong.** All three tie-breakers (§7.1) point the same direction; no countervailing tie-breaker exists.

### 7.4 Working Hypothesis status

The Working Hypothesis stated in the plan (flat merge into `nebula-metrics`, delete `nebula-telemetry`, rename `Telemetry{Error,Result}` → `Metrics{Error,Result}`, drop `TelemetryAdapter`) is **adopted unchanged** by §6.2 / §7.3. The hypothesis is *defended on its merits* by §1-§5 evidence; it is not silently substituted by C-6 with a materially different form. Per the C-6 rule in the plan, no supersession paragraph is required.

ADR-0046 should adopt §6.2 verbatim as Decision and reference this audit doc for evidence.
