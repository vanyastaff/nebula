# `nebula-metrics` Observability Re-Audit (post-ADR-0046)

> **Scope.** Comprehensive observability re-audit of the merged `nebula-metrics`
> crate, covering implementation invariants, cardinality safety, exporter
> correctness, and the status of prior `docs/audits/nebula-{metrics,telemetry,joint}-*.md`
> findings. Follow-up to ADR-0046 §Next steps; *not* a boundary re-decision.
>
> **Working hypothesis.** ADR-0046 closed the boundary question; the merge +
> Phase X spec-correct finish (PRs #653 and #654, squash commits `160c1874`
> and `269d4207`) addressed the structural friction. This audit verifies the
> merged crate's *implementation* invariants on `main` HEAD and inherits the
> prior audit backlog with explicit Resolved / Still-open labels.

| Field | Value |
|---|---|
| Status | accepted |
| Date | 2026-05-06 |
| Branch | `docs/metrics-observability-reaudit` |
| Worktree base | `269d4207` (post-Phase-X) |
| ADR | [ADR-0046](../adr/0046-metrics-telemetry-boundary.md) |
| Prior audits | [`metrics-telemetry-merge.md`](./metrics-telemetry-merge.md), [`nebula-metrics-architecture-audit.md`](./nebula-metrics-architecture-audit.md), [`nebula-telemetry-metrics-joint-audit.md`](./nebula-telemetry-metrics-joint-audit.md) |

---

## TL;DR

- **Invariants axis:** 8/8 PASS — atomics, seqlock, registry concurrency, error
  classification, and flat-layout re-exports are all behaving as ADR-0046
  §Decision intends.
- **Cardinality axis:** 3/5 OK, 2/5 attention-needed. Today's safety is by
  callsite discipline (closed-enum reasons), not type-enforced; the long-lived
  guards (`retain_recent`, `LabelAllowlist::apply`) are written but
  unscheduled / unused in production. Both are already tracked by the in-flight
  `2026-05-05-telemetry-metrics-stack-refactor.md` plan.
- **Exporter axis:** 8/8 PASS spec-compliance, but the `# HELP` catalog misses
  10 declared `NEBULA_*` constants (rendered as `"Custom counter."`). Easy
  quick-win fixable in a follow-up PR.
- **Prior backlog (50+ findings collapsed):** ~14 closed by the merge / Phase X,
  ~6 mitigated, ~30 still open. Open severity distribution: **4 Critical**,
  **17 High**, **9 Medium**. The single largest knot is the
  `retain_recent` / `compact_interner` / `Arc<MetricsRegistry>` cluster — one
  architectural decision closes 7+ findings.

---

## 1. Inventory (current `nebula-metrics` state)

The merged crate (post-Phase-X) ships the flat module layout from ADR-0046 §Decision:

```text
crates/metrics/src/
├── lib.rs              # crate root: mod decls + flat re-exports
├── counter.rs          # Counter (AtomicU64 hot path)
├── gauge.rs            # Gauge (AtomicI64; idle-gauge invariant on set)
├── histogram.rs        # Histogram, HistogramSnapshot, DEFAULT_BUCKETS (pub(crate))
├── registry.rs         # MetricsRegistry, MetricSeries, now_ms (pub(crate))
├── labels.rs           # LabelInterner (lasso::ThreadedRodeo), LabelSet, MetricKey
├── naming.rs           # NEBULA_* constants + label helpers (pub mod)
├── filter.rs           # LabelAllowlist (cardinality guard)
├── prometheus.rs       # PrometheusExporter, snapshot, content_type
├── eventbus.rs         # record_eventbus_stats free function
├── error.rs            # MetricsError (#[non_exhaustive], Classify codes), MetricsResult
└── prelude.rs          # convenience re-exports
```

