# nebula-telemetry Architecture & Correctness Audit

Scope: `crates/telemetry` as the primitive in-memory metrics layer below `nebula-metrics`.

Evidence was taken from:

- `crates/telemetry/src/metrics.rs`
- `crates/telemetry/src/labels.rs`
- `crates/telemetry/src/error.rs`
- `crates/telemetry/src/lib.rs`
- `crates/telemetry/README.md`
- `crates/telemetry/examples/basic_metrics.rs`
- repository call sites importing `nebula_telemetry::metrics::MetricsRegistry`

This audit does not propose moving naming, Prometheus export, OTLP export, or label allowlist policy into `nebula-telemetry`. The crate should remain primitive. The findings below are about primitive invariants: identity, type safety, histogram correctness, registry behavior, concurrency contracts, and allocation claims.

## Executive Summary

`nebula-telemetry` is conceptually close to the right boundary: it contains atomics-backed `Counter`, `Gauge`, `Histogram`, a concurrent `MetricsRegistry`, and a `lasso`-backed label interner. It mostly avoids direct metric catalog and exporter implementation.

However, `nebula-metrics` cannot safely rely on it as a stable foundation until primitive invariants are encoded instead of left to convention. The largest problems are not simple missed increments. The dangerous cases are metrics that appear to work while their identity, type, or snapshot meaning is unstable.

Biggest 3 risks:

1. Metric identity is not registry-bound. `LabelSet` and `MetricKey` expose raw `Spur` symbols and can be constructed from one interner and used with another registry, causing semantic collisions, wrong label resolution, or panics.
2. A single `MetricKey` can be registered as multiple metric types, and one histogram key can be requested with incompatible bucket layouts. That breaks the primitive identity contract the exporter must depend on.
3. Histogram snapshots are weak compound reads over independent relaxed atomics, and the `snapshot_*` APIs return live handles, not immutable values. Exporters can observe count, sum, and bucket states that never existed together.

Most important design correction:

Introduce registry-owned metric identity and metric descriptors: a registry-local canonical `MetricKey`, a single map keyed by identity with an encoded `MetricKind`, immutable validated histogram bucket layout, and an explicit snapshot consistency contract. This is an API correction and architecture correction at the primitive layer. It does not require moving higher-layer metric naming or label policy downward.

## Critical Findings

| ID | Severity | Area | Problem | Failure Scenario | Recommended Fix |
|----|----------|------|---------|------------------|-----------------|
| T-001 | Critical | Metric identity | `LabelSet` and `MetricKey` are raw `Spur` containers not bound to the `LabelInterner` or `MetricsRegistry` that created them. `MetricKey` fields are public. | A `LabelSet` built with registry A is passed to registry B. The same `Spur` numeric IDs may resolve to different strings, collide with unrelated labels, or panic when exported. | API correction: make label sets registry-bound or validate an interner identity/generation before accepting them. Make `MetricKey` construction private or checked. |
| T-002 | Critical | Registry type safety | `MetricsRegistry` stores counters, gauges, and histograms in three separate `DashMap<MetricKey, _>` maps, so the same identity can exist as multiple metric types. | Two crates call `reg.counter("nebula_x")` and `reg.gauge("nebula_x")`. Exporters see one name as both counter and gauge, producing invalid or misleading metric families. | Architecture correction: store metric entries in one registry map with `MetricKind`, reject type conflicts with a primitive error. |
| T-003 | Critical | Histogram snapshot consistency | Histogram bucket counts, total count, sum, and last update are independent relaxed atomics. `snapshot_histograms()` returns live handles rather than a frozen value snapshot. | During a scrape, `total_count` is observed after an update while the matching bucket or sum is observed before it. Exported histogram count, sum, and buckets disagree. | Refactor/API correction: define weak vs strong snapshot semantics. Prefer immutable histogram snapshot structs, or document best-effort and make exporters robust. |
| T-004 | High | Histogram layout | `histogram_with_buckets_labeled()` ignores incompatible bucket layouts for an existing key and only emits a `tracing::warn!`. Bucket layout is not part of identity and no error reaches the caller. | One component expects latency buckets `[0.1, 0.5, 1.0]`; another expects `[2.0, 4.0, 8.0]`. Both receive the first layout silently and record values under the wrong distribution contract. | API correction: validate bucket layout on get-or-create and return `TelemetryError::MetricConflict` on mismatch. |
| T-005 | High | Registry retention | `retain_recent()` removes series from the registry and compacts the interner, but existing cloned metric handles remain live and can still be updated outside the registry. | A component caches `Counter`, the registry evicts its key as stale, then the component increments the cached handle. The value is invisible to future snapshots. | Architecture correction: either remove retention from this primitive API, make handles registry-attached/reinsertable, or define invalidation semantics. |
| T-006 | High | Counter/Gauge arithmetic | `Counter::inc_by()` uses `AtomicU64::fetch_add`; `Gauge::inc/dec` use `AtomicI64::fetch_add/fetch_sub`. Overflow and underflow wrap in release builds and are not documented. | A long-running counter reaches `u64::MAX` and wraps to 0. A gauge near `i64::MAX` increments and becomes negative. Dashboards interpret reset or impossible capacity. | API correction: define overflow behavior. Use checked/saturating arithmetic, return errors, or document explicit wrapping as unacceptable for operator metrics. |
| T-007 | High | Label interning | `LabelInterner` is public and append-only until manual compaction. It deduplicates strings but is not cardinality safety. | Plugin code interns 100k unique execution IDs. Memory grows permanently until retention/compaction and may never be reclaimed if labels remain referenced. | Documentation/test only plus API correction: document that interning is not a cardinality guard, prefer registry-owned label construction, and test high-cardinality misuse. Enforcement belongs in `nebula-metrics`. |
| T-008 | High | Hot-path allocation contract | Docs claim zero-copy label dimensions and no heap allocation on the hot path, but `LabelInterner::label_set()` allocates a `Vec`, interns strings, and registry lookup clones keys. The allocation-free path is only cached handle updates. | Callers repeatedly build dynamic label sets and call registry get-or-create in action hot paths, believing telemetry is allocation-free. | Documentation/test only: define "hot path" precisely. Add allocation benchmarks/tests for cached handles vs registry lookup plus label construction. |
| T-009 | High | Error model | `TelemetryError` only contains `Io`. Primitive invalid states currently panic, warn, or silently drop data: invalid buckets panic, bucket conflicts warn, non-finite observations are ignored. | `nebula-metrics` cannot distinguish type conflict, invalid histogram layout, or rejected observation, so tests and operators cannot assert failure modes. | API correction: add primitive error variants and fallible constructors/get-or-create APIs for invalid primitive state. |
| T-010 | High | Layering/docs | Implementation docs and examples leak upward policy: examples use `nebula_*` names, registry docs mention Prometheus/OTLP, and histogram defaults are described as Prometheus-style. | Developers learn to use `nebula_telemetry::MetricsRegistry` directly for canonical Nebula metrics, bypassing `nebula-metrics` naming and label safety. | Documentation/test only plus small refactor: keep primitive examples policy-neutral and reserve naming/export guidance for `nebula-metrics`. |

## Architecture Risks

### Primitive boundary is right, but the public surface is too porous

The crate correctly avoids a metric catalog, a Prometheus exporter, and OTLP implementation. `crates/telemetry/src/lib.rs` states that naming, export adapters, and Prometheus text generation live in `nebula-metrics`.

The problem is that the primitive API makes invalid primitive states easy:

- `MetricsRegistry` exposes raw `counter`, `gauge`, `histogram`, and labeled variants that accept arbitrary names and labels.
- `MetricKey` has public `name` and `labels` fields in `crates/telemetry/src/labels.rs`.
- `LabelSet` contains only a `Vec<(Spur, Spur)>`; it has no registry/interner identity.
- `LabelInterner` is public and can be used directly by any caller.

Direct registry access is not itself a telemetry-layer bug. But because the registry does not encode type conflicts, bucket layout conflicts, or interner ownership, higher layers cannot reliably build stronger semantics on top.

### The registry does not have a single metric identity table

`MetricsRegistry` has separate maps:

- `counters: DashMap<MetricKey, Counter>`
- `gauges: DashMap<MetricKey, Gauge>`
- `histograms: DashMap<MetricKey, Histogram>`

This allows one `MetricKey` to be present in all three maps. A primitive registry should know that `(name, labels)` identifies a metric family/series with exactly one type. Naming policy belongs above this crate, but type identity belongs here.

### Histogram descriptors are not first-class

Bucket boundaries are stored inside each `Histogram`, but they are not part of registry conflict detection. `histogram_with_buckets_labeled()` returns the already registered histogram when the same key is requested with different boundaries and only logs a warning.

This is worse than returning an error because the caller receives a valid `Histogram` handle and proceeds as though the requested layout was honored.

### Retention conflicts with cached-handle usage

The crate encourages cached handle updates for low overhead. `Counter`, `Gauge`, and `Histogram` are cloneable `Arc` handles. But `MetricsRegistry::retain_recent()` removes stale entries from the maps and compacts interned labels. A cached handle can still be updated after removal, but that update no longer reaches registry snapshots.

This makes retention unsafe unless the user also proves no old handles exist. That contract is not encoded or documented.

### Snapshot APIs are named stronger than they are

`snapshot_counters`, `snapshot_gauges`, and `snapshot_histograms` clone map entries into vectors, but the metric values remain live atomics. A "snapshot" can change after the vector is returned. For counters and gauges this may be acceptable if documented as best effort. For histograms, the exporter can read mutually inconsistent buckets, count, and sum.

## Primitive Correctness Review

### Counter

Evidence:

- `Counter` stores `Arc<AtomicU64>` and `Arc<AtomicU64>` for `last_updated_ms`.
- `Counter::inc()` and `Counter::inc_by()` use `fetch_add(..., Ordering::Relaxed)`.
- `Counter::get()` uses `load(Ordering::Relaxed)`.

What works:

- Cloned `Counter` handles share the same atomic value.
- Concurrent increments of a single counter are not lost under normal atomic semantics.
- Negative increments are impossible through the public API because the increment amount is `u64`.
- A cached counter handle update is heap-allocation-free.

Risks:

- Overflow wraps silently in optimized Rust builds. There is no saturation, error, or documented reset behavior.
- `last_updated_ms` is a separate relaxed write after the value update. A snapshot or retention pass can observe a new value with an old timestamp or vice versa.
- `now_ms()` uses wall-clock time. Clock changes can make age comparisons non-monotonic.
- `inc_by(0)` refreshes `last_updated_ms`. That may or may not be intended.

Required production contract:

- Either counters saturate, return error on overflow, or explicitly document wrapping as not acceptable for long-lived processes.
- Memory ordering can remain relaxed for the value if the contract says counter reads are eventually consistent and independent.
- Retention must not depend on stronger ordering than the counter provides.

### Gauge

Evidence:

- `Gauge` stores `Arc<AtomicI64>`.
- `inc()` and `dec()` use `fetch_add(1)` and `fetch_sub(1)`.
- `set()` stores an absolute value.

What works:

- Gauges can represent negative values because the type is `i64`.
- Concurrent increments and decrements are atomic RMW operations.
- Cloned handles share the same value.

Risks:

- Overflow and underflow wrap in release builds.
- The semantic race between `set()` and `inc()/dec()` is not defined. If one thread sets a gauge while another increments, either operation may "win" in a way that is atomically valid but semantically surprising.
- The crate does not document whether negative values are universally valid or simply allowed by the primitive.
- A gauge can be misused as an event counter because the registry has no metric descriptor or kind-specific semantics beyond the type chosen by the caller.

Required production contract:

- Define overflow/underflow behavior.
- Document `set` vs arithmetic concurrency semantics.
- Keep gauge unit and meaning out of this crate, but make the primitive numeric behavior explicit.

### Histogram

Evidence:

- `Histogram::with_buckets()` validates bucket boundaries with `assert!`, not `Result`.
- Boundaries must be non-empty, positive, finite, and strictly increasing.
- `observe()` silently returns on non-finite values.
- Observations equal to a boundary map to that boundary's bucket via `binary_search_by`.
- Bucket counts, total count, and sum are separate atomics.
- `buckets()` allocates and returns cumulative finite buckets.
- There is an implicit overflow bucket in `counts`, but `buckets()` does not expose a `+Inf` boundary. Exporters must combine finite buckets with `count()`.

What works:

- Invalid bucket boundaries are rejected early, although by panic.
- Boundary equality is handled as `value <= boundary`.
- Observations above the last finite bucket are counted in an overflow bucket.
- Concurrent observations increment atomic counters and update sum via atomic f64 bit update.

Risks:

- Invalid bucket config panics in library code, contrary to the workspace norm of typed errors in libraries.
- The struct comment says counts are cumulative, but storage is non-cumulative; cumulative values are computed in `buckets()`.
- Non-finite observations are silently dropped. This prevents poison sums, but it hides bad caller behavior.
- Negative finite observations are accepted and fall into the first bucket. That may be valid for a generic distribution primitive, but it conflicts with the positive default bucket layout and is undocumented.
- `percentile(p)` documents `0.0-1.0` but does not validate `p`. NaN, negative values, and values greater than 1 can produce misleading results.
- Snapshot consistency is weak: a scrape can observe total count, bucket counts, and sum from different moments.
- The default bucket docs call them Prometheus-style, which leaks exporter vocabulary into the primitive layer.

Required production contract:

- Bucket layout is a primitive descriptor and must be immutable and conflict-checked.
- Invalid bucket layouts should be fallible primitive errors, not panics.
- Non-finite and negative observation behavior must be documented and tested.
- Histogram snapshot consistency must be explicit.

## Atomic and Concurrency Review

Relaxed atomics are reasonable for independent metric counters when the contract is "eventually visible, no ordering with application memory." They are not automatically sufficient for compound invariants.

Current atomic operations:

| Type | Operation | Ordering | Assessment |
|------|-----------|----------|------------|
| `Counter.value` | `fetch_add`, `load` | Relaxed | OK for independent counter increments if overflow is handled/documented. |
| `Counter.last_updated_ms` | `store`, `load` | Relaxed | OK only as approximate metadata. Not safe as exact retention evidence under races or clock changes. |
| `Gauge.value` | `fetch_add`, `fetch_sub`, `store`, `load` | Relaxed | Atomic but semantics of `set` racing arithmetic are undefined. Overflow wraps. |
| `Gauge.last_updated_ms` | `store`, `load` | Relaxed | Same approximate metadata caveat. |
| `Histogram.counts[i]` | `fetch_add`, `load` | Relaxed | OK for per-bucket increments; not a consistent family snapshot. |
| `Histogram.total_count` | `fetch_add`, `load` | Relaxed | OK independently; can disagree with bucket loads during scrape. |
| `Histogram.sum_bits` | atomic f64-bit update, `load` | Relaxed | Avoids lost updates, but sum ordering is nondeterministic and not atomic with count/buckets. |
| `Histogram.last_updated_ms` | `store`, `load` | Relaxed | Approximate only. |

Main concurrency risks:

- A histogram observation updates bucket, count, sum, and timestamp in multiple operations. Any snapshot can see a prefix or interleaving.
- `snapshot_*` iterates `DashMap` while metrics may be registered. That is memory-safe, but ordering and inclusion are best-effort.
- `retain_recent()` requires `&mut self`, so it does not race with registry methods on the same `MetricsRegistry` value. But cloned metric handles can still be updated after eviction.
- `LabelInterner::resolve()` can panic if called with a `Spur` from another interner. The registry APIs do not prevent that.

No evidence supports a strong snapshot contract today. The primitive layer should either provide immutable value snapshots or explicitly document weak consistency so `nebula-metrics` does not assume exact Prometheus families.

## MetricsRegistry Review

Evidence:

- `MetricsRegistry` owns a `LabelInterner` and three `DashMap`s.
- `counter`, `gauge`, and `histogram` call `entry(...).or_insert_with(...)`.
- Labeled methods accept a borrowed `LabelSet` and clone it into the `MetricKey`.
- `snapshot_*` clones keys and handles into vectors.
- `retain_recent()` removes entries by `last_updated_ms()` and calls `compact_interner()`.

What works:

- Same-type concurrent get-or-create is handled by `DashMap::entry`.
- Cloning the registry shares all maps and the interner.
- Label order normalization works within a single interner for label sets produced by that interner.

Primitive correctness gaps:

- Same key can be registered as counter, gauge, and histogram simultaneously.
- Same histogram key can be associated with incompatible requested bucket layouts.
- There is no global metric descriptor table.
- There is no deterministic snapshot ordering.
- Snapshot returns live metric handles, not frozen values.
- The registry can grow without bound. Cardinality budgets belong in `nebula-metrics`, but the primitive docs must warn that growth is unbounded.
- Retention can orphan cached handles.
- `metric_count()` sums maps, so the same identity registered as multiple types is counted multiple times.

Expected registry invariants:

- A registry-local metric identity maps to one metric type.
- A histogram identity maps to one immutable bucket layout.
- `LabelSet` accepted by a registry was produced by the same registry/interner or has been safely re-interned.
- Snapshot semantics are explicit: best-effort live handles or immutable values.

## MetricKey / LabelSet Identity Review

Evidence:

- `LabelKey` and `LabelValue` are type aliases for `lasso::Spur`.
- `LabelSet` stores `Vec<(LabelKey, LabelValue)>`.
- `LabelInterner::label_set()` interns keys and values, sorts by key symbol, and deduplicates duplicate keys with last value wins.
- `MetricKey` has public `name: Spur` and `labels: LabelSet`.

What works:

- Within one interner, the same label strings in different input order produce the same canonical `LabelSet`.
- Duplicate key behavior is deterministic for a single input order: last value wins.
- Empty label sets are supported.

Risks:

- Identity is only correct relative to one interner, but the types do not encode that relationship.
- Sorting by `Spur` is deterministic within one interner but not a semantic lexical order.
- Public `MetricKey` fields allow callers to construct keys from arbitrary symbols.
- `LabelSet::resolve()` calls `interner.resolve()`, which panics if a symbol does not belong to that interner.
- Empty label keys and invalid label names are possible. Prometheus validity belongs above this crate, but primitive docs should say no exporter validity is implied.
- Duplicate label keys are silently collapsed. That is safer than duplicate sample labels, but callers may not realize a dimension was overwritten.

Cross-interner example:

```rust
let reg_a = MetricsRegistry::new();
let reg_b = MetricsRegistry::new();

let labels_from_a = reg_a.interner().label_set(&[("status", "ok")]);
reg_b.counter_labeled("requests", &labels_from_a).inc();
```

The current API accepts this. The symbols in `labels_from_a` are registry-local numeric IDs, not portable semantic labels.

Recommended design:

- Make `LabelSet` opaque and registry-bound, or store an interner generation/registry ID.
- Make registry labeled APIs accept raw `(&str, &str)` pairs and canonicalize internally, or accept only a `BoundLabelSet<'registry>` tied to the registry lifetime.
- Keep Prometheus label validation out of this crate, but reject identity-unsafe label sets here.

## LabelInterner Review

Evidence:

- `LabelInterner` wraps `lasso::ThreadedRodeo`.
- Docs state it is append-only until registry compaction.
- `resolve()` panics for foreign symbols; `try_resolve()` exists.
- `filter_label_set()` interns allowed keys and filters by interned key ID.

What works:

- Repeated strings deduplicate.
- Concurrent interning is delegated to `ThreadedRodeo`.
- `try_resolve()` provides a non-panicking primitive for callers that use it.

Risks:

- Exposing the interner encourages callers to treat interned symbols as portable IDs.
- High-cardinality labels still allocate and remain interned. Interning is not a cardinality guard.
- `filter_label_set()` is a low-level helper that can support higher-layer allowlists, but it also shows how easy it is to build allowlist-like policy against registry-local symbols. It should be clearly documented as a utility, not enforcement.
- Label resolution during scrape can allocate because `LabelSet::resolve()` returns a `Vec`.
- Compaction remaps registry keys but cannot update external `LabelSet` or `MetricKey` values held by callers.

Recommended design:

- Keep interning in telemetry, but reduce direct public reliance on `Spur`.
- Document interner lifetime and non-portability.
- Add high-cardinality misuse tests to prove the failure mode and guide `nebula-metrics` enforcement.

## Snapshot Semantics Review

Current snapshot APIs:

- `snapshot_counters() -> Vec<(MetricKey, Counter)>`
- `snapshot_gauges() -> Vec<(MetricKey, Gauge)>`
- `snapshot_histograms() -> Vec<(MetricKey, Histogram)>`

These APIs snapshot registry membership approximately, not metric values. Each returned handle can change after the vector is returned.

Risks:

- Exporters may assume frozen values.
- Histogram families can be internally inconsistent.
- Snapshot ordering is not deterministic because `DashMap` iteration order is not stable.
- Type conflicts across maps are not visible as conflicts; they are simply separate snapshot lists.
- Label resolution can panic if a key contains foreign symbols.
- Concurrent registration can appear or not appear depending on iteration timing.

Recommended contract:

- Define snapshot as either "weak live-handle enumeration" or "immutable value snapshot."
- For exporter use, prefer immutable structs such as:
  - `CounterSnapshot { key, value, last_updated_ms }`
  - `GaugeSnapshot { key, value, last_updated_ms }`
  - `HistogramSnapshot { key, boundaries, cumulative_buckets, count, sum, last_updated_ms, consistency }`
- If the snapshot remains weak, document that exporters must tolerate bucket/count/sum skew.

## Hot-Path Allocation Review

The crate documents "zero-copy label dimensions" and "without heap allocation on the hot path." That is true only for cached metric handle updates.

Likely allocation-free:

- `Counter::inc()` and `Counter::inc_by()` after handle creation.
- `Gauge::inc()`, `Gauge::dec()`, and `Gauge::set()` after handle creation.
- `Histogram::observe()` after handle creation, aside from atomics and time call.