External public surface (from `crates/metrics/src/lib.rs`): `Counter`, `Gauge`,
`Histogram`, `HistogramSnapshot`, `MetricsRegistry`, `LabelInterner`, `LabelKey`,
`LabelSet`, `LabelValue`, `MetricKey`, `MetricKind`, `MetricsError`,
`MetricsResult`, `record_eventbus_stats`, `LabelAllowlist`, `PrometheusExporter`,
`content_type`, `snapshot`, `naming::*` (root-level NEBULA_* re-export). All
other modules are private (`mod`, not `pub mod`).

---

## 2. Implementation invariants

Verified against the worktree at HEAD `269d4207`.

| # | Invariant | Verdict | Evidence |
|---|---|---|---|
| I-1 | `Counter` atomics + `inc_by(0)` no-op | PASS | `counter.rs:33,46`; `counter.rs:43-45` short-circuit before `last_updated_ms.store` |
| I-2 | `Gauge::set` idle-gauge guard (no `last_updated_ms` bump on equal value) | PASS | `gauge.rs:50-55` `swap` then `if previous != v` |
| I-3 | `Histogram` seqlock + `sum_bits` + `DEFAULT_BUCKETS` validity | PASS | `histogram.rs:193,217-218` SeqCst seq pair; `histogram.rs:247-250` drift-guard retry; `histogram.rs:188-191` NaN/±∞ early-return; `DEFAULT_BUCKETS` covered by `default_bucket_table_is_valid` |
| I-4 | `MetricsRegistry` `Entry::Occupied`/`Vacant` routing + `MetricKindConflict` + `HistogramLayoutConflict` + `compact_interner` rebuild | PASS | `registry.rs:227-279`; layout-conflict at `registry.rs:264-268`; rebuild at `registry.rs:364-399` |
| I-5 | `MetricKey` identity, `LabelSet` order-invariance + last-wins dedup | PASS | `labels.rs:288-293`; sort + 2-pointer dedup at `labels.rs:210-222`; tests `label_set_order_invariant`, `label_set_dedupes_duplicate_keys_last_wins` |
| I-6 | `MetricsError` `#[non_exhaustive]` + Classify codes + recovery context | PASS | `error.rs:16-66` — codes `METRICS:IO`, `METRICS:METRIC_KIND_CONFLICT`, `METRICS:HISTOGRAM_LAYOUT_CONFLICT`, `METRICS:INVALID_HISTOGRAM_BUCKETS`; validation variants `retryable=false` |
| I-7 | `record_eventbus_stats` four sequential gauge writes + `is_finite` guard + saturating clamp | PASS (defensive) | `eventbus.rs:48-64`; `eventbus.rs:68-74` saturating `try_from(...).unwrap_or(i64::MAX)`; defensive: `EventBusStats::drop_ratio` cannot return NaN/±∞ today (`stats.rs:53-59` zero-guarded), so the `is_finite` branch is forward-compat insurance |
| I-8 | `lib.rs` flat layout, `pub use naming::*`, modules private except `naming`/`prelude` | PASS | `lib.rs:33-60` |

### 2.1 Bug-risk advisories surfaced (all advisory; non-blocking)

1. **`record_eventbus_stats` writes four gauges non-atomically** (`eventbus.rs:48-64`).
   A scrape between writes briefly violates `drop_ratio_ppm == round(dropped/(sent+dropped) * 1e6)`.
   Documented as intentional; flagged for dashboards alerting on ratio spikes.
2. **`Histogram::observe` does not bump `last_updated_ms` on NaN/±∞ early-return**
   (`histogram.rs:189-191`). Correct (no value recorded) but a histogram fed only
   non-finite samples looks idle to `retain_recent`. Worth a docstring note.
3. **`Histogram::buckets()` returns inconsistent cumulative tallies under
   concurrent observers** (relaxed loads, no seqlock). Already documented at
   `histogram.rs:277-279`. Exporters that bypass `snapshot()` see partial state.
4. **`now_ms()` clamps `u64::MAX` on `try_into` failure** (`registry.rs:36-41`).
   Theoretical only (year ~584 million).
5. **`compact_interner` requires `&mut self`** while `Arc<MetricsRegistry>` is
   the production sharing pattern. Safe by `&mut`-borrow at the type level; the
   wider implication is that `retain_recent` cannot be invoked from production
   today (see §3 D2). Same root cause as findings M-023, M-026, T-005, T-012,
   T-020, J-021, J-023, J-024 in §5.