Allocation or expensive work occurs in:

- `LabelInterner::label_set()`, which creates a `Vec`, interns key/value strings, sorts, and deduplicates.
- Registry get-or-create, which builds/interns `MetricKey` and touches `DashMap`.
- `LabelSet::resolve()`, which creates a `Vec` of resolved labels.
- `Histogram::buckets()`, which creates a `Vec`.
- Snapshot APIs, which allocate vectors and clone keys/handles.
- `last_updated_ms` updates, which call wall-clock time on every metric update.

Production risk:

If developers interpret "hot path" as "registry lookup plus label construction per observation," they may put dynamic labels in high-frequency action paths and pay allocation, interning, sorting, and map lookup costs.

Recommended documentation:

Define two paths:

- Cached handle hot path: intended for high-frequency recording and should be allocation-free.
- Dynamic registration path: may allocate, intern, sort, and lock; not for per-event dynamic labels.

## Error Model Review

Evidence:

- `TelemetryError` only has `Io`.
- `TelemetryResult<T>` exists but core metric construction and registry accessors generally do not return it.

Missing primitive errors:

- Invalid histogram bucket layout.
- Metric type conflict.
- Histogram bucket layout conflict.
- Foreign label set or foreign metric key.
- Interner resolution failure.
- Arithmetic overflow, if checked arithmetic is chosen.
- Invalid percentile argument, if percentile remains public.

Current behavior:

- Invalid buckets panic.
- Bucket layout conflicts warn and return the existing histogram.
- Non-finite observations are silently ignored.
- Cross-interner label resolution may panic.

Recommended correction:

Add primitive error variants and fallible APIs where invalid primitive states can occur. Keep naming, label allowlist, and exporter validity errors in `nebula-metrics`.

## Layering Boundary Review

The crate mostly keeps the right dependencies: no dependency on `nebula-metrics`, no metric catalog, no exporter module.

Layering concerns:

- `crates/telemetry/examples/basic_metrics.rs` teaches direct `MetricsRegistry` use with `nebula_*` names.
- `MetricsRegistry` docs mention Prometheus and OTLP.
- Histogram defaults are called Prometheus-style.
- `tracing::warn!` is used for bucket layout conflicts instead of returning a primitive error. This couples correctness feedback to logging visibility.
- Repository call sites in `engine`, `api`, and `resource` import `nebula_telemetry::metrics::MetricsRegistry` directly. Some direct primitive use is acceptable, but canonical Nebula metrics should generally enter through `nebula-metrics`.

What must remain out of `nebula-telemetry`:

- `nebula_*` naming.
- Metric catalog.
- HELP/TYPE metadata.
- Prometheus escaping and content type.
- Label allowlist and cardinality policy.
- OTLP export.
- HTTP endpoint behavior.
- Operator dashboard semantics.

What belongs here:

- Registry identity and type conflict prevention.
- Label interner ownership safety.
- Valid histogram bucket layout.
- Arithmetic behavior.
- Snapshot consistency contract.
- Concurrency safety.
- Cached-handle allocation contract.

## API Misuse Cases

| Misuse | How current API allows it | Production failure | Prevent in |
|--------|---------------------------|--------------------|------------|
| Use a `LabelSet` from registry A in registry B | Labeled registry methods accept any `&LabelSet` | Wrong labels, key collision, or panic on resolve | `nebula-telemetry` |
| Register same key as counter and gauge | Separate maps allow `reg.counter("x")` and `reg.gauge("x")` | Exporter emits same metric family with conflicting types | `nebula-telemetry` |
| Register same histogram key with different buckets | `histogram_with_buckets_labeled()` ignores new boundaries | Wrong latency distribution | `nebula-telemetry` |
| Assume snapshot freezes values | `snapshot_*` returns live handles | Scrape reads changing values and inconsistent histograms | `nebula-telemetry` contract |
| Cache handle after retention | Metric handles are independent `Arc`s | Updates disappear from registry after eviction | `nebula-telemetry` |
| Build labels per observation | `interner().label_set()` is public and easy | Allocations and map lookups on hot path | Docs in telemetry, enforcement above |
| Intern execution IDs | Public interner accepts arbitrary strings | Memory growth and high-cardinality series | `nebula-metrics` policy, telemetry docs |
| Treat `Spur` as globally stable | `MetricKey` exposes raw `Spur` fields | Cross-registry corruption | `nebula-telemetry` |
| Use gauge as event counter | Registry does not encode descriptors | Non-monotonic event metric | `nebula-metrics` catalog |
| Use counter for current state | Type choice is caller-controlled | Current state cannot decrease | `nebula-metrics` catalog |
| Rely on counter never resetting | `fetch_add` wraps on overflow | False reset or alert noise | `nebula-telemetry` |
| Store invalid histogram buckets from config | `with_buckets()` panics | Library panic on startup or dynamic config | `nebula-telemetry` |
| Observe NaN/Inf and expect visibility | `observe()` returns silently | Bad caller behavior hidden | `nebula-telemetry` contract |
| Pass duplicate label keys | `LabelSet` last-wins dedupe | Dimension overwritten silently | Docs/tests in telemetry; schemas above |
| Depend on snapshot ordering | `DashMap` iteration is nondeterministic | Flaky tests and diffs | `nebula-telemetry` or exporter sorting |
| Use raw `nebula_*` names directly | Examples and public registry make it easy | Bypasses `nebula-metrics` naming and label safety | `nebula-metrics` API/docs |

## Missing Invariants

| Invariant | Currently encoded in types? | Currently tested? | Risk |
|-----------|-----------------------------|-------------------|------|
| Counter increments must be monotonic and never silently wrap without an explicit contract. | No | No overflow test | Counter can reset to 0 in long-running process. |
| Gauge arithmetic must define overflow, underflow, negative, NaN, and Infinity behavior. | Partially: integer gauge excludes NaN/Inf | Basic inc/dec only | Gauge can wrap and `set` races arithmetic with undefined semantics. |
| Histogram bucket boundaries must be sorted, finite, non-NaN, and non-duplicated. | Runtime `assert!` | Empty and unsorted only | Invalid config panics instead of returning primitive error; duplicate/nonfinite gaps in tests. |
| Histogram observations must map to exactly one bucket or a clearly defined overflow bucket. | Mostly | Basic bucket tests | Negative and non-finite behavior is not fully contractual. |
| Histogram count, sum, and buckets must have documented snapshot consistency semantics. | No | No | Exporters may assume impossible consistency. |
| Same `MetricKey` must not be registered as multiple metric types. | No | No | Invalid metric families and type confusion. |
| Same histogram `MetricKey` must not be registered with incompatible bucket layouts. | No | Test currently asserts first layout wins | Wrong distribution contract. |
| `LabelSet` identity must be independent of input label order. | Yes within one interner | Yes | Cross-interner identity remains unsafe. |
| Duplicate label keys must be rejected or canonicalized deterministically. | Canonicalized last wins | Yes | Silent overwrite can surprise callers. |
| Interned label symbols must not be treated as globally stable across registries unless guaranteed. | No | No | Foreign label sets can collide or panic. |
| Primitive hot-path update operations must not allocate if that is part of the crate contract. | No formal contract | No allocation tests | Users may allocate on every observation. |
| Snapshot consistency must be documented and sufficient for exporters. | No | No | Prometheus exporter may emit inconsistent histograms. |
| This crate must not contain naming, Prometheus, OTLP, or operator catalog policy. | Partially | No layering tests | Examples/docs teach bypass of `nebula-metrics`. |
| Cached metric handles must remain visible to registry snapshots or become explicitly invalid after retention. | No | No | Retention can orphan handles. |
| Registry-local label sets must be rejected or re-interned when used with another registry. | No | No | Wrong metric identity. |

## Real Nebula Scenarios

| # | Scenario | Current design behavior | Expected caller assumption | What could go wrong | Invariant or test |
|---|----------|-------------------------|----------------------------|---------------------|-------------------|
| 1 | 100 workers increment `executions_started` concurrently. | Atomic counter increments are not lost. | Count eventually equals total starts. | Overflow contract absent; last_updated may lag. | Concurrent counter test plus overflow test. |
| 2 | 100 workers observe action duration histograms concurrently. | Bucket, count, and sum atomics update independently. | Count, sum, and buckets form one coherent distribution. | Scrape can observe count without bucket or sum. | Concurrent histogram snapshot consistency test. |
| 3 | Scheduler updates running execution gauge while workers complete. | `set`, `inc`, and `dec` race with relaxed atomics. | Gauge reflects current running count. | `set` can erase concurrent increments/decrements. | Document set-vs-delta semantics; race test. |
| 4 | Registry snapshot happens during heavy updates. | Snapshot returns live handles; values keep changing. | Snapshot is a stable export view. | Exporter emits internally inconsistent values. | Immutable snapshot API or weak snapshot tests. |
| 5 | Two crates request same counter with same name and labels. | Same map entry returns shared handle. | Correctly shared counter. | Works if labels come from same interner. | Concurrent get-or-create same key test. |
| 6 | Two crates request same metric key with different metric types. | Separate maps create both. | Registry rejects conflict. | Exporter sees same name/labels as counter and gauge. | Type-conflict rejection test. |
| 7 | Same histogram key requested with different bucket layouts. | First layout wins; warning only. | Registry rejects conflict or includes layout in descriptor. | Caller records into wrong buckets. | Bucket layout conflict error test. |
| 8 | Labels are passed in different order by two call sites. | Canonical within same interner. | Same series. | Works only if same interner. | Existing label order test plus cross-interner test. |
| 9 | Duplicate label key is passed by mistake. | Last value wins. | Either explicit rejection or documented canonicalization. | A dimension is overwritten silently. | Duplicate-key behavior docs and test. |
| 10 | Plugin records execution ID as a label. | Interner and registry accept it. | Higher layer blocks it. | Series and interner cardinality explode. | `nebula-metrics` allowlist test; telemetry high-cardinality warning. |
| 11 | Plugin creates 100k unique label values. | Interner grows. | Primitive remains memory-bounded or higher layer blocks. | Memory DoS if bypassing metrics layer. | High-cardinality misuse benchmark/test. |
| 12 | `nebula-metrics` exporter resolves labels during scrape. | Uses registry interner. | Label symbols are valid for that interner. | Foreign `LabelSet` can panic or resolve wrong. | Foreign LabelSet rejection test. |
| 13 | Counter reaches maximum value in a long-running process. | `fetch_add` wraps. | Counter never decreases unless process restarts. | Alerts see fake reset. | Overflow behavior test. |
| 14 | Gauge `set` races with increment. | Atomic operations interleave. | Current state remains exact. | Increment can be lost relative to set semantics. | Concurrency contract test. |
| 15 | Histogram observes NaN or Infinity. | Observation is silently ignored. | Caller can detect invalid observation. | Bad upstream duration math hidden. | Non-finite observation behavior test. |
| 16 | Histogram observes value exactly on bucket boundary. | Goes into matching boundary bucket. | `<= le` semantics. | Currently likely correct. | Boundary inclusion property test. |
| 17 | `MetricsRegistry` is cloned into many components. | Clones share maps and interner. | Shared registry. | Retention from an owned mutable registry can still orphan older handles. | Clone plus retention handle test. |
| 18 | `LabelSet` created in one registry is used with another. | Accepted. | Rejected or re-interned. | Collision, wrong labels, or panic. | Cross-interner identity test. |
| 19 | Snapshot is assumed exact by Prometheus exporter. | Not exact; live handles and relaxed reads. | Valid metric family. | Inconsistent histogram and flaky tests. | Exporter-facing snapshot contract test. |
| 20 | High-frequency action metrics run in a hot path. | Cached handle updates are cheap; registry lookup and label construction are not. | No heap allocation. | Repeated dynamic registration causes allocation and contention. | Bench cached vs lookup paths. |

## Recommended Test Plan

### P0: must add before relying on this crate as stable

- Type conflict rejection: same registry, same name/labels, counter then gauge/histogram must fail.
- Histogram bucket conflict rejection: same key, different buckets must return a primitive error.
- Foreign `LabelSet`/`MetricKey` test: label sets from registry A must be rejected or safely re-interned by registry B.
- Snapshot contract test: prove immutable snapshots, or prove/document weak histogram skew under concurrent observation.
- Retention stale handle test: update a cached handle after `retain_recent()` and assert the chosen contract.
- Counter overflow behavior test near `u64::MAX`.
- Gauge overflow/underflow behavior tests near `i64::MAX` and `i64::MIN`.
- Invalid histogram bucket tests for duplicate, zero, negative, NaN, and infinity boundaries.
- Non-finite histogram observation behavior test.
- Concurrent get-or-create same key and same type.
- Concurrent get-or-create same key and different types.

### P1: should add soon

- Concurrent gauge `set` plus `inc/dec` test documenting expected behavior.
- Histogram observation property tests for below first bucket, boundary equality, between buckets, above last bucket, and negative values.
- Percentile argument tests for NaN, negative, zero, one, and greater than one.
- Label interner concurrent same-string and many-string tests.
- LabelSet duplicate-key docs test and cross-interner non-portability test.
- Snapshot during registration test.
- Deterministic snapshot ordering test if telemetry owns ordering, or exporter sorting test if not.
- High-cardinality interner growth simulation with explicit documentation that this is not a guard.

### P2: nice to have

- Loom tests for registry get-or-create and histogram snapshot if the implementation is refactored to stronger consistency.
- Property tests for label canonicalization within one interner.
- Property tests for histogram bucket mapping.
- Docs examples compile and avoid canonical Nebula naming policy.
- Allocation tests for cached counter/gauge/histogram updates.

## Recommended Benchmark Plan

### Hot-path counter update

- Cached `Counter::inc()` single-thread throughput.
- Cached `Counter::inc()` with 64 threads contending on one counter.
- `registry.counter(name).inc()` repeated lookup throughput.

### Hot-path gauge update

- Cached `Gauge::inc/dec` throughput.
- `Gauge::set` throughput.
- Mixed `set` and `inc/dec` contention.

### Hot-path histogram observe

- Cached `Histogram::observe()` with default buckets.
- Cached `Histogram::observe()` with many buckets.
- Contended observe with 64 threads.
- Sum update contention under high observation rate.

### Registry lookup

- Get-or-create existing metric.
- Register new metric.
- Concurrent registration of the same key.
- Concurrent high-cardinality registration.

### Label interning

- Repeated same label pairs.
- One million unique values.
- Concurrent same-string interning.
- Concurrent unique-string interning.

### Snapshot generation

- Snapshot 1k, 10k, and 100k counters.
- Snapshot 1k, 10k, and 100k histograms.
- Snapshot while updating.
- Snapshot while registering.

### High-cardinality misuse simulation

- Dynamic `execution_id` labels with 10k, 100k, and 1M unique values.
- Measure registry memory, interner memory, snapshot time, and compaction behavior.

## Recommended Refactor Plan

### Phase 1: clarify primitive contracts and snapshot semantics

- Document cached-handle hot path vs registry lookup path.
- Document relaxed atomic consistency.
- Document counter/gauge overflow behavior, even before changing it.
- Document histogram non-finite and negative observation behavior.
- Rename or document `snapshot_*` as weak live-handle enumeration if it stays that way.

### Phase 2: enforce MetricKey, LabelSet, and histogram invariants

- Make `MetricKey` fields private.
- Bind `LabelSet` to a registry/interner identity, or accept raw label pairs only through registry methods.
- Add fallible histogram bucket validation.
- Define duplicate label key behavior as reject or explicit last-wins.

### Phase 3: harden registry type-conflict behavior

- Replace separate maps with a single descriptor map, or add a central type table.
- Reject same key with different metric kind.
- Reject same histogram key with incompatible bucket layout.
- Add `TelemetryError` variants for primitive conflicts.

### Phase 4: add concurrency and property tests

- Add P0 concurrency tests for registry, histogram, and snapshots.
- Add property tests for histogram bucket mapping and label canonicalization.
- Add stale-handle retention tests.

### Phase 5: verify hot-path allocation/performance claims

- Add benchmarks for cached handles, registry lookup, label construction, interner contention, and snapshot.
- Add optional allocation assertions for cached handle update paths.
- Decide whether `last_updated_ms` on every update is acceptable.

### Phase 6: document layering boundary with nebula-metrics

- Remove canonical `nebula_*` examples from telemetry docs/examples.
- Keep naming and exporter guidance in `nebula-metrics`.
- Add a clear warning that `LabelInterner` is not cardinality safety.
- Ensure future OTLP and Prometheus semantics remain above this crate.

## Proposed Canon Invariants

| Proposed Invariant | Why Nebula Needs It | How To Encode | How To Test |
|--------------------|---------------------|---------------|-------------|
| A `MetricKey` is registry-local identity and cannot be constructed from foreign interner symbols. | Prevents label collisions, wrong resolution, and scrape panics. | Private fields plus registry-bound label set token or interner generation. | Cross-registry label set misuse test. |
| Label order must not affect metric identity within a registry. | Two call sites must share the same series. | Canonicalize labels in registry/interner. | Property test permutations of label pairs. |
| Duplicate label keys must have explicit behavior. | Prevents silently ambiguous identity. | Reject duplicates or document last-wins in API. | Duplicate key tests and docs tests. |
| A registry identity maps to exactly one metric kind. | Exporters require one HELP/TYPE contract per metric family. | Single map with `MetricKind` descriptor or conflict table. | Counter then gauge same key must error. |
| A histogram identity maps to exactly one immutable bucket layout. | Bucket layout defines distribution meaning. | Store bucket layout in descriptor and compare on lookup. | Same key with different buckets must error. |
| Histogram bucket boundaries are finite, non-NaN, positive if required, strictly increasing, and non-empty. | Prevents invalid bucket placement and export confusion. | Fallible bucket constructor. | Invalid boundary table tests. |
| Counter overflow behavior is explicit and tested. | Operators must not see silent false resets. | Checked/saturating increment or documented error contract. | Near-maximum counter test. |
| Gauge overflow, underflow, and set-vs-delta races are explicit and tested. | Current-state metrics must not become impossible silently. | Checked/saturating arithmetic or documented wrapping rejection. | Boundary and concurrency tests. |
| Histogram count, sum, and bucket snapshot consistency is explicit. | Exporters must know whether a family is exact or best-effort. | Immutable snapshot struct or documented weak consistency. | Snapshot-under-update test. |
| Cached handle updates remain visible to the registry, or handles become explicitly invalid after retention. | Avoids invisible metric updates. | Remove retention, use generational handles, or reinsert on update. | Retain then update cached handle test. |
| Label interning is not cardinality control. | Prevents memory DoS assumptions. | README warning and API docs. | High-cardinality misuse test/benchmark. |
| Cached handle update path does not allocate if documented as hot path. | Keeps action/execution telemetry cheap. | Benchmarks and allocation tests. | Allocation assertion around cached updates. |
| This crate contains no metric naming, Prometheus, OTLP, HTTP, or operator catalog policy. | Preserves layering with `nebula-metrics`. | Docs lint or review checklist. | Search test for `nebula_*`, Prometheus exporter concepts, and content type in telemetry. |

## GitHub Issues

### Issue T-001: Bind `LabelSet` and `MetricKey` identity to the registry/interner

Severity: Critical

Classification: API correction

Problem:
`LabelSet` stores raw `Spur` symbols and `MetricKey` exposes public `name` and `labels` fields. Registry labeled APIs accept any `&LabelSet`. A label set created by one `LabelInterner` can be passed to another registry.

Evidence:
`crates/telemetry/src/labels.rs` defines `LabelKey = Spur`, `LabelValue = Spur`, `LabelSet { labels: Vec<(LabelKey, LabelValue)> }`, and public `MetricKey` fields. `crates/telemetry/src/metrics.rs` labeled methods clone the provided `LabelSet` without verifying origin.

Failure scenario:
A plugin or subsystem builds labels with registry A and records into registry B. The same symbol IDs may mean different strings or be unresolved in B, causing wrong metric identity or panic during label resolution.

Recommended fix:
Make metric identity registry-bound. Options: private `MetricKey` fields, registry-owned `LabelSet` with an interner generation ID, or registry APIs that accept raw label pairs and re-intern internally. Use `try_resolve` or validation for unsafe cases.

Acceptance criteria:

- Foreign label sets are rejected or safely re-interned.
- Public API no longer lets callers construct arbitrary `MetricKey` from raw symbols.
- Tests cover same labels across two registries and different labels with colliding symbol IDs.

### Issue T-002: Prevent registering one metric identity as multiple metric types

Severity: Critical

Classification: Architecture correction

Problem:
`MetricsRegistry` uses separate maps for counters, gauges, and histograms. The same `MetricKey` can exist in more than one map.

Evidence:
`crates/telemetry/src/metrics.rs` defines `counters`, `gauges`, and `histograms` as separate `DashMap<MetricKey, _>` fields. `counter`, `gauge`, and `histogram` create entries independently.

Failure scenario:
One component records `requests` as a counter and another records `requests` as a gauge with the same labels. The exporter sees conflicting metric types for the same identity.

Recommended fix:
Introduce a single descriptor table keyed by `MetricKey` that stores `MetricKind`, or a unified enum entry map. Return a `TelemetryError` on type conflict.

Acceptance criteria:

- Same key same type returns the same handle.
- Same key different type returns a deterministic error.
- Tests cover concurrent same-key different-type registration.

### Issue T-003: Define and enforce histogram snapshot consistency

Severity: Critical

Classification: Refactor/API correction

Problem:
Histogram count, sum, buckets, and timestamp are independent relaxed atomics. Registry snapshot returns live `Histogram` handles, not immutable values.

Evidence:
`Histogram::observe()` updates bucket count, total count, sum bits, and last update in separate operations. `snapshot_histograms()` returns `Vec<(MetricKey, Histogram)>`.