6. **`f64 → i64` cast in ppm path** (`eventbus.rs:60`). Saturating since Rust 1.45;
   `clamp(0.0, i64::MAX as f64)` keeps it in range. No bug, just noted.
7. **`total_attempts()` in `EventBusStats`** uses unchecked `u64 + u64`
   (`stats.rs:43-45`). Cannot overflow under any realistic counter; `saturating_add`
   would be a defensive hardening.

---

## 3. Cardinality safety

| Dim | Question | Status | Note |
|---|---|---|---|
| D-1 | Are `*_labeled` callsites using user-controlled label values? | OK | Every production `*_labeled` invocation routes through a closed `pub const` enum in `crates/metrics/src/naming.rs` modules. No `USER_INPUT_UNFILTERED` callsite found. |
| D-2 | Is `MetricsRegistry::retain_recent` invoked from production? | **ATTENTION (medium)** | **No** production caller invokes it. Workspace shielded only by D-1 callsite discipline; if any callsite later admits a dynamic dimension (tenant, action key, route template), unbounded growth is one merge away with no scheduled compaction. Tracked by M-023 / M-026 / T-005 / T-012 / T-020 / J-021 / J-023 / J-024. |
| D-3 | `Spur` / `LabelKey` / `LabelValue` lifetime bounded to the metrics crate? | OK | Grep across `crates/!metrics/**` returns zero stored `Spur` outside transient `LabelSet` use. `compact_interner` invalidation caveat (`labels.rs:118`) is moot because nothing schedules it. |
| D-4 | Does every `LabelSet` go through dedup-aware construction? | OK | All callsites use `interner.label_set(&[..])` or `interner.single(k, v)`; the raw `LabelSet { pairs: ... }` constructor is private. |
| D-5 | Default policy of `LabelAllowlist` | **ATTENTION (low)** | `LabelAllowlist::default() == all()` (passthrough). `apply` invoked nowhere outside `filter.rs` tests + `cardinality_guard` example. Today's safety is by-convention (D-1), not type-enforced. The in-flight `2026-05-05-telemetry-metrics-stack-refactor.md` plan replaces this with per-descriptor `LabelSchema` (deny-by-default). |

### 3.1 Concrete callsite map (D-1 evidence)

- `crates/api/src/services/webhook/transport.rs:527-533` — `reason` from closed
  enum `webhook_signature_failure_reason::{MISSING,INVALID,MISSING_SECRET}`.
- `crates/engine/src/runtime/runtime.rs:805,2343,2426,2472` — `reason` from
  `dispatch_reject_reason::*`.
- `crates/engine/src/engine.rs:879,941-944,970-973` — `reason` from
  `engine_lease_contention_reason::{ALREADY_HELD,HEARTBEAT_LOST}`.
- `crates/engine/src/control_consumer.rs:386,398` — `outcome` from
  `control_reclaim_outcome::{RECLAIMED,EXHAUSTED}`.
- `crates/engine/src/credential/refresh/metrics.rs:62-105` — five closed enums
  (`refresh_coord_*`), pre-bound at construction.
- `crates/resource/src/metrics.rs` — unlabeled `counter()` only; no labeled record path.

---

## 4. Exporter correctness (Prometheus text format)