Failure scenario:
Prometheus scrape reads `total_count` after an observation and a bucket count or sum before that observation. The exported histogram family is internally inconsistent.

Recommended fix:
Define snapshot semantics. Prefer immutable histogram snapshot structs for exporters. If exact consistency is too expensive, document best-effort semantics and ensure exporters/tests do not assert stronger invariants.

Acceptance criteria:

- Snapshot docs state exact consistency level.
- Tests cover snapshot during concurrent observation.
- Exporter code has a documented assumption matching the primitive contract.

### Issue T-004: Reject incompatible histogram bucket layouts for the same key

Severity: High

Classification: API correction

Problem:
`histogram_with_buckets_labeled()` ignores new boundaries when a histogram already exists for the key and logs a warning.

Evidence:
`crates/telemetry/src/metrics.rs` compares existing boundaries to requested boundaries and warns that it is ignoring new boundaries.

Failure scenario:
Two subsystems believe they are recording the same metric with different bucket layouts. One silently records under the other's layout.

Recommended fix:
Make bucket layout part of the histogram descriptor. Return a primitive conflict error when an existing key has a different layout.

Acceptance criteria:

- Same key same layout succeeds.
- Same key different layout errors.
- Existing test that asserts first layout wins is replaced with conflict behavior.

### Issue T-005: Fix or remove retention behavior that orphans cached metric handles

Severity: High

Classification: Architecture correction

Problem:
`retain_recent()` removes entries from registry maps and compacts the interner while cloned metric handles remain live. Updates to removed handles are no longer visible in snapshots.

Evidence:
Metric types are cloneable `Arc` handles. `retain_recent()` removes entries by `last_updated_ms()` and rebuilds map keys in `compact_interner()`.

Failure scenario:
A component caches a counter for performance. The registry evicts it as stale. The component later increments the cached handle, but the registry has no key pointing to it.

Recommended fix:
Choose a clear contract: remove retention from primitive layer, make handles generational and invalid after eviction, or make updates reattach/reinsert. Document the chosen behavior.

Acceptance criteria:

- Test demonstrates the contract for cached handle updates after retention.
- Docs warn if retention requires proving no stale handles exist.
- No silent invisible updates.

### Issue T-006: Define counter and gauge overflow/underflow behavior

Severity: High

Classification: API correction

Problem:
Counter and gauge arithmetic uses atomic wrapping operations without an explicit contract.

Evidence:
`Counter::inc_by()` uses `AtomicU64::fetch_add`. `Gauge::inc()` and `Gauge::dec()` use `AtomicI64::fetch_add` and `fetch_sub`.

Failure scenario:
A long-running counter wraps to 0. A gauge wraps from max positive to negative. Operators see false resets or impossible state.

Recommended fix:
Use checked or saturating atomic update loops, return primitive errors, or document an explicit overflow policy and test it. Silent wrapping should not be the default stable contract.

Acceptance criteria:

- Counter near `u64::MAX` has tested behavior.
- Gauge near `i64::MAX` and `i64::MIN` has tested behavior.
- Public docs describe the behavior.

### Issue T-007: Document and test that `LabelInterner` is not cardinality safety

Severity: High

Classification: Documentation/test only plus API correction

Problem:
The public interner deduplicates repeated strings but remains append-only until compaction. It does not prevent high-cardinality labels.

Evidence:
`LabelInterner` docs state append-only behavior. `LabelInterner::label_set()` accepts arbitrary label strings.

Failure scenario:
Plugin code interns unique execution IDs or request IDs. Memory grows with every unique label value, and the registry creates unbounded series.

Recommended fix:
Document explicitly that cardinality policy lives in `nebula-metrics`. Consider reducing direct `LabelInterner` exposure by routing label creation through the registry. Add high-cardinality misuse tests/benchmarks.

Acceptance criteria:

- README and API docs warn that interning is not a guard.
- Benchmark/test shows behavior under many unique labels.
- `nebula-metrics` remains responsible for allowlist policy.

### Issue T-008: Clarify and verify the hot-path allocation contract

Severity: High

Classification: Documentation/test only

Problem:
Docs imply label dimensions are zero-copy and hot-path safe, but label construction and registry lookup allocate and perform map/interner work. Only cached handle updates are plausibly allocation-free.

Evidence:
`LabelInterner::label_set()` allocates a `Vec`, interns, sorts, and deduplicates. Snapshot and resolution APIs allocate vectors.

Failure scenario:
Action execution code builds labels and performs registry lookup on every attempt, assuming it is allocation-free.

Recommended fix:
Document the distinction between cached handle update path and dynamic registration path. Add benchmarks and allocation tests.

Acceptance criteria:

- Docs define "hot path."
- Cached handle update benchmarks exist.
- Registry lookup plus label construction benchmarks exist.

### Issue T-009: Expand `TelemetryError` for primitive invalid states

Severity: High

Classification: API correction

Problem:
`TelemetryError` only has `Io`, while primitive operations can encounter invalid buckets, metric conflicts, foreign label sets, and invalid observations.

Evidence:
`crates/telemetry/src/error.rs` has only `TelemetryError::Io`. Invalid bucket layouts panic; bucket conflicts warn; non-finite observations are silently ignored.

Failure scenario:
Higher layers cannot assert that an invalid primitive state was rejected. Bugs become panics, warnings, or invisible data loss.

Recommended fix:
Add primitive-layer error variants and fallible APIs where invalid primitive states occur. Do not add naming/export/allowlist errors to this crate.

Acceptance criteria:

- Invalid bucket layout can be asserted as an error.
- Type conflict and bucket conflict can be asserted as errors.
- Foreign label set behavior is erroring or impossible.

### Issue T-010: Remove naming/export policy leakage from telemetry docs and examples

Severity: High

Classification: Documentation/test only

Problem:
Telemetry docs and examples mention `nebula_*`, Prometheus, OTLP, and Prometheus-style buckets. This teaches direct use of the primitive layer for policy-bearing Nebula metrics.

Evidence:
`crates/telemetry/README.md` discusses naming conventions and Prometheus/OTLP boundaries. `crates/telemetry/examples/basic_metrics.rs` records canonical-looking `nebula_*` names. `metrics.rs` docs refer to Prometheus-style buckets.

Failure scenario:
Developers bypass `nebula-metrics`, losing naming and label safety while still exporting plausible metrics.

Recommended fix:
Keep telemetry examples policy-neutral. Move canonical naming examples to `nebula-metrics`. Describe default buckets in primitive terms, not exporter terms.

Acceptance criteria:

- Telemetry examples do not use canonical `nebula_*` metric names.
- Telemetry docs clearly say primitive only and point to `nebula-metrics` for naming/export policy.
- A search-based docs test or review checklist prevents reintroducing exporter policy into this crate.

---

# Independent Re-pass (2026-05-05)

This section is a fresh audit pass over `crates/telemetry` against current code. It (1) verifies whether the original T-001..T-010 findings still hold, (2) records anything that was partially addressed, (3) adds new findings the first pass missed, and (4) applies the layering rule so refactor work cannot smuggle policy from `nebula-metrics` downward.

## Status of Original Findings

| ID | Status | Evidence (current code) |
|----|--------|--------------------------|
| T-001 | **CONFIRMED** | `crates/telemetry/src/labels.rs:35-38` — `LabelKey`/`LabelValue` are `Spur` aliases. `crates/telemetry/src/labels.rs:271-294` — `MetricKey { pub name: Spur, pub labels: LabelSet }`. `crates/telemetry/src/labels.rs:159-161` — `LabelInterner::resolve` panics for foreign Spurs. Registry methods (`crates/telemetry/src/metrics.rs:463-478`) accept any `&LabelSet` without origin verification. |
| T-002 | **CONFIRMED** | `crates/telemetry/src/metrics.rs:402-405` — three independent `DashMap<MetricKey, _>` fields. `crates/telemetry/src/metrics.rs:431-446` (unlabeled accessors) and `:463-478` (labeled accessors) each call `entry().or_default()` on their own map without checking the others. |
| T-003 | **CONFIRMED AND DEEPENED** | `crates/telemetry/src/metrics.rs:220-246` — `observe` performs three independent Relaxed atomics (`counts[idx].fetch_add`, `total_count.fetch_add`, `sum_bits.update`). `crates/telemetry/src/metrics.rs:534-539` — `snapshot_histograms` returns `Vec<(MetricKey, Histogram)>` (live handles), not frozen snapshots. The existing concurrency test `histogram_concurrent_observe` (`crates/telemetry/src/metrics.rs:783-803`) only asserts the join-final `count == 100_000`; **there is no test that asserts `+Inf == _count == sum_of_finite_buckets` while observe and snapshot run concurrently.** The invariant the exporter depends on is untested. |
| T-004 | **CONFIRMED** | `crates/telemetry/src/metrics.rs:489-511` — `histogram_with_buckets_labeled` returns the existing histogram unchanged on layout mismatch and emits `tracing::warn!` only. The test `histogram_with_buckets_labeled_returns_first_layout_on_conflict` (`crates/telemetry/src/metrics.rs:909-925`) asserts current (broken) behavior, locking it in until the contract changes. |
| T-005 | **CONFIRMED AND DEEPENED** | `crates/telemetry/src/metrics.rs:575-583` — `retain_recent` is `&mut self`. `crates/api/src/state.rs:91` holds `Option<Arc<MetricsRegistry>>` in production. `Arc::get_mut` requires zero other clones; engine/api/resource clone the registry into multiple components. **The retention feature is unreachable in production composition** — see T-020 below for the layering consequence. |
| T-006 | **CONFIRMED** | `crates/telemetry/src/metrics.rs:48-57` (`fetch_add` Relaxed), `:96-110` (`fetch_add`/`fetch_sub`/`store` Relaxed). No saturation, no checked variants, no documentation of overflow policy. |
| T-007 | **CONFIRMED** | `crates/telemetry/src/labels.rs:108-114` documents append-only behavior. `crates/telemetry/src/labels.rs:149-151` accepts arbitrary `&str`. No high-cardinality misuse test exists. |
| T-008 | **CONFIRMED** | `crates/telemetry/src/labels.rs:196-218` (`label_set` allocates `Vec`, interns, sorts, deduplicates). `crates/telemetry/src/metrics.rs:431-446` / `:463-478` (`entry()` on `DashMap` plus key clone). No allocation benchmark exists. |
| T-009 | **CONFIRMED** | `crates/telemetry/src/error.rs:6-11` — `TelemetryError` has only `Io`. Invalid bucket layouts still panic (`crates/telemetry/src/metrics.rs:188-199`). Bucket conflicts only `tracing::warn!` (`crates/telemetry/src/metrics.rs:502-509`). Non-finite observations silently return (`crates/telemetry/src/metrics.rs:220-223`). |
| T-010 | **CONFIRMED** | `crates/telemetry/examples/basic_metrics.rs:23-44, 53-71` — example uses canonical `nebula_executions_total` / `nebula_action_duration_seconds` / `nebula_action_executions_total` names. `crates/telemetry/src/metrics.rs:132-135` describes defaults as "Default Prometheus histogram bucket boundaries". `crates/telemetry/src/metrics.rs:139-141` says "Prometheus-style bucket boundaries". Boundary leakage intact. |

## New Findings

### T-011: High — `Histogram::with_buckets` is `pub` and constructs metrics outside any registry

`crates/telemetry/src/metrics.rs:186-213` exposes `Histogram::with_buckets(Vec<f64>)` as a public constructor. Same for `Histogram::new()`, `Counter::new()`, `Gauge::new()`. A caller can:

```rust
let h = Histogram::with_buckets(vec![0.1, 1.0]);
h.observe(0.5);
// observation is real, atomics work — but no registry sees this metric.
```

The orphan handle is `Clone`-shared via `Arc` and can be passed around as if it were a registry-backed metric. Snapshots never include it. There is no way for `nebula-metrics` (or any other consumer) to enforce "all metrics must enter through the registry" because the primitive types are publicly constructible.

This is the layering image of T-002: T-002 is "the registry holds inconsistent type bindings"; T-011 is "the metric exists outside any binding at all". Both come from the same root: metric handles are not registry-bound.

Failure scenario: a subsystem caches a `Histogram::new()` returned from a helper that was meant to use the shared registry but a refactor accidentally created a new one. Recordings happen, dashboards remain blank, and the bug is invisible until someone scrapes.

Classification: Architecture correction. Make `Counter::new` / `Gauge::new` / `Histogram::with_buckets` `pub(crate)` and require all handle creation to go through `MetricsRegistry`.

### T-012: Critical — `compact_interner` desyncs cached `MetricKey`s

`crates/telemetry/src/metrics.rs:603-642` rebuilds the interner and remaps `MetricKey`s in the registry maps. But:

- Cached metric handles (`Counter` / `Gauge` / `Histogram`) share atomics by `Arc`, not the registry's stored `MetricKey`. The handle continues to work (T-005).
- However, any caller that holds `LabelSet` or `MetricKey` values directly — e.g., the metrics exporter, which iterates `snapshot_*` and resolves labels (`crates/metrics/src/export/prometheus.rs:253-264`) — is fine **only if** it only ever uses the Spurs from the *current* registry interner.
- After `compact_interner`, **the new interner has different Spur IDs for the same strings**. If any `LabelSet` cached anywhere outside the registry maps still holds old Spurs, calling `interner.resolve(old_spur)` panics (`crates/telemetry/src/labels.rs:155-161`).

Sharper than T-005: T-005 said "cached handles can update unseen". T-012 says "if anyone caches a LabelSet across compaction, the next snapshot panics". The downstream exporter currently builds LabelSets per scrape (no caching), so today this is latent — but the API does not warn, and a future caller caching a LabelSet for performance triggers a hard panic in `resolve`.

Classification: Architecture correction. Either drop the rebuild-style compaction, or expose interner generation IDs so consumers can detect staleness.

### T-013: Medium — `Histogram::buckets()` is O(n) per call and called per-scrape

`crates/telemetry/src/metrics.rs:264-277` recomputes the cumulative array on every invocation (load each atomic, accumulate, allocate `Vec`). Exporters call this on every scrape for every histogram (`crates/metrics/src/export/prometheus.rs:317`).

For a registry with 1000 histograms × 12 buckets = 12000 atomic loads + 1000 `Vec` allocations per scrape. Acceptable for small catalogs; under cardinality misuse (T-007), this becomes a measurable scrape latency contributor.

Classification: Refactor — provide a snapshot-style accessor that loads once and returns an immutable `HistogramSnapshot` (paired with the T-003 fix).

### T-014: Medium — `Histogram::sum_bits` update is a CAS loop, not wait-free

`crates/telemetry/src/metrics.rs:240-244` uses `AtomicU64::update` (Rust 1.95) which is internally a `compare_exchange_weak` loop. Under high contention (many threads observing the same histogram), each observation retries until the CAS succeeds. The histogram type's docstring (`crates/telemetry/src/metrics.rs:140-141`) claims **"lock-free"**, which is technically correct but misleading: bucket and count updates are wait-free, sum updates are lock-free with retry under contention.

Failure scenario: 64-thread benchmark observing one histogram pegs CAS retries; per-op latency tail explodes despite the "lock-free" docstring. No benchmark exists to characterize it.

Classification: Documentation/test only — clarify the wait-free vs lock-free distinction; add a contention benchmark.

### T-015: Medium — `LabelSet::resolve()` allocates a `Vec<(&str, &str)>` per call

`crates/telemetry/src/labels.rs:90-95` returns `Vec<(&'a str, &'a str)>`. Every exporter scrape allocates one of these per series (`crates/metrics/src/export/prometheus.rs:253-264, 276-287, 299-310` build label_str via `render_labels`, which iterates `LabelSet::iter()` directly — but other call sites such as `compact_interner` at `crates/telemetry/src/metrics.rs:610-614` use `resolve()` to rebuild label refs, allocating per series across the whole registry).

Combined with T-013, scrape allocation grows linearly with `(series × labels)`. For a 10k-series registry under regular scrape, this can dominate scrape time over the actual atomic loads.

Classification: Refactor — provide a `for_each<F: FnMut(&str, &str)>(&self, interner)` non-allocating accessor; reserve `resolve()` for one-shot debugging.

### T-016: Medium — `LabelInterner::filter_label_set` re-interns `allowed_keys` on every call

`crates/telemetry/src/labels.rs:248-256`:

```rust
pub fn filter_label_set(&self, labels: &LabelSet, allowed_keys: &[&str]) -> LabelSet {
    let allowed: Vec<Spur> = allowed_keys.iter().map(|k| self.intern(k)).collect();
    ...
}
```

Every call interns the allow list. `ThreadedRodeo::get_or_intern` is fast for already-interned strings (read-side lock-free), but it still touches the rodeo hashmap and allocates the `Vec<Spur>`. The downstream `LabelAllowlist::apply` (`crates/metrics/src/filter.rs:95-101`) calls this on every metric record path with the allowlist's keys.

For a 5-key allowlist applied at 100k records/sec, that's 500k unnecessary `get_or_intern` calls per second. The proper API is to intern the allowlist once at construction and pass `&[Spur]`.

Classification: Boundary contract issue — telemetry should provide a `filter_label_set_by_spur(&self, &LabelSet, &[Spur]) -> LabelSet` so the downstream allowlist can intern its keys exactly once at build time. Today's `&[&str]` shape forces re-interning.

### T-017: Patch — `now_ms()` uses lossy `as u64` truncation

`crates/telemetry/src/metrics.rs:21-28`:

```rust
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
```

`as_millis()` returns `u128`; the `as u64` cast silently truncates. Will not overflow in practice (u64 ms = 584 million years), but `clippy::cast_possible_truncation` would flag it. The deeper issue is that `unwrap_or_default()` on a clock-step-backwards condition collapses to `Duration::ZERO`, so `now_ms()` can return `0` after an NTP step backward — and `last_updated_ms` then *decreases*, breaking T-005's already-fragile retention semantics.

Classification: Patch — use `try_into().unwrap_or(u64::MAX)` and document the clock-monotonicity assumption.

### T-018: Medium — `Histogram::percentile(p)` does not validate `p`

`crates/telemetry/src/metrics.rs:308-348` documents `p: 0.0–1.0` but does not validate. Behavior:
- `p = NaN` → `target = NaN`; `cumulative as f64 >= NaN` is always false → returns the fallback `last_boundary`, which is misleading-but-finite.
- `p = -0.5` → `target` negative; first iteration's `cumulative == 0 >= negative` is true → returns `0.0`.
- `p = 2.0` → `target > total`; loop exits without match → returns `last_boundary`.

For an external caller (e.g., a debugging dashboard), invalid `p` returns a plausible-looking number rather than an error. The function is `pub` but not part of the export path; downstream consumers calling it need to validate `p` themselves.

Classification: API correction — return `Option<f64>` or take `Percentile` newtype.

### T-019: High — `inc_by(0)` and `Gauge::set(same_value)` semantics are undefined

`Counter::inc_by(0)` (`crates/telemetry/src/metrics.rs:54-57`) always stores `now_ms()` into `last_updated_ms` even when no value changed. `Gauge::set(v)` (`crates/telemetry/src/metrics.rs:108-110`) always stores the timestamp regardless of whether `v` equals the current value.

This creates ambiguity in the `last_updated_ms` contract:
- Is it "last time the value changed"? No — `inc_by(0)` and `set(unchanged)` bump it.
- Is it "last time the metric was touched"? Yes today, but the API does not say so.

`retain_recent` (`crates/telemetry/src/metrics.rs:575-583`) uses `last_updated_ms() >= cutoff` to decide eviction. Under "last touched" semantics, a per-loop liveness ping (`counter.inc_by(0)` once per second) keeps a series alive forever, defeating cardinality compaction even when the value is genuinely zero. Under "last value-change" semantics, a steadily-incrementing counter would also be retained — but the semantics are not stated, so callers cannot reason about which usage pattern they should adopt.

Classification: API correction — define the semantics in docs, then enforce by either (a) skipping the timestamp store on no-op writes or (b) keeping current behavior but documenting it as "metric was touched".

### T-020: High — `retain_recent` `&mut self` requirement is incompatible with `Arc<MetricsRegistry>` clone pattern

`crates/telemetry/src/metrics.rs:575-583` requires `&mut self`. `crates/telemetry/src/metrics.rs:398-405` makes `MetricsRegistry: Clone` cheap (Arc-wrapped maps and interner). Production composition holds `Arc<MetricsRegistry>` (`crates/api/src/state.rs:91`, plus engine/api/resource clones).

`Arc::get_mut` requires the strong count to be 1. With multiple clones live, `&mut MetricsRegistry` cannot be obtained without dropping every other clone first — i.e., never in steady-state production.

Therefore: **the registry's only cardinality-management feature is unreachable in production**. Every metric write pays the `now_ms()` cost (T-017, T-019) for a feature no production path can call.

Classification: Architecture correction. Decide:
- (a) Remove `retain_recent` and the `last_updated_ms` infrastructure — the feature is dead weight.
- (b) Provide a compaction-safe registry (e.g., `Arc<RwLock<Inner>>`, or a pruning thread that takes ownership transiently via `Arc::try_unwrap` patterns) that production composition can actually invoke.
- (c) Keep current behavior but document explicitly that it is not callable from `Arc<MetricsRegistry>` and remove the docs implying production use.

### T-021: Medium — `DEFAULT_BUCKETS` is policy disguised as primitive default

`crates/telemetry/src/metrics.rs:132-135`:

```rust
/// Default Prometheus histogram bucket boundaries.
const DEFAULT_BUCKETS: &[f64] = &[0.005, 0.01, ..., 10.0];
```

The value choice is HTTP-latency-shaped, ranging up to 10 seconds. This is a **catalog-domain decision** disguised as a primitive default:

- Primitives should not have defaults at all (caller decides), OR
- The default should be explicitly named in primitive terms ("sub-10-second latency"), not branded "Prometheus".