Verified against the [Prometheus exposition format spec](https://prometheus.io/docs/instrumenting/exposition_formats/).

| # | Property | Verdict | Evidence |
|---|---|---|---|
| E-1 | `Content-Type: text/plain; version=0.0.4; charset=utf-8` | PASS | `prometheus.rs:43`; `api/src/routes/metrics.rs:16` returns it via `nebula_metrics::content_type()` |
| E-2 | One `# HELP` + one `# TYPE` per metric family, BEFORE samples | PASS (compliance) / FAIL (coverage) | `prometheus.rs:269-365` group by exported name into `BTreeMap` and emit single HELP/TYPE per family. Coverage gap below. |
| E-3 | Sample line format (counter/gauge/histogram) | PASS | Counter/gauge: `prometheus.rs:287,310`; histogram: `prometheus.rs:341,345,350-352,355,359,362` |
| E-4 | Label rendering `{k="v",...}` with escape | PASS (advisory) | `render_labels` at `prometheus.rs:145-173`; `escape_label_value` at `233-246` escapes `\\`, `"`, `\n`, `\r`, `\t` (spec mandates only `\\`, `\n`, `"` — `\r`/`\t` are conservative additions, harmless) |
| E-5 | Name + label-key sanitization with collision disambiguation | PASS | `sanitize_identifier` at `prometheus.rs:183-199`; `allocate_exported_metric_name` at `211-231` with `__{hash:016x}` suffix; tests `snapshot_disambiguates_sanitized_*_collisions` |
| E-6 | Histogram `+Inf` cumulative count == `<name>_count` | PASS | `prometheus.rs:332-365`; cumulative_buckets() running totals + `+Inf` set to `count` (`350,355`) |
| E-7 | Histogram exporter uses `snapshot()` (seqlock), not chained `count()/sum()/buckets()` | PASS | `prometheus.rs:332` calls `hist.snapshot()` |
| E-8 | Empty registry produces parseable output | PASS | `empty_histogram_renders_all_zeros` test at `prometheus.rs:468-481` |

### 4.1 Format-compliance findings

- **E-FIND-1 (medium): `# HELP` catalog gap.** `counter_help` / `gauge_help` /
  `histogram_help` (`prometheus.rs:47-121`) miss 10 declared `NEBULA_*` constants
  — they fall through to `"Custom counter."` / `"Custom histogram."` placeholders.
  Spec-compliant (empty-ish HELP allowed) but degrades dashboards / alerting.
  Affected names: `nebula_webhook_signature_failures_total`,
  `nebula_resource_circuit_breaker_opened_total`,
  `nebula_resource_circuit_breaker_closed_total`,
  `nebula_resource_credential_rotation_skipped_total`,
  `nebula_credential_refresh_coord_claims_total`,
  `nebula_credential_refresh_coord_coalesced_total`,
  `nebula_credential_refresh_coord_sentinel_events_total`,
  `nebula_credential_refresh_coord_reclaim_sweeps_total`,
  `nebula_credential_refresh_coord_hold_duration_seconds`,
  `nebula_credential_resolver_reauth_persist_cas_exhausted_total`. **Quick win.**
- **E-FIND-2 (advisory): label-key ordering not deterministic.**
  `render_labels` (`prometheus.rs:151`) iterates `LabelSet::iter()` order. Spec
  doesn't mandate sorting, but golden tests / older parsers expect it. Two
  semantically-identical scrapes built from different insertion paths produce
  different label-key orderings. **Quick win** if `BTreeMap`-sorted output
  is preferred for cross-scrape determinism.
- **E-FIND-3 (advisory): conservative escape set.** `\r`/`\t` escaped though
  spec only mandates `\\`, `\n`, `"`. Harmless / arguably safer; no action.
- **E-FIND-4 (low): generic help fallback.** `_ => "Custom counter."` is fine
  for unknown user metrics, but for declared `NEBULA_*` constants the help
  table should be exhaustive (overlaps E-FIND-1).

---

## 5. Prior audit findings — status sweep

Re-evaluated against `269d4207`. Original IDs preserved; "Original" is the
verdict from the source audit (e.g. `nebula-metrics-architecture-audit.md`),
"Current" is the post-Phase-X verdict.

| ID | Title | Original | Current | Evidence |
|---|---|---|---|---|
| M-001 | TelemetryAdapter raw registry exposure / safety optional | CONFIRMED | **Resolved (Phase X)** | `lib.rs:33-60`; no `TelemetryAdapter` exported |
| M-002 | Allowlist filters after interning | CONFIRMED | Still open | `filter.rs:103-123` — `apply()` still takes already-built `LabelSet` |
| M-003 | Same name as multiple kinds → duplicate `# TYPE` | CONFIRMED+DEEPENED | **Resolved (registration-side)** | `registry.rs:108,125-126,231,245,271` — single `series` map with `MetricKindConflict` |
| M-004 | Histogram non-atomic snapshot | CONFIRMED | **Resolved** | `histogram.rs:188-219,228-239` — seqlock |
| M-005 | Cross-series sanitized label-key collision | PARTIAL | Still open | `prometheus.rs:145-173` — `used_keys` is per-sample only |
| M-006 | Catalog HELP gaps | CONFIRMED | Still open | `prometheus.rs::*_help`; no `MetricDescriptor` catalog. Aligned with E-FIND-1. |
| M-007 | Workflow terminal status collapse | CONFIRMED | Still open | `engine.rs:3408-3416` — only `Completed`/`Failed`, `_ => {}` |
| M-008 | Action failures count `Result::Err` only | CONFIRMED | Out of scope | `runtime.rs` — engine semantics, not metrics |
| M-009 | Allowlist global, no per-metric schema | CONFIRMED | Still open | `filter.rs:35-139` — flat `Keys` list, no `LabelSchema`. Tracked by `2026-05-05-...refactor.md`. |
| M-010 | Production uses telemetry directly | CONFIRMED | **Resolved (merge)** | `engine.rs:41,46`; `runtime.rs:17,21` import only `nebula_metrics::*` |
| M-011 | `snapshot()` infallible / silent sanitization | CONFIRMED | Still open | `prometheus.rs:175-199,258` — `sanitize_*` silent, `snapshot()` returns `String` |
| M-012 | Missing RED/USE metrics for queue/CB/scheduler | CONFIRMED | Still open | No new emission paths added |
| M-013 | `health_state` docs claim 0.5 but `Gauge` is `i64` | New / **Critical** | Still open | `gauge.rs` `AtomicI64`; naming docstring unchanged |
| M-014 | `_sum` overflow → lowercase `inf` | New / High | Mitigated (rendering) / Still open (sum) | `prometheus.rs:131-136` formats `+Inf`/`-Inf`; sum saturation unfixed |
| M-015 | Negative observations distort `_seconds` | New / High | Still open | `histogram.rs:188-203` — only `is_finite()` guard; negatives accepted |
| M-016 | Default histogram buckets wrong for long durations | New / High | Still open | No per-metric bucket schema; `DEFAULT_BUCKETS` still 0.005..10s |
| M-017 | `Counter::inc_by(0)` keeps stale series | New / High | **Resolved** | `counter.rs:42-47` short-circuits before `store(now_ms)` |
| M-018 | Cumulative cache/eventbus counts as gauges | New / High | Still open | `eventbus.rs` still uses gauges; naming unchanged |
| M-019 | `_total` suffix on gauge | New / High | Still open | `naming.rs::NEBULA_CREDENTIAL_ACTIVE_TOTAL` unchanged |
| M-020 | refresh-coord hold-duration HELP missing | New / High | Still open | `prometheus.rs::histogram_help` arms (overlaps E-FIND-1) |
| M-021 | Sanitization collides third-party with catalog | New / High | Still open | `prometheus.rs:175-199` silent rewrite; no typed `MetricName` |
| M-022 | Per-sample collision depends on Spur order | New / Med | Still open | `labels.rs:207-210` sort by Spur; `prometheus.rs:151` (overlaps E-FIND-2) |
| M-023 | Hot-path `now_ms()` cost for unreachable feature | New / Med | Still open | `registry.rs:36-41,344-349` — `retain_recent(&mut self)` vs `Arc<MetricsRegistry>` in production |
| M-024 | Registered-but-unobserved missing from scrape | New / High | Still open | No pre-registration path |
| M-025 | No `Duration`-typed observe wrapper | New / Med | Still open | `histogram.rs::observe(value: f64)` |
| M-026 | `compact_interner` desyncs cached handles | New / Med | Still open | Same `Arc` issue as M-023 |
| T-001 | `LabelSet` / `MetricKey` not registry-bound | CONFIRMED | Still open / **Critical** | `labels.rs:281+` `MetricKey` fields still public; no generation binding |
| T-002 | Same key registered as multiple kinds | CONFIRMED | **Resolved** | Same as M-003 |
| T-003 | Snapshot consistency | CONFIRMED | **Resolved** | Same as M-004 |
| T-004 | Histogram bucket layout silent override | CONFIRMED | **Resolved** | `registry.rs:265 HistogramLayoutConflict` |
| T-005 / T-020 | Retention/cached-handles + `&mut self` vs `Arc` | CONFIRMED | Still open | `registry.rs:344` still `&mut self` |
| T-006 | Counter/gauge overflow undefined | CONFIRMED | Still open | `counter.rs:46` raw `fetch_add` Relaxed |
| T-007 | `LabelInterner` not cardinality safety | CONFIRMED | Mitigated (docs) | Append-only; merge superseded boundary contract |
| T-011 | `Histogram::with_buckets` constructs outside registry | CONFIRMED | Still open | `try_with_buckets` still pub-callable |
| T-012 | `compact_interner` desyncs MetricKeys | New / **Critical** | Still open | Same as M-026 |
| T-013/14/15 | Per-call O(n) / CAS-loop / alloc | New / Med | Mitigated (T-013 via `snapshot`) / Still open | Snapshot exists; CAS loop and `LabelSet::resolve` allocations unchanged |
| T-017 | `now_ms()` lossy `as u64` | Patch | Still open | `registry.rs:36-41` |
| T-018 | `percentile(p)` no validation | Med | Not assessed | `histogram.rs::percentile` |
| T-019 | `inc_by(0)` / `Gauge::set(same)` semantics | High | **Resolved (counter)** | `counter.rs:42-47` |
| T-021 | `DEFAULT_BUCKETS` policy in primitive | Med | **Resolved (merge)** | Now intra-crate; constants live in same crate as catalog |
| J-001 | `LabelSet` / `MetricKey` not registry-bound | **Critical** | Still open | Same as T-001 |
| J-002 | Same key multi-kind | **Critical** | **Resolved** | Same as M-003 |
| J-003 | Snapshot/histogram inconsistency | High | **Resolved** | Same as M-004 |
| J-004 | Safe-path bypass / dual-import friction | High | **Resolved (merge)** | Single crate, no `TelemetryAdapter` |
| J-005 | Bucket layout silent override | High | **Resolved** | Same as T-004 |
| J-006 | Counter/gauge overflow | High | Still open | Same as T-006 |
| J-007 | `LabelAllowlist` optional/global | High | Still open | Same as M-009 |
| J-008 | Catalog enforcement absent | High | Still open | No `MetricDescriptor` |
| J-009 | Prometheus type conflicts at export | High | **Resolved (registration)** | Same as M-003 |
| J-010 | Numeric `inf` rendering | High | **Resolved** | `prometheus.rs:131-136` |
| J-011 | Retention/cached handles | High | Still open | Same as T-005 / T-020 |
| J-012 | `LabelInterner` ≠ cardinality safety | High | Mitigated (docs) | Same crate now; canon obsolete |
| J-013 | Exporter sanitization hides bugs | — | Still open | `prometheus.rs:175-199` |
| J-014 | Output vs decision dataflow canon | — | Still open (canon doc) | No canon doc landed |
| J-015 | Catalog descriptors lack bucket schemas | High | Still open | No catalog |
| J-016 | Pre-registration impossible without typed catalog | High | Still open | No catalog |
| J-017 | `last_updated_ms` semantics undefined | High | Mitigated | Single crate; semantics still undocumented across surface |
| J-018 | Tests for joint behavior absent | Med | **Resolved (merge)** | Single crate; tests in one suite |
| J-019 | Exporter does not consume primitive identity | Med | Mitigated | Exporter still groups by kind but registry is unified |
| J-020 | `Histogram::clone()` Arc-share semantics undocumented | Med | Still open | Clone docs unchanged |
| J-021 | `MetricsRegistry::clone()` vs `&mut self` | Med | Still open | Same as M-023 |
| J-022 | Reverse-flow contract for engine self-throttle | High | Still open | No canon doc |
| J-023 | Registry clone + `compact_interner` forks | **Critical** | Still open | Same as M-023 / M-026 / T-012 |
| J-024 | `LabelInterner::clone()` divergence after compaction | High | Still open | Same root cause |
| J-025 | Cached handles cross-pollute `last_updated_ms` | Med | Mitigated (M-017) | Counter `inc_by(0)` short-circuit removes one pollution path |
| J-026 | `content_type()` hardcodes legacy text format | High | Still open | `prometheus.rs::content_type()` unchanged |

### 5.1 Aggregate

- **Resolved by merge / Phase X (~14):** M-001, M-003, M-004, M-010, M-014-rendering, M-017, T-002, T-003, T-004, T-019, T-021, J-002, J-003, J-004, J-005, J-009, J-010, J-018.
- **Mitigated (~6):** T-007, J-012, J-017, J-019, J-025, M-014-sum (rendering only).
- **Still open (~30):** distribution below.

| Severity | Count | IDs |
|---|---|---|
| Critical | 4 | M-013 (`health_state` docs), J-023 (registry clone fork), T-001 / J-001 (`MetricKey` not registry-bound), T-012 (`compact_interner` desync) |
| High | 17 | M-006, M-007, M-009, M-011, M-012, M-014-sum, M-015, M-016, M-018, M-019, M-020, M-021, M-024, T-006, T-011, J-006/7/8/22/24/26 |
| Medium | 9 | M-022, M-023, M-025, M-026, T-013/14/15, T-017, T-018, J-019/20/21/25 |

The cluster around `retain_recent` / `compact_interner` / `Arc<MetricsRegistry>`
(M-023, M-026, T-005, T-012, T-020, J-021, J-023, J-024 — eight findings)
is the single largest knot. **One architectural decision** (drop retention,
move to copy-on-write registry, or replace `&mut self` with interior-mutable
sharing) closes 7+ findings at once. This is exactly the surface the
in-flight `docs/superpowers/plans/2026-05-05-telemetry-metrics-stack-refactor.md`
plan targets.

---

## 6. Recommendations

### 6.1 Quick wins (deliverable as a small follow-up PR)

- **R-1 (medium):** Fill in the `# HELP` catalog gap for the 10 declared
  `NEBULA_*` constants listed in E-FIND-1 / M-006 / M-020. Mechanical edit in
  `prometheus.rs::counter_help` / `gauge_help` / `histogram_help`. Closes
  E-FIND-1, M-006, M-020.
- **R-2 (advisory):** Sort label keys deterministically in `render_labels`
  (`prometheus.rs:145-173`). Closes E-FIND-2, M-022.
- **R-3 (advisory):** Document on `Histogram::observe` that NaN/±∞ early-return
  does not bump `last_updated_ms` (so retention sees these histograms as idle).
  Doc-only edit. Closes 2.1 #2.

### 6.2 Architectural — defer to in-flight refactor plan

The largest open knot
(`retain_recent` / `compact_interner` / `Arc<MetricsRegistry>` /
typed-identity / `LabelSchema`) is already tracked by
[`docs/superpowers/plans/2026-05-05-telemetry-metrics-stack-refactor.md`](../superpowers/plans/2026-05-05-telemetry-metrics-stack-refactor.md).
This audit confirms its scope is correct and recommends **landing that plan
before any new `*_labeled` callsites with dynamic dimensions are introduced**.
Findings closed by that plan when it lands: M-009, M-013, M-023, M-024, M-026,
T-001, T-005, T-006, T-011, T-012, T-020, J-001, J-006-8, J-011, J-021, J-023,
J-024, J-026 (≈ 17 findings).

### 6.3 Independently fixable (separate small PRs)

- **R-4 (high, M-007):** Cover all `WorkflowStatus` terminal variants in
  `engine.rs:3408-3416` (drop the `_ => {}` arm). Engine concern, not metrics.
- **R-5 (high, M-013):** Fix the `health_state` naming docstring to match
  `Gauge` being `AtomicI64` (no fractional 0.5 representation). One-line edit
  in `naming.rs`.
- **R-6 (high, M-018, M-019):** Decide cumulative vs gauge representation for
  `eventbus` and `nebula_credential_active_total`; rename if needed (semver
  consideration). Workspace-wide impact.
- **R-7 (high, M-015, M-016):** Per-metric bucket schemas + reject negative
  observations. Requires the catalog work in 6.2.
- **R-8 (medium, T-018):** Validate `Histogram::percentile(p)` argument range
  (`p ∈ [0, 1]`); typed error variant.

### 6.4 Canon / docs follow-ups

- **R-9 (J-014, J-022):** Author the canonical "metrics dataflow + reverse-flow
  for engine self-throttle" doc. Defer to `nebula-metrics` follow-up `/aif-plan`
  iteration; not a bug, but a documented invariant gap.

### 6.5 Out of scope (re-confirmed)

- **Boundary decision** (merge vs split) — closed by ADR-0046; not revisited.
- **`nebula-sdk` / `nebula-plugin-sdk` public contract** — unaffected; verified
  zero `nebula_metrics` / `nebula_telemetry` references.
- **HTTP `/metrics` endpoint behavior** — byte-format unchanged from pre-merge
  (`api/src/routes/metrics.rs:18 nebula_metrics::snapshot(registry)`).

---

## 7. Open issues recommended for tracker

If you maintain a tracker (Linear / GitHub Issues), file issues for the
quick-win cluster and the architectural cluster:

| Suggested issue | Closes | Severity |
|---|---|---|
| `metrics: fill # HELP catalog for declared NEBULA_* constants` | E-FIND-1, M-006, M-020 | medium |
| `metrics: deterministic label-key ordering in Prometheus exporter` | E-FIND-2, M-022 | low |
| `metrics: docstring `Histogram::observe` non-finite no-bump` | §2.1 #2 | advisory |
| `metrics: schedule retain_recent OR drop it (architectural)` | M-023, M-026, T-005, T-012, T-020, J-021, J-023, J-024 | high — meta |
| `metrics: per-descriptor LabelSchema (deny-by-default)` | M-009, J-007 | high — meta |
| `metrics: registry-bound MetricKey identity` | T-001, J-001 | critical |
| `metrics: health_state docstring vs i64 representation` | M-013 | critical (docs) |
| `metrics: workflow terminal status full coverage` | M-007 | high (engine) |
| `metrics: cumulative-vs-gauge naming review for eventbus + credential_active_total` | M-018, M-019 | high |
| `metrics: per-metric histogram bucket schema; reject negative observations` | M-015, M-016 | high |
| `metrics: validate Histogram::percentile argument range` | T-018 | medium |
| `metrics: canonical metrics-dataflow doc + reverse-flow contract` | J-014, J-022 | medium (docs) |

---

## 8. References

- [ADR-0046](../adr/0046-metrics-telemetry-boundary.md) — Merge `nebula-telemetry` into `nebula-metrics`.
- [`docs/audits/metrics-telemetry-merge.md`](./metrics-telemetry-merge.md) — pre-merge boundary audit (evidence base for ADR-0046).
- [`docs/audits/nebula-metrics-architecture-audit.md`](./nebula-metrics-architecture-audit.md) — pre-merge architecture audit (M-* findings).
- [`docs/audits/nebula-telemetry-metrics-joint-audit.md`](./nebula-telemetry-metrics-joint-audit.md) — pre-merge joint stack audit (J-* findings).
- [`docs/superpowers/plans/2026-05-05-telemetry-metrics-stack-refactor.md`](../superpowers/plans/2026-05-05-telemetry-metrics-stack-refactor.md) — in-flight refactor that closes the largest open cluster.
- [`crates/metrics/README.md`](../../crates/metrics/README.md) — current crate role description.
- Roadmap §M9 — Observability + DoD audit pass.