Telemetry's role is "lock-free atomic histograms with caller-supplied bucket boundaries". Picking a bucket layout that suits HTTP latencies is a metric-shape policy that belongs to `nebula-metrics` (per the metrics audit M-016: per-metric bucket schemas in the catalog).

Classification: Boundary contract issue (subsumed by metrics M-016). Either remove `DEFAULT_BUCKETS` entirely (force callers to supply boundaries) or rename + de-couple from "Prometheus" branding. The catalog in `nebula-metrics` becomes the authoritative source of bucket layouts.

## Layering Responsibility Classification

Per the layering rule: cardinality, naming, label safety, and Prometheus correctness belong to `nebula-metrics`; primitive atomic correctness, registry identity, and bucket data structures belong here. HTTP serving belongs to `nebula-api`. Logs belong to `nebula-log`.

For each finding, the table below shows: who owns the fix, what `nebula-telemetry` must do, and what (if anything) the adjacent crate must do.

| Finding | Class | `nebula-telemetry` action | Other-crate action |
|---------|-------|----------------------------|--------------------|
| T-001 | Own | Make `MetricKey` fields private; bind `LabelSet` to interner identity (generation ID or registry handle); registry methods accept raw `(&str, &str)` pairs and intern internally, OR take a registry-bound `BoundLabelSet`. | none (fix lifts upward — `nebula-metrics::SafeLabels` design becomes possible only after this lands). |
| T-002 | Own | Single descriptor map keyed by `MetricKey` with `MetricKind` enum; reject cross-kind reuse with `TelemetryError::MetricKindConflict`. | `nebula-metrics`'s catalog enforces *naming* policy on top; the *type-per-name* invariant is a primitive identity contract owned here. |
| T-003 | Own | Define snapshot semantics. Provide `Histogram::snapshot() -> HistogramSnapshot { count, sum, cumulative_buckets }` using seqlock or equivalent for atomic read of all three. | `nebula-metrics`'s exporter consumes the new snapshot. |
| T-004 | Own | Make bucket layout part of histogram identity; `histogram_with_buckets_labeled` returns `TelemetryError::HistogramLayoutConflict` on mismatch. | none. |
| T-005 | Own | See T-020 — requires architectural decision on the entire retention feature, not a local patch. | none. |
| T-006 | Own | Define overflow policy: checked, saturating, or wrapping with explicit contract. | none — overflow is a primitive arithmetic concern. |
| T-007 | Boundary contract | Document explicitly that interning is not a cardinality guard; reduce direct interner exposure where feasible. | `nebula-metrics` enforces cardinality policy through its `LabelAllowlist` / `LabelSchema` (metrics audit M-002, M-009). |
| T-008 | Own | Document cached-handle vs registry-lookup paths; add allocation benchmarks. | none — the contract definition is owned here; downstream callers adopt the documented pattern. |
| T-009 | Own | Add `TelemetryError` variants for invalid primitive states. Keep naming/Prometheus errors out. | `nebula-metrics` builds higher-layer error variants (M-009 LabelAllowlist diagnostic counter). |
| T-010 | Own | Strip `nebula_*` names from examples; rename "Prometheus-style buckets" to a primitive-neutral phrase; remove Prometheus/OTLP from docstrings. | `nebula-metrics` becomes the single home for canonical naming examples (already partly there). |
| T-011 | Own | Make `Counter::new` / `Gauge::new` / `Histogram::with_buckets` `pub(crate)`; require registry-mediated handle creation. | none. |
| T-012 | Own | Either drop rebuild-style compaction or expose interner generation ID for downstream staleness detection (paired with T-005, T-020). | `nebula-metrics` exporter must check generation if the API exposes it. |
| T-013 | Own | Provide non-allocating snapshot/iterate APIs (paired with T-003). | none. |
| T-014 | Own | Document lock-free-with-retry semantics for `sum_bits`; add contention benchmark. | none. |
| T-015 | Own | Provide `LabelSet::for_each(&interner, F)` non-allocating accessor; reserve `resolve()` for debug. | none. |
| T-016 | Boundary contract | Provide `filter_label_set_by_spur(&self, &LabelSet, &[Spur]) -> LabelSet` so allowlist can intern keys once. | `nebula-metrics::LabelAllowlist::apply` switches to the spur-based primitive after building its key set once. |
| T-017 | Patch | Use `try_into().unwrap_or(u64::MAX)`; document clock-monotonicity assumption. | none. |
| T-018 | Own | `percentile(p)` returns `Option<f64>` or accepts a `Percentile` newtype. | none. |
| T-019 | Own | Define `last_updated_ms` semantics in docs; either skip timestamp on no-op writes or keep current behavior with explicit "metric was touched" semantics. | none. |
| T-020 | Own (architectural) | Decide retention's future: drop / refactor / document-as-not-prod. ADR-grade decision. | `nebula-metrics` audit M-023 raises the same point from the other side; both crates must align. |
| T-021 | Boundary contract | Remove or rename `DEFAULT_BUCKETS`; strip "Prometheus" branding. | `nebula-metrics`'s catalog (per metrics M-016) becomes the authoritative source of per-metric bucket layouts. |

### Layering Anti-patterns to Avoid in the Refactor

- **Do not** add naming/Prometheus/OTLP policy here under the guise of "primitive consistency". `MetricKind` is a primitive concept (counter vs gauge vs histogram); `_total` suffix conventions are `nebula-metrics` policy. Don't conflate them.
- **Do not** add a label allowlist / cardinality budget to `nebula-telemetry`. T-007's fix is *documentation* here; *enforcement* lives in `nebula-metrics`.
- **Do not** "fix" T-001 by adding a `pub fn validate_label_set(&self, &LabelSet) -> bool` that downstream callers must remember to invoke. Type-enforced identity (registry-bound `LabelSet`) is the only correct fix; relying on caller discipline reproduces the original bug under a different name.
- **Do not** "fix" T-005 / T-020 by silently making `retain_recent` a no-op when called via `Arc`. Either the feature exists and works, or it is removed. Half-existing features burn future maintainers.
- **Do not** "fix" T-014 by replacing `update` with a `Mutex<f64>`. The lock-free property is a primitive contract; the fix is documentation, not relocation of the synchronization primitive.
- **Do not** add HELP/TYPE descriptors to `MetricsRegistry`. Snapshots can carry a `MetricKind` (counter/gauge/histogram), but operator-facing HELP text and Prometheus type labels stay in `nebula-metrics`.
- **Do not** move `LabelInterner` into `nebula-metrics` to "hide" T-007. Interning is a primitive concern; the policy that decides which strings *should* be interned is the upper layer's. Keep interning here, push policy upward.

### Net Layering Picture

Of 21 telemetry findings:

- **Own (16)**: T-001, T-002, T-003, T-004, T-005, T-006, T-008, T-009, T-010, T-011, T-012, T-013, T-014, T-015, T-018, T-019, T-020 (architectural). All require changes only inside `nebula-telemetry`.
- **Boundary contract (3)**: T-007, T-016, T-021. The crate publishes a stronger primitive or clearer doc; `nebula-metrics` consumes the improved surface (often paired 1:1 with metrics audit findings).
- **Patch (1)**: T-017.
- **Upstream (0)**: nothing in this crate depends on a stronger primitive from elsewhere — telemetry IS the primitive layer.
- **Downstream (0)**: every problem needs at least one telemetry-side change.

Compared with the metrics audit (which had 5 Upstream items pointing INTO this crate — M-004, M-014-overflow, M-017, M-023, M-026), the symmetry is: **`nebula-metrics` cannot honestly claim "stable" until `nebula-telemetry` closes T-002, T-003, T-006, T-019, T-020, and T-012 — those are the upstream prerequisites the metrics audit identified.**

## Sharpened Aggregated Recommendation

The original audit's six-phase plan stands. The new findings tighten three priorities:

1. **Registry identity must be a single source of truth.** T-001 + T-002 + T-011 + T-012 are one problem with four faces: metric handles, MetricKey identity, type binding, and interner generation. Phase 2 (enforce MetricKey/LabelSet/histogram invariants) must address all four together, not in isolation. A half-fix (e.g., private `MetricKey` fields without registry-bound construction) leaves the bypass open via `Histogram::with_buckets`.

2. **Snapshot consistency is a primitive contract, not an exporter convenience.** T-003 + T-013 + T-015 are one problem: the registry returns live handles + per-call O(n) cumulative reads + per-call allocation. Phase 1 (clarify primitive contracts and snapshot semantics) must publish an immutable `MetricSnapshot` family that exporters can consume without retrying for consistency. The metrics audit's M-004 cannot be honestly closed without this.

3. **`retain_recent` and `last_updated_ms` need an ADR-grade decision.** T-005 + T-006-side + T-017 + T-019 + T-020 + T-012 form one architectural thread: the retention feature requires `&mut self`, production holds `Arc`, cached handles desync, timestamps are paid on every write, and `inc_by(0)` semantics are undefined. Either the feature works in production (fix all of them coherently) or it is removed (and the per-write `now_ms()` cost goes with it). Don't ship the half-finished version into "stable" maturity.

## Additional GitHub Issues

### Issue T-011: Make primitive metric handle creation registry-only

Severity: High — Architecture correction.

`Counter::new` / `Gauge::new` / `Histogram::with_buckets` are public, allowing metrics to exist outside any registry. Make them `pub(crate)`; require all handle creation through `MetricsRegistry` accessors.

Acceptance:
- `Histogram::with_buckets(...)` outside the crate fails to compile.
- Existing tests/examples migrate to `registry.histogram_with_buckets_labeled`.

### Issue T-012: Resolve interner generation / cached handle desync

Severity: Critical — Architecture correction (paired with T-005, T-020).

After `compact_interner`, externally-cached `LabelSet`s carry stale Spurs that panic on `resolve()`. Either drop rebuild-style compaction or expose generation IDs.

Acceptance:
- A test caches a `LabelSet`, triggers `retain_recent`, then resolves the cached set; the result is either correct or a typed error — never a panic.

### Issue T-013: Non-allocating histogram snapshot path

Severity: Medium — Refactor (subsumed by T-003).

`Histogram::buckets()` allocates a `Vec` per call and recomputes cumulative on every scrape. Provide `Histogram::snapshot() -> HistogramSnapshot` and a non-allocating iterator alternative.

### Issue T-014: Document lock-free vs wait-free semantics for histogram observe

Severity: Medium — Documentation/test only.

`sum_bits` update is lock-free with retry; bucket and count updates are wait-free. The current docs say "lock-free" without distinction. Add a contention benchmark.

### Issue T-015: Non-allocating `LabelSet::for_each`

Severity: Medium — Refactor.

Exporter scrapes allocate a `Vec<(&str, &str)>` per series. Provide a `for_each(&interner, F)` accessor.

### Issue T-016: Spur-based filter primitive

Severity: Medium — Boundary contract.

`filter_label_set(&[&str])` re-interns the allow list every call. Add `filter_label_set_by_spur(&[Spur])` so `nebula-metrics::LabelAllowlist` can intern its keys once.

### Issue T-017: Fix `now_ms` truncation cast and clock-step semantics

Severity: Patch.

Use `try_into().unwrap_or(u64::MAX)`; document that wall-clock steps backward can collapse to zero with the current `unwrap_or_default` and break retention monotonicity.

### Issue T-018: `Histogram::percentile(p)` validates input

Severity: Medium — API correction.

Return `Option<f64>` or accept a validated `Percentile` newtype; today NaN/negative/>1 silently produce plausible-but-wrong values.

### Issue T-019: Define `last_updated_ms` semantics

Severity: High — API correction.

`inc_by(0)` and `set(unchanged)` bump the timestamp. Define the contract: "metric was touched" or "value changed". If "value changed", short-circuit no-op writes.

### Issue T-020: ADR for retention / `last_updated_ms` / compaction

Severity: High — Architecture correction.

`retain_recent` is `&mut self` but production holds `Arc<MetricsRegistry>`. The feature is unreachable. Decide: drop it, refactor for `Arc`, or document as test-only. Pair with T-005, T-012, and metrics audit M-023.

### Issue T-021: De-couple `DEFAULT_BUCKETS` from "Prometheus" branding

Severity: Medium — Boundary contract (subsumed by metrics M-016).

Either remove the default (force callers to supply boundaries) or rename + describe in primitive-neutral terms. Per-metric bucket layouts belong to the `nebula-metrics` catalog.

## Layering Constraint Check (2026-05-05)

Re-running every classification under the rule:

> Do not expand this crate's role just because a downstream scenario depends on it.
> Use downstream scenarios only to test whether this crate exposes the right contract.

For each finding I asked: "Is this a primitive-layer correctness issue I'm catching via a downstream symptom, or am I using a downstream pain to push policy into telemetry?"

The check found **one over-reach** that I am correcting, plus several borderline cases where the original classification stands but the reasoning needs to be sharpened.

### Correction: T-011 reclassified from Own to Downstream

I previously wrote:

> T-011: Make `Counter::new` / `Gauge::new` / `Histogram::with_buckets` `pub(crate)`; require registry-mediated handle creation.

Re-examining: the downstream pain ("metrics constructed outside the registry are invisible to snapshots, so a `nebula-metrics` catalog metric could be silently orphaned") is real. But the FIX I proposed — restricting primitive constructors — uses that downstream concern to constrain the primitive layer's API surface.

A primitive metric library is allowed to let callers construct standalone `Counter` / `Gauge` / `Histogram` handles. That is a valid use case (e.g., test harnesses, ad-hoc local measurement). The constraint "all *catalog* metrics must go through the shared registry" is `nebula-metrics` policy, not a primitive correctness invariant.

**Correct classification: Downstream.**

- `nebula-telemetry` action: document that primitive handles created outside any `MetricsRegistry` are not enumerated by `snapshot_*`. That is already implicit in the registry-keyed snapshot APIs; make it explicit.
- `nebula-metrics` action: ensure every catalog descriptor's emission path goes through the shared registry. Lint or audit prevents `Counter::new()` / `Histogram::with_buckets()` import from non-test code in catalog-bearing crates. This sits next to the `nebula-metrics` audit's M-021 (forbid raw `&str` registration) — both are catalog-side enforcement, not telemetry-side restriction.

### Borderline cases that survive — and why

For these, I asked the same question and concluded telemetry IS the correct fix site, but the reasoning is worth recording so a future maintainer does not relocate them under pressure:

**T-002 (Type conflicts) — stays Own.**
Test by the rule: "Is one-kind-per-(name, labels) a primitive identity invariant or an exporter policy?"
- `MetricKey` is defined as `{ name, labels }` with no kind field (`crates/telemetry/src/labels.rs:271-294`). The implicit primitive contract is "(name, labels) identifies a metric series".
- Allowing the same `MetricKey` to live in three independent maps means the implicit contract is actually "(name, labels, implicit_kind_from_which_map) identifies a metric series" — and that implicit kind is invisible to consumers.
- The downstream "duplicate `# TYPE` lines" pain is the symptom; the bug is that primitive identity is not what the type signature claims.
- Fix: Own (registry-side conflict detection or explicit `MetricKind` in `MetricKey`).

**T-003 (Histogram snapshot consistency) — stays Own.**
Test: "Is atomic snapshot of `count`/`sum`/`buckets` a primitive contract or an exporter convenience?"
- The exporter's job is to format whatever the primitive gives it. If the primitive cannot give a self-consistent snapshot, no exporter — Prometheus, OTLP, or otherwise — can produce a valid output.
- This is not exporter-specific policy; this is primitive atomicity.
- Fix: Own (seqlock or equivalent snapshot in `Histogram`).

**T-004 (Bucket layout part of identity) — stays Own.**
Test: "Is bucket layout part of metric identity or part of the catalog's metric shape?"
- `nebula-metrics` (per its M-016) decides WHICH layout each catalog metric uses. That is metrics-side policy.
- But for a single registry instance, "two callers of the same `MetricKey` get the same bucket layout" is a primitive identity invariant — without it, the second caller's observations land in unexpected buckets regardless of the catalog.
- Fix: Own (return error on layout mismatch instead of warn). The catalog still owns the choice of layout.

**T-007 (Interner is not cardinality safety) — stays Boundary contract.**
Test: "Should telemetry implement cardinality limits?"
- No. Cardinality policy depends on per-metric schemas the catalog owns. Adding a budget here would push `nebula-metrics`'s LabelAllowlist policy down.
- Fix: telemetry documents the gap clearly; `nebula-metrics` enforces.

**T-016 (filter_label_set re-interns) — stays Boundary contract.**
Test: "Is providing a Spur-based filter primitive expanding telemetry's role?"
- No. The Spur-based primitive is symmetric to the existing `&[&str]`-based one and represents a strictly more efficient access pattern. The policy decision (which keys to allow) stays in `nebula-metrics`.
- Fix: telemetry adds the primitive; `nebula-metrics` migrates to it.

**T-020 (retain_recent vs Arc) — stays Own.**
Test: "Is 'feature is unreachable in production' a downstream-composition complaint that I'm using to force a telemetry change?"
- The pain (`Arc::get_mut` requires unique ownership) is real. But `nebula-metrics` did not invent `Arc`-shared registries — that is the standard pattern for any shared service. If `retain_recent` cannot be called under that pattern, `retain_recent` is the unrealistic API.
- Telemetry can legitimately decide: drop the feature, refactor it for shared ownership, or document it as test-only. All three options are own-crate decisions.
- Fix: Own (ADR-grade decision in telemetry).

**T-021 (DEFAULT_BUCKETS) — stays Boundary contract.**
Test: "Is forcing callers to supply boundaries pushing policy?"
- Borderline. A primitive layer is allowed to provide a sensible default. But the default's NAME ("Prometheus-style") and shape (HTTP-latency-tuned) are catalog-domain. Telemetry can keep a default but should describe it primitively. The actual per-metric layout decision lives in `nebula-metrics` (M-016).
- Fix: telemetry renames/redocuments the default; the catalog provides per-metric layouts that override it.

### Findings That Were Already Correct — Brief Audit

The remaining classifications survived the check without reasoning changes:

- T-001, T-005, T-006, T-008, T-009, T-010, T-012, T-013, T-014, T-015, T-018, T-019 — all genuinely primitive-layer concerns. No downstream scenario was used to push policy down; downstream pain was used as a symptom that revealed a primitive bug.
- T-017 — pure patch.

### Updated Classification Summary

| ID | Original class | Corrected class | Reason for change (if any) |
|----|----------------|-----------------|------------------------------|
| T-001 | Own | Own | unchanged |
| T-002 | Own | Own | unchanged (primitive identity) |
| T-003 | Own | Own | unchanged (primitive atomicity) |
| T-004 | Own | Own | unchanged (registry identity) |
| T-005 | Own | Own | unchanged |
| T-006 | Own | Own | unchanged |
| T-007 | Boundary contract | Boundary contract | unchanged |
| T-008 | Own | Own | unchanged |
| T-009 | Own | Own | unchanged |
| T-010 | Own | Own | unchanged |
| T-011 | Own | **Downstream** | Restricting primitive constructors used downstream pain to expand telemetry's role. Catalog enforcement belongs in `nebula-metrics`. |
| T-012 | Own | Own | unchanged |
| T-013 | Own | Own | unchanged |
| T-014 | Own | Own | unchanged |
| T-015 | Own | Own | unchanged |
| T-016 | Boundary contract | Boundary contract | unchanged |
| T-017 | Patch | Patch | unchanged |
| T-018 | Own | Own | unchanged |
| T-019 | Own | Own | unchanged |
| T-020 | Own (architectural) | Own (architectural) | unchanged — see borderline analysis above |
| T-021 | Boundary contract | Boundary contract | unchanged |

Net telemetry-side count after correction: **15 Own, 3 Boundary contract, 1 Downstream, 1 Patch, 1 Own (architectural)**.

The Downstream item is now explicit: T-011's `nebula-metrics` action ("ensure catalog metrics go through the shared registry; lint against `Counter::new()` / `Histogram::with_buckets()` outside test code") is a metrics-audit issue, mirroring M-021 there. Telemetry's only action is to document that orphan handles exist and are not enumerated by `snapshot_*`.

### Symmetry Check Against the `nebula-metrics` Audit

The metrics audit had 5 Upstream items pointing into telemetry: M-004, M-014-overflow, M-017, M-023, M-026. Re-checking each against the constraint:

| Metrics finding | Maps to telemetry finding | Layering check |
|-----------------|---------------------------|-----------------|
| M-004 (histogram snapshot atomicity) | T-003 | ✓ Telemetry's primitive snapshot contract is the right fix site. |
| M-014-overflow (`_sum` overflows to `Inf`) | T-006 (extended) and primitive saturation contract | ✓ Numeric saturation is a primitive-arithmetic decision, not exporter policy. |
| M-017 (`inc_by(0)` keeps stale series) | T-019 | ✓ `last_updated_ms` semantics is a primitive contract. |
| M-023 (hot-path `now_ms()` cost) | T-020 | ✓ Telemetry decides whether `last_updated_ms` exists at all. |
| M-026 (cached-handle / compaction desync) | T-005 + T-012 | ✓ Compaction is a telemetry-internal feature; its handle/Spur lifecycle is a primitive contract. |

All five are correctly Upstream from the metrics side, which means correctly Own from the telemetry side. The two audits are consistent.

### Layering Anti-pattern Reminder (Sharpened)

The check produced one practical refinement of the anti-pattern list earlier in this section:

> **Do not** "fix" a primitive bug by adding a `pub(crate)` restriction that constrains valid standalone use cases. If the only motivation is a downstream catalog enforcement, leave the primitive open and enforce in the catalog layer.

This is the rule that flagged T-011. It generalizes: every time the proposed fix narrows the primitive API to prevent a downstream-policy violation, ask whether the violation belongs above. If yes, document the contract here and enforce there.
