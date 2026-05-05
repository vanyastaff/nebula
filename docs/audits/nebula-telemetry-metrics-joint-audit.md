# nebula-telemetry + nebula-metrics Joint Architecture & Correctness Audit

Scope:

- `crates/telemetry`: primitive metric storage, identity, atomics, histograms, label interning, snapshots.
- `crates/metrics`: metric naming catalog, label safety, adapter, Prometheus text export, `/metrics` snapshot string.

Evidence reviewed:

- `crates/telemetry/src/metrics.rs`
- `crates/telemetry/src/labels.rs`
- `crates/telemetry/src/error.rs`
- `crates/telemetry/src/lib.rs`
- `crates/telemetry/README.md`
- `crates/metrics/src/adapter.rs`
- `crates/metrics/src/filter.rs`
- `crates/metrics/src/export/prometheus.rs`
- `crates/metrics/src/naming.rs`
- `crates/metrics/src/lib.rs`
- `crates/metrics/README.md`
- runtime call sites in `crates/engine`, `crates/resource`, and `crates/api`

Important dataflow constraint:

- Observability output path: runtime components record observations through telemetry primitives; metrics shapes those primitives into operator-facing output; API serves `/metrics`; Prometheus/Grafana scrape it.
- Decision input path: engine and scheduler decide from `nebula-system` host facts, configuration, policy, and internal state such as queues, leases, cancellation, resources, and execution state. Engine must not make scheduling decisions from `/metrics`. If self-throttling reads counters directly, that is a direct primitive read path, not exporter feedback.

## Executive Summary

The two crates are conceptually sound as a split: `nebula-telemetry` stores primitive metric state, while `nebula-metrics` names, filters, and exports that state. The boundary is directionally right.

The implementation is not yet production-safe as a stable metrics stack. The core risk is not that metrics fail to export. The risk is that they export successfully while encoding unstable primitive identity, unsafe cardinality, invalid or misleading histogram state, or an operator-facing metric contract that can be bypassed by accident.

The boundary is currently too porous:

- `nebula-telemetry` does not prevent primitive identity conflicts, cross-interner label misuse, or same-key different-kind registration.
- `nebula-metrics` documents itself as the safe shaping layer but re-exports `MetricsRegistry`, exposes raw generic accessors, defaults the adapter allowlist to pass-through, and relies on callers to apply `LabelAllowlist`.
- `PrometheusExporter` has useful hardening for escaping and HELP/TYPE grouping, but it assumes telemetry snapshot and identity properties that telemetry does not guarantee.

Can Nebula rely on this metrics stack for production observability today?

Not as a stable normative boundary. It is useful and close, but the current public APIs make unsafe usage easier than the safe path in several important cases. Before treating it as production-stable, Nebula should encode primitive identity/type invariants in `nebula-telemetry`, make label/naming safety mandatory or hard to bypass in `nebula-metrics`, and document the cross-crate snapshot/cardinality contract.

Biggest 3 risks:

1. Primitive identity is not type-safe or registry-bound. Same `MetricKey` can be counter, gauge, and histogram; `LabelSet` can cross registries as raw interner symbols.
2. Cardinality safety is optional ceremony. `LabelAllowlist` is global, default pass-through, manually applied, and bypassable through re-exported primitives and adapter generic accessors.
3. Prometheus output can be syntactically valid while semantically lying: same raw name with different metric kinds can emit duplicate TYPE families, histogram snapshots can be internally inconsistent, and exporter sanitization can hide invalid names rather than failing catalog drift.

Most important design correction:

Create a typed metric catalog and registry boundary:

- `nebula-telemetry` owns a registry-local metric identity table with one `MetricKind` per key, validated histogram bucket layout, and explicit immutable or weak snapshot semantics.
- `nebula-metrics` owns `MetricDescriptor`s with name, kind, unit, help, allowed labels, and label value policy; adapter methods record by descriptor rather than raw strings.
- Re-exported primitives are kept only as low-level escape hatches or moved behind clearly unsafe/custom APIs.

## Responsibility Map

| Concern | Owner Crate | Notes |
|---------|-------------|-------|
| Primitive counter/gauge/histogram correctness | `nebula-telemetry` | Atomic behavior, overflow, histogram bucket placement, count/sum/bucket consistency. |
| Registry identity | `nebula-telemetry` | Same registry key must not mean multiple metric kinds. |
| Label interning | `nebula-telemetry` | Interning and symbol lifetime; not cardinality policy. |
| Snapshot primitive contract | `nebula-telemetry` | Must define whether snapshot is immutable, weak, or live-handle enumeration. |
| Metric catalog | `nebula-metrics` | Names, kinds, units, HELP text, label schema, operator semantics. |
| Label safety | `nebula-metrics` | Dangerous label keys and value policies must be blocked before registry insertion. |
| Prometheus export | `nebula-metrics` | Text format, escaping, HELP/TYPE, histogram family rendering, content type. |
| HTTP serving | Downstream crate | `nebula-api` serves `nebula_metrics::snapshot()` at `/metrics`. |
| Engine scheduling decisions | Downstream crate | Engine/scheduler decisions must use host facts, config, internal queues/leases/state, not `/metrics`. |
| Direct primitive reads for self-throttling | Boundary contract | If needed, engine may read telemetry handles directly; this is not Prometheus feedback. |
| High-cardinality user/plugin labels | `nebula-metrics` | Telemetry may store strings; metrics must prevent unsafe labels before insertion. |
| Valid Prometheus label names and values | `nebula-metrics` | Exporter owns escaping/format. Catalog owns rejecting invalid operator-facing labels. |
| Sensitive label prevention | `nebula-metrics` | Security/privacy policy belongs in shaping layer and plugin-facing APIs. |
| Canon invariants across crates | Cross-crate canon | PRODUCT_CANON/crate docs should state the split and safe path. |

## Critical Findings

| ID | Severity | Area | Responsibility | Problem | Failure Scenario | Recommended Fix |
|----|----------|------|----------------|---------|------------------|-----------------|
| J-001 | Critical | Metric identity | `nebula-telemetry` | `LabelSet` and `MetricKey` are raw `Spur` containers not bound to the `LabelInterner` or `MetricsRegistry` that created them. `MetricKey` fields are public. | A `LabelSet` built with registry A is passed to registry B. Labels resolve to wrong strings, collide, or panic during export. | API correction: make `MetricKey` opaque and bind `LabelSet` to a registry/interner generation or re-intern labels in registry APIs. |
| J-002 | Critical | Registry type safety | `nebula-telemetry` | `MetricsRegistry` uses separate maps for counters, gauges, and histograms, so the same `MetricKey` can be registered as multiple metric kinds. | One crate records `nebula_x` as a counter and another records `nebula_x` as a gauge. The exporter emits the same family name with conflicting TYPE lines. | Architecture correction: use one descriptor table keyed by metric identity with a single `MetricKind`; return a primitive conflict error. |
| J-003 | Critical | Snapshot/histogram consistency | Boundary contract | Telemetry snapshots return live handles and histograms use independent relaxed atomics for buckets, count, and sum; metrics exporter renders them as coherent Prometheus families. | Prometheus scrape sees `count` after an observation but bucket or sum before it; histogram output is internally inconsistent. | Boundary contract/API correction: define weak vs immutable snapshot semantics. Prefer immutable value snapshots for exporter use. |
| J-004 | Critical | Safe path bypass | `nebula-metrics` | `nebula-metrics` re-exports `MetricsRegistry`, `Counter`, `Gauge`, `Histogram`; `TelemetryAdapter` exposes `registry()`, `interner()`, and generic raw `counter/gauge/histogram` methods. | Runtime code records raw names or unsafe labels directly, bypassing catalog constants and `LabelAllowlist`, yet the metrics export successfully. | Architecture correction: make descriptor/adapter recording the normal API; demote raw access to explicit custom/unsafe escape hatch or separate module. |
| J-005 | High | Histogram layout | `nebula-telemetry` | `histogram_with_buckets_labeled()` silently returns the first layout on same-key bucket conflict and only logs a warning. | Two components believe the same latency metric uses different buckets; one records into the wrong distribution contract. | API correction: bucket layout is part of the histogram descriptor; same key different layout must error. |
| J-006 | High | Counter/gauge arithmetic | `nebula-telemetry` | Atomic `fetch_add`/`fetch_sub` wrap on overflow/underflow and no public contract states what happens. | A long-running counter wraps to 0; a gauge wraps negative from `i64::MAX`. Operators see false resets or impossible capacity. | API correction: define checked/saturating/error behavior and test boundary values. |
| J-007 | High | Cardinality guard | `nebula-metrics` | `LabelAllowlist` is default pass-through, global rather than per-metric, manually applied, and strips silently with no diagnostic metric or development failure mode. | `workflow_id` or `execution_id` is stripped and dashboards silently lose a dimension, or pass-through config lets it reach telemetry. | API correction: per-metric label schema with safe-label builder; dev-mode rejection; production diagnostic counter for stripped labels. |
| J-008 | High | Catalog enforcement | `nebula-metrics` | Naming constants exist but are not enforced. Adapter has raw generic methods and many runtime call sites use `MetricsRegistry` directly with constants, not a typed catalog. | A new call site records an ad-hoc name or reuses a name with different labels/units. The exporter accepts it as custom. | API correction: `MetricDescriptor` catalog and tests that every operator metric is recorded by descriptor. |
| J-009 | High | Prometheus type conflicts | Boundary contract | The exporter groups per metric kind but reuses the same exported raw name across counters/gauges/histograms. If telemetry permits type conflicts, exporter emits duplicate HELP/TYPE for one name. | `reg.counter("x")` and `reg.gauge("x")` produce both `# TYPE x counter` and `# TYPE x gauge`. Prometheus scrape is invalid or misleading. | Fix at both sides: telemetry rejects type conflict; metrics adds exporter assertion/test for conflicting families. |
| J-010 | High | Prometheus numeric format | `nebula-metrics` | Exporter writes Rust numeric display directly. Histogram sum can become `inf` from large finite observations; Prometheus text expects `+Inf`, `-Inf`, or `NaN` tokens. | A huge but finite sequence overflows histogram sum to infinity and scrape output contains invalid `inf`. | Patch/refactor: format floats through a Prometheus numeric encoder; test NaN/+Inf/-Inf even if telemetry tries to avoid them. |
| J-011 | High | Retention and handles | `nebula-telemetry` | `retain_recent()` can remove registry entries while cloned metric handles remain live and updateable outside the registry. | A cached `Counter` is evicted, then incremented. Future snapshots do not include the increment. | Architecture correction: remove retention, invalidate handles, or make handles registry-attached/reinsertable. |
| J-012 | High | LabelInterner risk | Boundary contract | `LabelInterner` is exposed and append-only. It reduces repeated allocation but is not cardinality safety. Metrics docs say the allowlist prevents cardinality, but default APIs do not force it. | Plugin emits 100k unique label values. Telemetry interns them and registry grows unbounded. | Cross-crate canon correction plus API changes: document interner non-safety; metrics must gate plugin/user labels before interning/recording. |
| J-013 | High | Exporter sanitization hides catalog bugs | `nebula-metrics` | Exporter sanitizes invalid metric names and label keys, adding hash suffixes on collisions, rather than rejecting operator-facing catalog drift. | A raw metric name with newline exports as a sanitized custom name. Dashboards miss the intended series and the bug looks like a new metric. | API correction: catalog metrics should be validated/rejected before registry insertion; exporter sanitization should be limited to custom escape hatches or diagnostics. |
| J-014 | High | Canon/dataflow clarity | Cross-crate canon | Crate docs do not encode the two data lines: observability output path vs decision input path. No evidence found that engine reads `/metrics`, but canon should forbid that anti-pattern. | A future scheduler reads Prometheus output to throttle itself, creating delayed feedback and scrape-coupled control behavior. | Cross-crate canon correction: document that `/metrics` is output-only; direct primitive reads for self-observation must be explicit and separate. |

## nebula-telemetry Review

### Counter correctness

Evidence:

- `Counter` stores `Arc<AtomicU64>` and `Arc<AtomicU64>` timestamp metadata.
- `inc()` and `inc_by()` use `fetch_add(..., Ordering::Relaxed)`.
- `get()` uses `load(Ordering::Relaxed)`.

Assessment:

- Concurrent increments on a cached handle are atomic and should not be lost.
- Negative increments and decrement/reset are not exposed, which is good for monotonicity.
- Overflow behavior is missing. `AtomicU64::fetch_add` wraps in release builds. That violates the expected operator contract that counters are monotonic except for process restart.
- The timestamp is updated after the value with a separate relaxed store. It is approximate metadata, not an exact update witness.

Owner:

- `nebula-telemetry` owns overflow behavior, atomic semantics, and docs.
- `nebula-metrics` owns whether a named operator metric should be a counter.

### Gauge correctness

Evidence:

- `Gauge` stores `Arc<AtomicI64>`.
- `inc()` and `dec()` use atomic add/sub; `set()` stores an absolute value.

Assessment:

- Negative values are possible. This is fine for a primitive, but the contract should say so.
- Overflow and underflow wrap in release builds.
- Concurrent `set()` racing with `inc()`/`dec()` is atomic but semantically undefined. For point-in-time gauges, users should usually choose either absolute setting or delta updates, not both.
- NaN/Infinity are not possible because the gauge is integer.

Owner:

- `nebula-telemetry` owns integer arithmetic and concurrency semantics.
- `nebula-metrics` owns named gauge meaning and units.

### Histogram correctness

Evidence:

- `Histogram::with_buckets()` validates with `assert!`: boundaries must be non-empty, positive, finite, and strictly increasing.
- `observe()` silently ignores non-finite values.
- Boundary equality maps to the matching finite bucket.
- Counts are stored per bucket plus overflow; `buckets()` returns cumulative finite buckets.
- `count()`, `sum()`, and `buckets()` are independent relaxed reads.

Assessment:

- Bucket placement is mostly correct for finite observations.
- Invalid config panics in library code instead of returning a primitive error.
- Negative finite observations are accepted and land in the first bucket, despite positive bucket validation. This may be acceptable for generic distributions, but it is undocumented and surprising for duration histograms.
- Non-finite observations are silently dropped. This avoids poisoning sum, but hides bad caller behavior.
- The sum is updated through an atomic f64-bit update; concurrent additions are not lost, but floating-point ordering is nondeterministic.
- A finite sequence can overflow sum to infinity. The exporter does not format that as Prometheus `+Inf`.
- `percentile(p)` accepts p outside `0.0..=1.0` and NaN despite docs saying percentile is in that range.
- Same key different bucket layout is not rejected.

Owner:

- `nebula-telemetry` owns bucket validation, observation behavior, and snapshot consistency.
- `nebula-metrics` owns choosing duration bucket layouts for operator metrics and formatting histogram output.

### MetricsRegistry correctness

Evidence:

- `MetricsRegistry` has separate `DashMap<MetricKey, Counter>`, `DashMap<MetricKey, Gauge>`, and `DashMap<MetricKey, Histogram>`.
- Same-type get-or-create uses `entry(...).or_insert_with(...)`.
- Labeled methods accept any `&LabelSet` and clone it.
- Snapshot methods return vectors of `(MetricKey, metric handle)`.
- `retain_recent()` removes stale map entries and compacts the interner.

Assessment:

- Same key same type behaves as expected.
- Same key different type is allowed and is a primitive correctness bug.
- Same histogram key different buckets is allowed with warning-only semantics.
- The registry is unbounded. Cardinality budgets belong in `nebula-metrics`, but telemetry docs must say registry/interner growth is unbounded.
- Snapshot order is not deterministic because `DashMap` iteration order is not stable.
- Retention conflicts with cached handles and can make later updates invisible to the registry.

### MetricKey / LabelSet identity

Evidence:

- `LabelKey` and `LabelValue` are type aliases to `lasso::Spur`.
- `LabelSet` stores `Vec<(LabelKey, LabelValue)>`.
- `LabelInterner::label_set()` interns, sorts by key symbol, and deduplicates duplicate keys with last value wins.
- `MetricKey` has public `name` and `labels` fields.

Assessment:

- Within one interner, label order is canonicalized.
- Duplicate label keys are deterministic but silently overwritten.
- `LabelSet` identity is not portable across registries because `Spur` values are interner-local.
- `MetricKey` can be manually constructed from arbitrary symbols and labels.
- Empty keys/values are allowed at primitive level. Export validity belongs to `nebula-metrics`, but the primitive should not imply exporter validity.

### LabelInterner correctness

Evidence:

- `LabelInterner` wraps `lasso::ThreadedRodeo`.
- Docs state interned strings stay resident until registry compaction.
- `resolve()` can panic for foreign symbols; `try_resolve()` exists.
- `filter_label_set()` is a low-level utility used by `LabelAllowlist`.

Assessment:

- Interning is reasonable in the primitive layer for repeated labels.
- It is not cardinality safety.
- Exposing `LabelInterner` and raw `Spur`-backed types encourages callers to treat symbols as globally meaningful.
- Filtering after a label set has been constructed still interns dangerous key/value strings. To prevent interner growth from unsafe values, the safe path should filter or validate before interning user/plugin-provided dynamic values.

### Atomic/concurrency review

Relaxed atomics are acceptable for independent metric values if the contract says reads are approximate and no ordering with application memory is implied.

They are not enough for compound histogram invariants:

- `observe()` updates bucket, total count, sum, and timestamp separately.
- `buckets()`, `count()`, and `sum()` read those atomics separately.
- `snapshot_histograms()` returns live handles, so the exporter observes a moving target.

No evidence shows a strong histogram family snapshot contract. That contract must be either provided by telemetry or explicitly downgraded for metrics.

### Snapshot semantics

Current telemetry snapshot APIs are better described as live registry enumeration than snapshot:

- They allocate vectors of current map entries.
- Returned metric handles remain live and mutable.
- Inclusion during concurrent registration is best-effort.
- Values are read later by the exporter, not captured at snapshot time.

Required correction:

- Rename/document as weak live enumeration, or provide immutable value snapshots.
- If weak, `nebula-metrics` must not claim stronger consistency.
- If immutable, telemetry must decide how much histogram consistency is guaranteed.

### Hot-path allocation review

Cached handle updates are plausibly allocation-free:

- `Counter::inc()`
- `Gauge::inc()/dec()/set()`
- `Histogram::observe()`

Not allocation-free:

- `LabelInterner::label_set()` allocates a `Vec`, interns strings, sorts, and deduplicates.
- Registry lookup touches `DashMap` and builds/clones `MetricKey`.
- `LabelSet::resolve()` allocates a `Vec`.
- `Histogram::buckets()` allocates a `Vec`.
- Export snapshots allocate and format strings.

The docs should define "hot path" as cached handle update, not dynamic label construction plus registry lookup.

### Error model review

Evidence:

- `TelemetryError` only has `Io`.
- Invalid buckets panic.
- Histogram bucket conflicts warn.
- Non-finite observations are silently dropped.

Assessment:

The error model is not useful for primitive metric correctness. `nebula-metrics` cannot assert or react to primitive failures because they are not represented.

Needed primitive errors:

- Invalid bucket layout.
- Metric kind conflict.
- Histogram layout conflict.
- Foreign label set/key.
- Checked arithmetic overflow if chosen.
- Invalid observation if chosen.

### Layering review

The crate mostly avoids implementing exporter/catalog policy, but there are small leaks:

- `crates/telemetry/examples/basic_metrics.rs` uses canonical-looking `nebula_*` names.
- Telemetry README mentions naming/export boundaries, which is fine, but examples should stay primitive-neutral.
- Histogram docs call defaults Prometheus-style.
- `tracing::warn!` is used for bucket conflict instead of returning a primitive error.

Do not move naming, label allowlists, Prometheus, OTLP, HTTP, or dashboards into telemetry. Do move primitive identity/type/snapshot correctness into telemetry.

## nebula-metrics Review

### Naming catalog review

Evidence:

- `crates/metrics/src/naming.rs` defines many `NEBULA_*` constants.
- The module convention says `nebula_<domain>_<metric>_<unit>`.
- Tests cover some groups for uniqueness, prefix, snake_case, and registry safety.
- `TelemetryAdapter` uses constants for its typed workflow/action/eventbus accessors.
- Runtime call sites often use constants directly with `MetricsRegistry`.

Strengths:

- Most constants use the `nebula_` prefix.
- Duration metrics generally use `_seconds`.
- Many event counters use `_total`.
- Several bounded label value sets are represented as constants.

Risks:

- The catalog is not a descriptor catalog. It lacks a single typed source of truth for name, kind, unit, help, allowed label keys, value domains, and bucket layout.
- Constants are not enforced at registry insertion. Raw names can be recorded through `TelemetryAdapter::counter/gauge/histogram`, `TelemetryAdapter::registry()`, the re-exported `MetricsRegistry`, or direct `nebula_telemetry` imports.
- Some names encode gauges with total-like words, such as credential active/expired totals. The current exporter classifies these by how they were registered, not by descriptor.
- Exporter HELP text is a separate match table from naming constants, creating drift risk.

Owner:

- `nebula-metrics` owns catalog discipline and name semantics.
- `nebula-telemetry` should not validate `nebula_*` or units.

### LabelAllowlist / cardinality review

Evidence:

- `LabelAllowlist::all()` passes every label.
- `Default` is `all()`.
- `TelemetryAdapter::new()` uses `LabelAllowlist::all()`.
- `LabelAllowlist::apply()` strips keys not on a global list.
- Labeled adapter methods call `filter_labels()` before recording.
- Generic/raw registry paths do not apply the allowlist.

Strengths:

- The allowlist removes dangerous keys when callers use the labeled adapter path with `only(...)`.
- Tests prove that one labeled adapter method strips an unallowed key.

Risks:

- Filtering is optional and default-off.
- The allowlist is global, not per metric.
- Unknown labels are silently stripped.
- There is no diagnostic counter for stripped labels.
- There is no dev/test rejection mode.
- There is no label value length limit.
- There is no value-domain enforcement.
- Filtering after `LabelSet` construction may already have interned unsafe high-cardinality values.
- Plugin/user label policy is not encoded.

Dangerous labels that must never pass as metric labels:

- `execution_id`, `run_id`, `trace_id`, `span_id`, `request_id`, `correlation_id`
- `user_id`, high-cardinality `tenant_id`
- user-created `workflow_id` or `workflow_name`
- dynamic `node_id`, per-instance `action_id`
- email, raw URL, query string, file path, IP address
- unbounded hostname
- `error_message`, `error_debug`, arbitrary plugin data

Usually safe bounded labels:

- `status`, `outcome`, `error_class`, `action_kind`, `resource_kind`
- `trigger_kind`, `retryable`, `platform`, `component`
- bounded `method`
- `endpoint_template`, not raw URL

### TelemetryAdapter review

Evidence:

- Typed workflow/action/eventbus methods use naming constants.
- Labeled typed methods apply `filter_labels()`.
- Generic `counter`, `gauge`, and `histogram` methods accept arbitrary names.
- `registry()` exposes the underlying registry.
- `interner()` exposes the underlying interner.
- Default allowlist is pass-through.

Strengths:

- It makes common workflow/action metric names easy to access.
- It applies allowlist on the specific labeled adapter methods.
- It has eventbus snapshot recording helpers with clamping for large values.

Risks:

- It is a convenience wrapper, not an enforcement boundary.
- It cannot enforce per-metric label schemas.
- It exposes enough raw access to bypass its own safeguards.
- It has no typed event methods for important workflow outcomes such as timeout/cancel/rejection/fallback/retry.
- Same metric can be emitted with different label keys because schema is not encoded.
- Adapter tests cover only a subset of accessors and label filtering.

### Prometheus exporter review

Evidence:

- `content_type()` returns `text/plain; version=0.0.4; charset=utf-8`.
- `snapshot()` groups counters, gauges, and histograms by exported metric name using `BTreeMap`.
- HELP and TYPE lines are emitted before samples within each family.
- Label values escape backslash, quote, newline, carriage return, and tab.
- Metric names and label keys are sanitized.
- Colliding sanitized label keys and metric names are disambiguated with a hash suffix.
- Histogram output includes finite buckets, `+Inf`, `_sum`, and `_count`.

Strengths:

- Basic Prometheus text format is implemented.
- Label value escaping is present.
- HELP/TYPE grouping has been considered.
- Histogram `+Inf`, `_sum`, and `_count` are present.

Risks:

- Sanitizing invalid operator-facing names/label keys hides catalog bugs. A stable metric should be rejected before insertion, not renamed at export.
- Same raw metric name with different metric kind is not disambiguated; because `allocate_exported_metric_name` maps raw name to a single exported name, type conflicts produce duplicate TYPE families.
- Hash suffixes use `DefaultHasher`; the code calls it stable, but Rust does not promise this as a long-term external naming contract.
- Sample order within a family follows snapshot entry order and is not deterministic.
- Exporter reads live histogram handles, so buckets/count/sum may not match.
- Float formatting is direct Rust display. If histogram sum becomes infinity, Rust prints `inf`, while Prometheus expects `+Inf`.
- HELP text is not escaped for arbitrary custom names/help, though current custom HELP strings are static and safe.

### Histogram export review

Strengths:

- Cumulative finite buckets are emitted using telemetry `hist.buckets()`.
- `le` labels are added correctly.
- `+Inf` bucket uses `hist.count()`.
- `_sum` and `_count` are emitted.
- Custom histogram buckets are used when present.

Risks:

- Telemetry can return a histogram under a bucket layout different from the caller requested.
- Export consistency depends on live relaxed reads.
- Count can disagree with cumulative buckets under concurrent observation.
- Sum can be non-finite after large finite observations.
- No duplicate-series check after label sanitization and type grouping.

### Operator usefulness review

Current catalog helps with:

- Workflow starts/completions/failures/duration.
- Action executions/failures/duration/rejections.
- Resource lifecycle/acquire/wait/usage/pool exhaustion.
- Credential refresh coordinator outcomes.
- Webhook signature failures.
- Eventbus snapshots.

Gaps:

- Workflow failure taxonomy is too coarse for operations: timeout, cancellation, engine shutdown, user action error, retry exhausted, and panic are not consistently separated at the catalog level.
- Action retries and fallback semantics are not represented as first-class metrics in the base adapter.
- Queue backlog, scheduler pressure, saturation, and circuit breaker state are incomplete or spread across domains.
- Labels are not metric-specific, so the same metric can lose or gain dimensions over time.
- Operators cannot know whether stripped labels were attempted.

### Security/privacy review

Main privacy risk:

The unsafe path can accept labels containing IDs, emails, URLs, paths, raw errors, or plugin-provided data. Exporter escaping prevents text-format breakage for values, but it does not prevent sensitive data exposure or cardinality explosion.

Additional risks:

- `/metrics` is unauthenticated in `crates/api/src/routes/metrics.rs`, as is common for Prometheus, so labels must be treated as public-ish.
- Sanitizing label keys does not sanitize label values semantically.
- Silent stripping may hide attempted sensitive labels from tests and operators.

Owner:

- `nebula-metrics` owns label safety and privacy policy.
- `nebula-api` owns endpoint exposure/auth/network policy.

### Performance review

Risks:

- Dynamic labels cause interning, allocation, sorting, and map lookup before filtering if the caller builds a `LabelSet` first.
- Snapshot/export clones registry entries, resolves labels, allocates strings, and formats all samples.
- `DashMap` iteration plus BTreeMap grouping is acceptable for moderate cardinality but will be expensive under high-cardinality misuse.
- Multiple concurrent scrapes each call `snapshot()` and allocate full output.
- Histogram `buckets()` allocates for every histogram during every scrape.

Do not optimize prematurely, but benchmark the intended scale and high-cardinality failure mode.

### Layering review

`nebula-metrics` correctly owns naming/export/allowlist concepts. The problem is enforcement:

- It re-exports low-level primitives.
- It exposes the underlying registry from the adapter.
- It defaults label filtering to pass-through.
- It relies on telemetry's primitive snapshot/identity contract without that contract being strong enough.

## Cross-Crate Boundary Review

### What telemetry owns

- Atomic counter/gauge/histogram correctness.
- Registry-local metric identity.
- Metric kind conflicts.
- Histogram bucket layout conflicts.
- Label interner symbol lifetime and non-portability.
- Primitive snapshot consistency contract.
- Cached-handle hot-path behavior.

### What metrics owns

- `nebula_*` naming and catalog.
- Units, HELP text, and metric kind at operator level.
- Label safety and privacy.
- Per-metric label schema.
- Prometheus text rendering.
- Adapter semantics.

### What must not cross the boundary

Do not move into telemetry:

- `nebula_*` prefix policy.
- Prometheus HELP/TYPE metadata.
- Prometheus escaping/content type.
- Label allowlists or dangerous-label policy.
- Operator dashboards/alerts.
- OTLP exporter.

Do not move into metrics:

- Atomic overflow behavior.
- Registry type conflicts.
- Interner symbol identity.
- Histogram bucket layout conflicts.
- Primitive snapshot correctness.

### Where API allows bypass

- `nebula_metrics::MetricsRegistry` re-export.
- `nebula_metrics::Counter/Gauge/Histogram` re-exports.
- `TelemetryAdapter::registry()`.
- `TelemetryAdapter::interner()`.
- `TelemetryAdapter::counter/gauge/histogram`.
- Direct `nebula_telemetry::metrics::MetricsRegistry` imports in runtime crates.
- Public `LabelInterner`, `LabelSet`, and `MetricKey`.

### Where contracts are missing

- Whether a telemetry snapshot is a frozen value snapshot or weak live enumeration.
- Whether one metric identity can ever change type.
- Whether histogram bucket layout belongs to identity.
- Whether label symbols are portable across registries.
- Whether label filtering should happen before interning user/plugin labels.
- Whether operator-facing invalid names/labels should be rejected or sanitized.
- Whether stripped labels are observable.
- Whether `/metrics` is output-only canon and not an engine scheduling input.

## Missing Invariants

| Invariant | Owner | Currently encoded in types? | Currently tested? | Risk |
|-----------|-------|-----------------------------|-------------------|------|
| Counter increments are monotonic and overflow behavior is explicit. | `nebula-telemetry` | No | No overflow test | Silent counter wrap. |
| Gauge arithmetic defines overflow/underflow/NaN/Infinity behavior. | `nebula-telemetry` | Partially integer-only | No boundary tests | Silent gauge wrap or undefined set/delta races. |
| Histogram bucket config is valid and stable. | `nebula-telemetry` | Runtime asserts only | Partial | Panic or wrong layout. |
| Histogram observations map deterministically to buckets. | `nebula-telemetry` | Mostly | Partial | Negative/non-finite behavior unclear. |
| Same `MetricKey` cannot be registered as multiple metric kinds. | `nebula-telemetry` | No | No | Invalid exporter families. |
| Same histogram `MetricKey` cannot use incompatible bucket layouts. | `nebula-telemetry` | No | Test currently asserts first-layout wins | Wrong distributions. |
| `LabelSet` identity is independent of input label order. | `nebula-telemetry` | Yes within one interner | Yes | Cross-interner identity still unsafe. |
| Duplicate label keys are rejected or canonicalized deterministically. | Boundary contract | Last-wins | Yes | Silent dimension overwrite. |
| `LabelInterner` symbols are not treated as globally stable unless guaranteed. | `nebula-telemetry` | No | No | Foreign label corruption. |
| Primitive snapshot consistency is documented. | `nebula-telemetry` | No | No | Exporter assumes too much. |
| Metric names are defined in `nebula-metrics` naming catalog. | `nebula-metrics` | No | Partial constants tests | Ad-hoc names drift. |
| Operator-facing metric names use `nebula_*` prefix. | `nebula-metrics` | No | Partial | Collision and dashboard drift. |
| Metric names encode units where applicable. | `nebula-metrics` | No | Partial by convention | Unit confusion. |
| Same metric name cannot have multiple incompatible label schemas. | `nebula-metrics` | No | No | Query and dashboard breakage. |
| High-cardinality labels do not reach telemetry registry through metrics layer. | `nebula-metrics` | No | One adapter test | Memory/TSDB explosion. |
| Prometheus output is valid for all legal label values. | `nebula-metrics` | Mostly | Partial | Broken scrape on unusual numeric/string values. |
| Histogram export includes `+Inf`, `_sum`, and `_count`. | `nebula-metrics` | Yes in exporter | Yes | Core histogram family shape is mostly covered. |
| Sensitive data never appears in labels. | `nebula-metrics` | No | No | Privacy leak through unauthenticated scrape. |
| Engine decisions do not read `/metrics`. | Cross-crate canon | No | No | Feedback-loop anti-pattern. |

## Real Nebula Scenarios

| # | Scenario | Owner | Current behavior | Caller/operator expectation | What could go wrong | Invariant or test |
|---|----------|-------|------------------|-----------------------------|---------------------|-------------------|
| 1 | 100 workers increment executions_started concurrently. | `nebula-telemetry` | Atomic counter increments are not lost. | Count eventually equals starts. | Overflow behavior still undefined. | Concurrent counter plus overflow test. |
| 2 | 100 workers observe action duration concurrently. | `nebula-telemetry` | Atomic updates occur independently. | Histogram family is coherent. | Count/sum/buckets skew under scrape. | Concurrent observe plus snapshot test. |
| 3 | Scheduler updates running_execution gauge while workers complete. | `nebula-telemetry` | `set` and deltas race. | Gauge reflects current running count. | Delta lost relative to set semantics. | Gauge race contract test. |
| 4 | Registry snapshot happens during heavy updates. | Boundary contract | Snapshot returns live handles. | Export view is consistent enough. | Moving values and inconsistent histograms. | Snapshot contract test. |
| 5 | Two crates request same counter with same name and labels. | `nebula-telemetry` | Same-type map returns shared handle. | Shared series. | Works if label set is from same registry. | Same-key same-type concurrency test. |
| 6 | Two crates request same metric key with different metric types. | `nebula-telemetry` | Allowed in separate maps. | Rejected. | Duplicate Prometheus TYPE. | Type-conflict rejection test. |
| 7 | Same histogram key requested with different bucket layouts. | `nebula-telemetry` | First layout wins with warning. | Rejected or descriptor match. | Wrong bucket contract. | Bucket conflict test. |
| 8 | Labels are passed in different order by two call sites. | `nebula-telemetry` | Canonical within one interner. | Same series. | Cross-registry labels unsafe. | Order and foreign-label tests. |
| 9 | Duplicate label key is passed by mistake. | Boundary contract | Last wins silently. | Reject or documented canonicalization. | Dimension overwritten. | Duplicate key policy test. |
| 10 | Plugin records `execution_id` as a label. | `nebula-metrics` | Allowed if bypassing allowlist or pass-through default. | Blocked before registry. | Cardinality explosion. | Dangerous-label block test. |
| 11 | Plugin creates 100k unique label values. | `nebula-metrics` | Telemetry interns and registry grows. | Safe API prevents it. | Memory and TSDB DoS. | High-cardinality simulation. |
| 12 | Exporter resolves labels during scrape. | Boundary contract | Uses registry interner and `resolve`. | Symbols belong to registry. | Foreign labels can panic or resolve wrong. | Foreign LabelSet rejection test. |
| 13 | Counter reaches max in long-running process. | `nebula-telemetry` | Wraps. | Explicit reset only on process restart. | False reset. | Counter overflow test. |
| 14 | Gauge set races with increment. | `nebula-telemetry` | Atomic but semantic race. | Current state remains meaningful. | Lost update relative to intent. | Mixed gauge operation test. |
| 15 | Histogram observes NaN or Infinity. | `nebula-telemetry` | Silently ignored. | Caller can detect invalid value. | Bad duration math hidden. | Non-finite observation test. |
| 16 | Histogram observes value exactly on bucket boundary. | `nebula-telemetry` | Included in matching bucket. | `<= le` semantics. | Likely correct. | Boundary property test. |
| 17 | Metric name constant exists but adapter uses raw string. | `nebula-metrics` | Some typed adapter methods use constants; generic methods allow raw. | Catalog constants are normative. | Drift not caught. | Adapter constants test. |
| 18 | Allowlist strips `workflow_id` silently and dashboard loses dimension. | `nebula-metrics` | Unknown key stripped without diagnostic. | Dev/test sees rejection or metric. | Dashboard query surprises. | Stripped-label diagnostic test. |
| 19 | Prometheus label value has quote, slash, newline, unicode. | `nebula-metrics` | Quote/backslash/newline escaped; slash/unicode preserved. | Valid text format. | Mostly OK; add unicode and CR/TAB tests. | Escaping tests. |
| 20 | Two call sites emit same metric name with different label keys. | `nebula-metrics` | Allowed. | Per-metric schema fixed. | Queries miss series or aggregate wrong. | Label schema conflict test. |
| 21 | Action succeeds after retries. | `nebula-metrics` | Base action metric counts dispatched execution; retry semantics not encoded. | Operators can distinguish attempts vs final success. | Retry storms hidden. | Retry attempt/final outcome metrics. |
| 22 | Action fails permanently. | `nebula-metrics` | Failure counter increments on runtime error. | Permanent failure separated from transient attempts. | Failure taxonomy too coarse. | Outcome/error_class schema test. |
| 23 | Action is cancelled by engine shutdown. | `nebula-metrics` | No clear base catalog dimension. | Cancelled is not failed user code. | Operators misread cancellations as failures or silence. | Cancellation outcome metric. |
| 24 | Action times out. | `nebula-metrics` | Timeout not first-class in base adapter. | Timeout visible as bounded outcome/error_class. | Timeouts hidden in generic failures. | Timeout scenario test. |
| 25 | Circuit breaker rejects before action execution. | `nebula-metrics` | Dispatch rejected metric exists for some reasons. | Rejection separate from execution/failure. | Missing reason/schema for circuit breaker. | Rejection reason closed set test. |
| 26 | Fallback succeeds after primary failure. | `nebula-metrics` | No base fallback metric. | Fallback success visible separately. | Success hides primary failure. | Fallback outcome metric test. |
| 27 | Resource pool is exhausted. | `nebula-metrics` | Constants exist for exhausted and waiters. | Saturation visible. | Label schema/bounds not enforced. | Resource label schema test. |
| 28 | Queue backlog grows. | `nebula-metrics` / downstream | No clear queue backlog catalog in reviewed adapter. | Operators see backlog/saturation. | Scheduler pressure invisible. | Queue gauge descriptor test. |
| 29 | API endpoint returns many 500s. | `nebula-metrics` / downstream | No generic API RED catalog observed in adapter. | Rate/errors/duration by endpoint_template/status. | API incidents lack metrics. | API RED metric scenario. |
| 30 | Prometheus scrapes while metrics update. | Boundary contract | Exporter reads live handles. | Scrape remains valid and meaningful. | Histogram inconsistency; duplicate type if conflicts. | Concurrent scrape/update test. |

## API Misuse Cases

| Misuse | Current API Allows It? | Failure Mode | Correct Owner | Recommended Prevention |
|--------|------------------------|--------------|---------------|------------------------|
| Record arbitrary raw metric names through `MetricsRegistry`. | Yes | Catalog drift and invalid names. | `nebula-metrics` | Descriptor-only normal path; custom metric escape hatch. |
| Record arbitrary raw names through `TelemetryAdapter::counter`. | Yes | Bypasses naming constants. | `nebula-metrics` | Remove or rename to `custom_counter_unchecked`. |
| Bypass `LabelAllowlist` by using `MetricsRegistry` directly. | Yes | High-cardinality labels reach registry. | `nebula-metrics` | Safe adapter/descriptor API must own label construction. |
| Use `LabelAllowlist::all()` in production. | Yes and default | Dynamic labels pass through. | `nebula-metrics` | Default to deny/descriptor labels; require explicit custom pass-through. |
| Use `execution_id` as label. | Yes | Registry and TSDB cardinality explosion. | `nebula-metrics` | Dangerous-label denylist plus per-metric schemas. |
| Use `trace_id`/`span_id` as label. | Yes | Metrics become traces and explode. | `nebula-metrics` | Reject trace identifiers as labels. |
| Use workflow name as label. | Yes | User-created cardinality. | `nebula-metrics` | Prefer workflow_kind closed set; reject raw names. |
| Use raw URL/path/query as label. | Yes | Privacy leak and cardinality. | `nebula-metrics` | Use endpoint_template only. |
| Use error message as label. | Yes | Sensitive data and unbounded cardinality. | `nebula-metrics` | Use bounded error_class/outcome. |
| Same `MetricKey` different type. | Yes | Duplicate/invalid exporter families. | `nebula-telemetry` | Registry type conflict error. |
| Same histogram key different bucket layout. | Yes | Wrong distribution. | `nebula-telemetry` | Bucket layout conflict error. |
| Label order changes identity. | No within one interner | Works locally. | `nebula-telemetry` | Keep canonicalization; add property tests. |
| Foreign label set crosses registries. | Yes | Wrong labels or panic. | `nebula-telemetry` | Registry-bound labels or re-interning. |
| Duplicate label keys. | Yes, last wins | Silent overwrite. | Boundary contract | Reject or document; schemas should prevent. |
| Assume `snapshot_*` is atomic. | Easy | Inconsistent histogram/read view. | Boundary contract | Rename/document or immutable snapshots. |
| Assume Prometheus exporter validates any string. | Partially | Sanitized names hide bugs. | `nebula-metrics` | Validate catalog names before insertion. |
| Assume LabelInterner is a cardinality guard. | Easy | Memory growth. | Cross-crate canon | Docs and safe API block unsafe labels. |
| Assume engine reads `/metrics` for scheduling. | Not observed, but possible future misunderstanding | Delayed scrape feedback loop. | Cross-crate canon | Document output vs decision input paths. |
| Cache handle after `retain_recent`. | Yes | Updates invisible after eviction. | `nebula-telemetry` | Remove/define retention-handle semantics. |
| Emit same metric name with different label schemas. | Yes | Broken queries and aggregation. | `nebula-metrics` | Descriptor label schema. |

## Recommended Test Plan

### P0

- Concurrent counter increments with many threads.
- Concurrent gauge updates including set vs inc/dec contract.
- Concurrent histogram observations plus snapshot/export during updates.
- Same `MetricKey` different metric type rejected by telemetry.
- Same histogram key different bucket layout rejected or explicitly documented.
- `LabelSet` order canonicalization property test.
- Foreign `LabelSet`/cross-registry misuse rejected or re-interned.
- Duplicate label key handling test with documented behavior.
- `LabelAllowlist` blocks dangerous labels such as `execution_id`, `trace_id`, `workflow_id`, `error_message`, raw URL.
- Adapter uses naming constants for all typed methods.
- Adapter applies allowlist for every labeled method.
- Prometheus label escaping for quote, backslash, newline, carriage return, tab, slash, and unicode.
- Histogram export includes finite buckets, `+Inf`, `_sum`, and `_count`.
- Exporter rejects or detects same name different type.
- Snapshot during concurrent updates remains within documented contract.
- High-cardinality label attack simulation through safe and unsafe paths.

### P1

- Counter overflow behavior near `u64::MAX`.
- Gauge overflow/underflow behavior near `i64::MAX` and `i64::MIN`.
- Histogram boundary values, below-first, above-last, negative, NaN, and Infinity observations.
- Histogram invalid bucket config for empty, duplicate, unsorted, zero, negative, NaN, and Infinity.
- Deterministic snapshot/export ordering or explicit exporter sorting.
- Concurrent registry get-or-create same key.
- Concurrent registry get-or-create same key different type.
- Interner concurrent same string and many unique strings.
- Raw string metric detection in production code.
- Duplicate metric name with different label schema.
- Duplicate metric name with different type.
- Docs examples compile and use the intended safe path.
- Prometheus float formatting for `+Inf`, `-Inf`, and `NaN`.

### P2

- Benchmarks for hot-path counter update.
- Benchmarks for gauge update.
- Benchmarks for histogram observe.
- Benchmarks for registry lookup with cached vs dynamic labels.
- Benchmarks for label interning under contention.
- Benchmarks for snapshot generation and export.
- Fuzz/property tests for `LabelSet` canonicalization.
- Property tests for histogram bucket placement.
- Scrape pressure tests with concurrent `snapshot()` calls.

## Recommended Benchmark Plan

### Counter hot path

- Cached `Counter::inc()` single-thread.
- Cached `Counter::inc()` with 64 workers.
- Registry lookup plus counter increment.
- Counter with last_updated timestamp disabled/enabled if configurable later.

### Gauge hot path

- Cached `Gauge::set()`.
- Cached `Gauge::inc()/dec()`.
- Mixed `set` and delta contention.

### Histogram hot path

- Cached `Histogram::observe()` default buckets.
- Cached `Histogram::observe()` custom many buckets.
- High-contention observe on one histogram.
- Many histograms with moderate cardinality.

### Registry lookup

- Existing metric lookup.
- New metric registration.
- Concurrent same-key registration.
- Concurrent high-cardinality registration.

### Label interning

- Repeated same label pairs.
- Many unique values.
- Concurrent same-string interning.
- Concurrent unique-string interning.
- Filtering before vs after interning.

### Snapshot/export

- Snapshot/export 1k, 10k, 100k counters.
- Snapshot/export 1k, 10k, 100k histograms.
- Concurrent scrapes.
- Export with long label values.

### High-cardinality misuse

- Dynamic `execution_id` labels.
- Dynamic workflow names.
- Raw URLs and paths.
- Plugin arbitrary key/value labels.

## Recommended Refactor Plan

### Phase 1: clarify boundaries and contracts

- Document output path vs decision input path.
- Document that engine must not consume `/metrics` for scheduling decisions.
- Document cached-handle hot path vs dynamic registration path.
- Document telemetry snapshot consistency exactly.
- Document that `LabelInterner` is not cardinality safety.

### Phase 2: enforce primitive identity invariants in nebula-telemetry

- Make `MetricKey` opaque.
- Bind `LabelSet` to registry/interner identity or re-intern in registry APIs.
- Add registry type conflict detection.
- Add histogram bucket layout conflict detection.
- Add primitive error variants.
- Define counter/gauge overflow behavior.

### Phase 3: enforce naming and label safety in nebula-metrics

- Introduce `MetricDescriptor` catalog with name, kind, unit, help, allowed labels, and bucket layout.
- Make adapter methods descriptor-based.
- Replace global allowlist with per-metric label schemas.
- Add safe label builder that filters/rejects before interning dynamic values.
- Add stripped/rejected label diagnostics.

### Phase 4: harden histogram and snapshot semantics

- Provide immutable telemetry snapshot structs or explicitly weak snapshot structs.
- Ensure exporter assumptions match telemetry snapshot contract.
- Format Prometheus floats through a valid numeric encoder.
- Reject type conflicts before export.

### Phase 5: add concurrency/cardinality/export tests

- Add P0 telemetry concurrency tests.
- Add P0 metrics cardinality and exporter tests.
- Add high-cardinality misuse simulations.
- Add duplicate schema/type tests.

### Phase 6: improve docs and canon invariants

- Add L2 canon invariants for both crates.
- Move telemetry examples away from canonical `nebula_*` names.
- Document safe path for runtime components.
- Document custom metric escape hatch and risks.

## Proposed Canon Invariants

| Proposed Invariant | Owner Crate | Why Nebula Needs It | How To Encode | How To Test |
|--------------------|-------------|---------------------|---------------|-------------|
| A registry metric identity maps to exactly one primitive metric kind. | `nebula-telemetry` | Prevents invalid exporter families. | Single descriptor table or type conflict map. | Register counter then gauge same key must error. |
| Histogram bucket layout is immutable for a metric identity. | `nebula-telemetry` | Bucket layout defines distribution meaning. | Store layout in descriptor; compare on lookup. | Same key different buckets must error. |
| `LabelSet` is registry/interner-bound or safely re-interned. | `nebula-telemetry` | Prevents foreign symbol corruption. | Opaque label token or interner generation. | Cross-registry label set test. |
| Counter overflow behavior is explicit. | `nebula-telemetry` | Operators cannot trust wrapping counters. | Checked/saturating/error contract. | Near-max counter test. |
| Gauge arithmetic semantics are explicit. | `nebula-telemetry` | Current-state metrics need defined races/bounds. | Docs plus checked/saturating update loops if chosen. | Boundary and set/delta race tests. |
| Histogram snapshots state their consistency level. | `nebula-telemetry` | Exporter must not assume impossible atomicity. | Immutable snapshots or weak snapshot docs. | Snapshot under update test. |
| Operator-facing metrics are defined by `nebula-metrics` descriptors. | `nebula-metrics` | Names, units, help, kind, labels must not drift. | Static catalog type. | Catalog completeness test. |
| Every operator metric name uses `nebula_*` and encodes unit where applicable. | `nebula-metrics` | Stable dashboards and alerts. | Descriptor validation. | Catalog validation test. |
| Every metric has a fixed label schema. | `nebula-metrics` | Prevents query breakage and cardinality drift. | Per-metric label schema. | Same name different labels test. |
| Dangerous labels are rejected before telemetry insertion. | `nebula-metrics` | Prevents memory/TSDB explosion and privacy leaks. | SafeLabels builder and plugin sandboxing. | Dangerous-label test. |
| Stripped/rejected labels are observable in development/test. | `nebula-metrics` | Silent dimension loss is an operations bug. | Dev error plus production diagnostic counter. | Unknown label test asserts diagnostic. |
| Prometheus output is valid for every legal telemetry value. | `nebula-metrics` | Scrapes must not break under edge values. | Prometheus numeric/string encoder. | Escaping and non-finite tests. |
| `/metrics` is output-only and not an engine decision input. | Cross-crate canon | Avoids delayed feedback control loops. | PRODUCT_CANON and crate docs. | Search/doc test or architecture checklist. |
| Cardinality policy never lives in `nebula-telemetry`. | Cross-crate canon | Keeps primitive layer reusable and narrow. | Canon docs and dependency/lint checks. | Search test for allowlist policy in telemetry. |

## GitHub Issues

### Issue J-001: Bind telemetry metric identity to its registry/interner

Severity: Critical

Responsibility: `nebula-telemetry`

Classification: API correction

Problem:
`LabelSet` and `MetricKey` are raw `Spur` containers. `MetricKey` fields are public, and registry labeled methods accept any `&LabelSet`.

Failure scenario:
A label set built from registry A is passed to registry B. Export resolves symbols against the wrong interner, causing wrong labels, collisions, or panic.

Recommended fix:
Make `MetricKey` opaque and bind `LabelSet` to an interner identity/generation, or make registry APIs accept raw pairs and re-intern internally.

Acceptance criteria:

- Foreign label sets cannot be recorded silently.
- Public code cannot construct arbitrary metric keys from raw symbols.
- Cross-registry tests cover collision and resolution failure cases.

### Issue J-002: Reject same telemetry MetricKey registered as multiple metric kinds

Severity: Critical

Responsibility: `nebula-telemetry`

Classification: Architecture correction

Problem:
`MetricsRegistry` stores counters, gauges, and histograms in separate maps. Same identity can exist as multiple types.

Failure scenario:
The exporter renders the same raw metric name as both counter and gauge, creating conflicting Prometheus families.

Recommended fix:
Use one registry descriptor table keyed by metric identity and return a `TelemetryError` on metric kind conflict.

Acceptance criteria:

- Same key same type succeeds.
- Same key different type errors.
- Concurrent conflict test passes deterministically.

### Issue J-003: Define cross-crate snapshot semantics for telemetry and metrics

Severity: Critical

Responsibility: Boundary contract

Classification: API correction

Problem:
Telemetry snapshot APIs return live handles, and histograms are read through independent relaxed atomics. Metrics exporter renders the result as coherent Prometheus output.

Failure scenario:
Scrape observes histogram count, sum, and buckets from different moments.

Recommended fix:
Define the snapshot contract. Prefer immutable telemetry snapshot values for exporter use. If weak, document it and test exporter behavior under concurrent updates.

Acceptance criteria:

- Snapshot docs are explicit.
- Concurrent scrape/update test proves the chosen contract.
- Exporter assumptions match telemetry docs.

### Issue J-004: Make the nebula-metrics safe path hard to bypass

Severity: Critical

Responsibility: `nebula-metrics`

Classification: Architecture correction

Problem:
`nebula-metrics` re-exports primitives and adapter exposes raw registry, interner, and arbitrary name methods. Label allowlist is optional.

Failure scenario:
A runtime component records a dynamic label directly into the registry. The metric exports successfully but creates unbounded series.

Recommended fix:
Make descriptor/adapter recording the normal path. Move raw primitive access behind explicit custom/unchecked APIs or a separate module with strong docs.

Acceptance criteria:

- Normal runtime docs use descriptor/adapter APIs.
- Generic raw methods are clearly custom/unchecked or removed.
- Tests prove labeled adapter paths apply schemas/allowlists.

### Issue J-005: Reject histogram bucket layout conflicts

Severity: High

Responsibility: `nebula-telemetry`

Classification: API correction

Problem:
Same histogram key with different buckets returns the first layout and warns.

Failure scenario:
One component records latency with buckets it did not actually get.

Recommended fix:
Bucket layout must be part of the histogram descriptor and conflict must return a primitive error.

Acceptance criteria:

- Same layout succeeds.
- Different layout fails.
- Existing first-layout-wins test is replaced.

### Issue J-006: Define counter/gauge overflow and underflow behavior

Severity: High

Responsibility: `nebula-telemetry`

Classification: API correction

Problem:
Counter and gauge atomic arithmetic wraps in release builds.

Failure scenario:
Long-running counters reset silently; gauges cross numeric bounds and become impossible.

Recommended fix:
Use checked/saturating arithmetic or return errors; document and test the behavior.

Acceptance criteria:

- Boundary tests for counter and gauge.
- Public docs state behavior.

### Issue J-007: Replace global optional LabelAllowlist with per-metric label schemas

Severity: High

Responsibility: `nebula-metrics`

Classification: API correction

Problem:
`LabelAllowlist` is global, default pass-through, manually applied, and silently strips unknown labels.

Failure scenario:
Unsafe labels pass through in default configuration, or useful labels are stripped silently and dashboards lose dimensions.

Recommended fix:
Add per-metric label schemas and a safe label builder. Reject unknown labels in dev/test and emit production diagnostics for stripped/rejected labels.

Acceptance criteria:

- Dangerous labels are blocked before registry insertion.
- Unknown label behavior is observable.
- Each descriptor defines allowed labels.

### Issue J-008: Make the naming catalog a typed MetricDescriptor catalog

Severity: High

Responsibility: `nebula-metrics`

Classification: API correction

Problem:
Name constants are normative by documentation but not enforced in code. HELP text, kind, unit, bucket layout, and label schema live separately or by convention.

Failure scenario:
A metric name is reused with different unit or labels; exporter treats it as custom and dashboards drift.

Recommended fix:
Create `MetricDescriptor` with name, kind, unit, help, allowed labels, and buckets. Adapter records by descriptor.

Acceptance criteria:

- Every operator metric has one descriptor.
- Descriptor tests verify prefix, snake_case, unit suffix, kind, help, labels.
- Adapter typed methods use descriptors.

### Issue J-009: Detect Prometheus type conflicts during export

Severity: High

Responsibility: Boundary contract

Classification: Refactor

Problem:
Exporter can emit duplicate HELP/TYPE families if telemetry contains same raw name under multiple metric kinds.

Failure scenario:
`counter("x")` and `gauge("x")` both export as `x`, one counter family and one gauge family.

Recommended fix:
Primary fix is telemetry type conflict rejection. Exporter should also detect conflicts and fail or emit a diagnostic rather than invalid text.

Acceptance criteria:

- Export test covers same raw name different type.
- Exporter no longer emits duplicate TYPE for one exported name.

### Issue J-010: Add Prometheus numeric encoding for non-finite floats

Severity: High

Responsibility: `nebula-metrics`

Classification: Patch/Refactor

Problem:
Exporter writes `f64` values with Rust display. Histogram sum can overflow to infinity and print `inf`, which is not the Prometheus token.

Failure scenario:
Huge finite observations make `_sum inf`, causing scrape parse failure.

Recommended fix:
Add a Prometheus number formatter that emits `+Inf`, `-Inf`, and `NaN` as required.

Acceptance criteria:

- Tests for finite, NaN, positive infinity, and negative infinity formatting.
- Histogram sum uses the formatter.

### Issue J-011: Fix retention semantics for cached telemetry handles

Severity: High

Responsibility: `nebula-telemetry`

Classification: Architecture correction

Problem:
`retain_recent()` removes registry entries while cloned handles remain live.

Failure scenario:
Cached handle increments after eviction are invisible to snapshots.

Recommended fix:
Remove retention from primitive layer, make handles generational/invalidated, or reattach handles on update.

Acceptance criteria:

- Test covers retain then update cached handle.
- Docs explain the chosen contract.

### Issue J-012: Document and enforce that LabelInterner is not a cardinality guard

Severity: High

Responsibility: Boundary contract

Classification: Cross-crate canon correction

Problem:
Telemetry interning deduplicates repeated labels but does not bound cardinality. Metrics allowlist is not mandatory.

Failure scenario:
Plugin emits many unique label values and telemetry stores all of them.

Recommended fix:
Document interner semantics in canon and crate docs. Ensure metrics safe APIs block high-cardinality labels before interning/recording.

Acceptance criteria:

- Docs state interner is not cardinality safety.
- High-cardinality misuse test exists.
- Metrics layer owns policy; telemetry stays primitive.

### Issue J-013: Stop exporter sanitization from hiding operator metric catalog bugs

Severity: High

Responsibility: `nebula-metrics`

Classification: API correction

Problem:
Exporter sanitizes invalid names and keys, producing custom exported names instead of rejecting catalog drift.

Failure scenario:
A newline in a metric name becomes `_`, dashboards miss the intended series, and the bug looks like a new metric.

Recommended fix:
Validate catalog metrics before registry insertion. Keep exporter sanitization only for explicit custom metrics or diagnostic fallback.

Acceptance criteria:

- Catalog names/labels are validated at descriptor definition.
- Invalid operator-facing names cannot be recorded silently.
- Exporter tests cover custom fallback separately.

### Issue J-014: Add canon for metrics output path vs engine decision input path

Severity: High

Responsibility: Cross-crate canon

Classification: Cross-crate canon correction

Problem:
The docs do not clearly encode that `/metrics` is an observability output and not a scheduler decision input.

Failure scenario:
A future scheduler reads Prometheus scrape output to self-throttle, creating delayed feedback loops and coupling decisions to scrape health.

Recommended fix:
Add canon/docs stating that engine decisions use host facts, config, policy, and internal state. Direct primitive reads are allowed only as explicit self-observation, not exporter feedback.

Acceptance criteria:

- Canon docs distinguish the two data lines.
- Engine/scheduler docs refer to internal state and `nebula-system`, not `/metrics`.
- Review checklist flags scrape-output control loops.

---

# Independent Re-pass (2026-05-05)

This section aligns the joint audit with the two per-crate audits that were independently re-passed and corrected for layering:

- `docs/audits/nebula-telemetry-architecture-audit.md` — 21 findings (T-001..T-021), including the layering-constraint check that reclassified T-011.
- `docs/audits/nebula-metrics-architecture-audit.md` — 26 findings (M-001..M-026), with full layering classification.

The original joint findings J-001..J-014 above remain valid. This re-pass adds:

1. A cross-reference matrix mapping each joint finding to its per-crate evidence (so a maintainer can drill from J-### → M-### / T-###).
2. New joint findings the first pass missed because they only become visible when the two crates are read together.
3. The explicit 7-point safe-path answer the prompt requires.
4. A layering check that flags any J-### whose recommended fix would push policy into the wrong crate.

## Cross-Reference Matrix: J-### ↔ T-### / M-###

| Joint finding | Telemetry side | Metrics side | Cross-crate notes |
|---------------|----------------|---------------|--------------------|
| J-001 (LabelSet/MetricKey not registry-bound) | T-001 (CONFIRMED) | — | Cross-interner identity is owned in telemetry; metrics needs the fix to be possible at all. |
| J-002 (Same MetricKey multi-kind) | T-002 (CONFIRMED) | M-003-registration | Telemetry owns registration-side rejection; metrics's M-003-export side is the visible symptom. |
| J-003 (Snapshot/histogram inconsistency) | T-003 (CONFIRMED, deepened) | M-004 | Boundary contract: telemetry must publish atomic `HistogramSnapshot`; metrics consumes it. |
| J-004 (Safe-path bypass) | T-011 (Downstream after correction) | M-001, M-010, M-021 | The catalog enforcement lives in metrics. Telemetry's `pub` constructors are not the problem; metrics's prelude re-export and missing `MetricName` are. |
| J-005 (Bucket layout silent override) | T-004 (CONFIRMED) | M-016 | Telemetry-owned identity; metrics-side bucket schemas in catalog are the policy half. |
| J-006 (Counter/gauge overflow) | T-006 (CONFIRMED) | M-014-overflow (relates) | Primitive arithmetic; metrics's `_sum inf` rendering is a separate metrics-side symptom. |
| J-007 (Cardinality guard optional) | T-007 (CONFIRMED) | M-001, M-002, M-009 | Boundary contract: telemetry documents non-guarantee; metrics enforces with per-metric `LabelSchema`. |
| J-008 (Catalog enforcement absent) | — | M-006, M-021, M-024 | Metrics-only. The catalog is metrics's contract. |
| J-009 (Prometheus type conflicts) | T-002 | M-003-export | Boundary contract: telemetry rejects at registration (T-002); metrics asserts at export. Both halves needed. |
| J-010 (Numeric `inf` rendering) | T-006 (sum overflow root) | M-014-rendering | Boundary: telemetry needs saturation-aware sum (upstream half of M-014); metrics renders `+Inf` not `inf`. |
| J-011 (Retention/cached handles) | T-005, T-012, T-020 | M-023, M-026 | Joint: every record pays `now_ms()` for an unreachable feature; cached handles desync; ADR-grade telemetry decision required. |
| J-012 (LabelInterner ≠ cardinality safety) | T-007 | M-002, M-009 | Cross-crate canon: telemetry documents; metrics enforces. |
| J-013 (Exporter sanitization hides bugs) | — | M-011, M-021 | Metrics-only. Reject at descriptor registration. |
| J-014 (Output vs decision dataflow canon) | — | — | Cross-crate canon, no code change in either crate; canon doc + review checklist. |

This matrix means: closing J-### usually requires action in BOTH per-crate audits' findings. The joint document is the only place that records that pairing.

## Additional Joint Findings (J-015..J-022)

These only become visible at the joint level — none of them surfaced as a single per-crate finding.

### J-015: High — Catalog descriptors do not specify bucket schemas

The `nebula-metrics` catalog (`crates/metrics/src/naming.rs`) holds names and label-value enums. It does NOT carry bucket layout. As a result:

- The telemetry-side default `DEFAULT_BUCKETS` (T-021) is the de-facto policy for every histogram metric in the workspace.
- Metrics like `nebula_workflow_execution_duration_seconds` and `nebula_resource_acquire_wait_duration_seconds` use the same HTTP-shape buckets (top 10s) that are wrong for them (M-016).
- There is no place to put the right answer: catalog has no slot for buckets; telemetry's `with_buckets` is per-call, not per-name.

Owner: Boundary contract. The `MetricDescriptor` proposal in M-006 / J-008 must include `BucketSchema`, and `nebula-metrics` must drive registration with the descriptor's buckets via `histogram_with_buckets_labeled`.

### J-016: High — Pre-registration is impossible without a typed catalog

Zero-traffic processes do not expose stable catalog families (M-024). The fix is to walk descriptors at registry construction. But the catalog is not a typed structure today — it is a set of `pub const &str` names. There is nothing to walk.

Owner: Metrics own. Without J-008 (typed `MetricCatalog`), J-016 cannot be solved.

### J-017: High — `last_updated_ms` semantics is undefined across the boundary

`Counter::inc_by(0)` (T-019) bumps `last_updated_ms`. `LabelAllowlist::apply` post-intern (M-002) is applied per record. Together, these mean:

- A "liveness ping" (`counter.inc_by(0)`) keeps the series alive for `retain_recent` (T-005), if it ever ran (T-020).
- The same liveness ping causes the allowlist to re-intern the allowed keys (T-016), even when the value did not change.

The semantics question — "is `last_updated_ms` last-touched or last-changed?" — has implications on both crates. The joint answer should be one consistent contract, not two crate-local interpretations.

Owner: Telemetry own (T-019), but the metrics-side adapter pattern depends on the answer.

### J-018: Medium — Tests for joint behavior are absent

Both per-crate audits note thin concurrency test coverage. The joint problem is sharper:

- No test asserts that a metric recorded through `TelemetryAdapter::action_executions_labeled` produces, under concurrent load, a Prometheus scrape with valid `+Inf == _count == sum_of_finite_buckets`.
- No test asserts that `LabelAllowlist::apply` running on the production prelude path keeps `MetricsRegistry::interner_len()` bounded under attack.
- No test asserts that the catalog-to-export round-trip preserves HELP / TYPE / kind across all 50+ constants.

Owner: Both crates. The test suite that proves "the safe path actually works end-to-end" does not exist today.

### J-019: Medium — `nebula-metrics` exporter does not consume primitive identity

Today the exporter loops over `snapshot_counters` / `snapshot_gauges` / `snapshot_histograms` and groups by raw name. If telemetry adds `MetricKind` to identity (J-002 / T-002), the exporter could iterate ONE descriptor table and never see cross-kind reuse. The current exporter shape commits to the broken telemetry model.

Owner: Boundary contract. Telemetry's J-002 fix should also reshape `snapshot_*` so the exporter receives `(MetricKey, MetricKind, value/handle)` tuples or one unified iterator.

### J-020: Medium — `Histogram::clone()` semantics rely on `Arc` sharing without docs

Both crates assume "cloning a metric handle shares the underlying atomics" (catalog adapter caches handles; engine caches handles). This is true today (`crates/telemetry/src/metrics.rs:159-169`) but undocumented. If telemetry ever decides to invalidate handles after `compact_interner` (J-011 fix), the clone-share contract changes.

Owner: Telemetry own. Document the clone-share contract as part of the cached-handle hot-path docs (T-008).

### J-021: Medium — `MetricsRegistry::clone()` cheap-clone via `Arc` is incompatible with `&mut self` API

`MetricsRegistry: Clone` is cheap (`crates/telemetry/src/metrics.rs:398-405`). `retain_recent(&mut self)` and `compact_interner(&mut self)` require unique ownership. Production composition patterns (engine / api / resource sharing) make `&mut self` unreachable. This is T-020 from the telemetry audit and M-023 from the metrics audit; jointly, it's a contract mismatch the README of neither crate calls out.

Owner: Telemetry own (decide on `&mut`-ed feature future); metrics audit M-023 is the downstream visibility of the same problem.

### J-022: High — No reverse-flow contract: how should engine self-throttle on its own counters?

The dataflow constraint says "engine reads telemetry handles directly, not Prometheus output". But neither crate documents how this is supposed to work. Specifically:

- Does the engine cache `Counter` handles long-term and read `.get()` periodically?
- If telemetry ever invalidates handles (J-011 fix), what happens to that read path?
- Should there be a stable "metric reader" API that survives compaction, separate from the exporter?

Today the engine has no self-throttling-on-own-counters code path, so this is latent. It becomes a real design question the moment any scheduler wants to throttle based on its own observed counters.

Owner: Cross-crate canon. The PRODUCT_CANON extension that locks in "decision input vs observability output" should also state "if any decision input comes from telemetry handles, document the read pattern explicitly".

## Layering Constraint Check

Per the rule:

> Do not expand a crate's role just because a downstream scenario depends on it. Use downstream scenarios only to test whether the crate exposes the right contract.

Re-examining each J-### proposed fix:

| Joint finding | Proposed fix location | Layering check | Verdict |
|----------------|------------------------|----------------|---------|
| J-001 | telemetry | Identity correctness is primitive. | ✓ Own (telemetry) |
| J-002 | telemetry | Type-per-key is primitive identity. | ✓ Own (telemetry) |
| J-003 | telemetry primitive + metrics consumer | Atomicity is primitive; rendering is shaping. | ✓ Boundary contract |
| J-004 | metrics | Catalog enforcement is shaping policy. The earlier instinct "make Counter::new pub(crate)" was the layering trap; correct fix is metrics-side enforcement (mirrors T-011 reclassification). | ✓ Own (metrics) |
| J-005 | telemetry | Histogram bucket identity is primitive. | ✓ Own (telemetry) |
| J-006 | telemetry | Primitive arithmetic. | ✓ Own (telemetry) |
| J-007 | metrics | Cardinality policy is shaping. | ✓ Own (metrics) |
| J-008 | metrics | Catalog is shaping. | ✓ Own (metrics) |
| J-009 | both | Telemetry rejects at registration; metrics asserts at export. Symmetric Boundary contract. | ✓ Boundary contract |
| J-010 | metrics rendering + telemetry sum overflow | Rendering = own (metrics). Sum overflow = own (telemetry). Joint = Boundary contract. | ✓ Boundary contract |
| J-011 | telemetry | ADR-grade decision about retention is primitive. | ✓ Own (telemetry) |
| J-012 | docs both | Cardinality policy is shaping; interner non-guarantee is primitive doc. | ✓ Cross-crate canon |
| J-013 | metrics | Sanitization is exporter rendering. | ✓ Own (metrics) |
| J-014 | docs cross-workspace | Pure canon. | ✓ Cross-crate canon |
| J-015 | metrics catalog (descriptor includes buckets); telemetry pre-registers | Catalog descriptor shape = own (metrics). Telemetry already supports per-key buckets. | ✓ Own (metrics) with telemetry already adequate |
| J-016 | metrics | Requires J-008. | ✓ Own (metrics) |
| J-017 | telemetry | `last_updated_ms` semantics is primitive contract. | ✓ Own (telemetry) |
| J-018 | both | Tests live in both crates' suites. | ✓ Own each side |
| J-019 | telemetry exposes kind-tagged iterator; metrics consumes | Boundary contract. | ✓ Boundary contract |
| J-020 | telemetry docs | Primitive contract documentation. | ✓ Own (telemetry) |
| J-021 | telemetry | Architectural decision is primitive. | ✓ Own (telemetry) |
| J-022 | cross-crate canon | Reverse-flow contract is workspace-level. | ✓ Cross-crate canon |

No fix moves catalog/naming/Prometheus/allowlist policy into telemetry. No fix moves primitive correctness into metrics.

## The 7-Point Safe-Path Answer

> Can a runtime component record a metric through the intended safe path such that:
> 1. the primitive value is stored correctly under concurrency;
> 2. the metric identity is canonical and type-safe;
> 3. dangerous labels are blocked before registry insertion;
> 4. the metric name and unit are catalog-defined;
> 5. Prometheus output is valid;
> 6. operators can trust the result;
> 7. callers cannot easily bypass the safe path by accident?

**Today (2026-05-05), the answer is NO on points 2, 3, 4, 5, 6, 7. Point 1 holds for Counter and Gauge under simple usage; fails for Histogram under concurrent observe + scrape.**

| Point | Status | Joint findings | Owner |
|-------|--------|-----------------|-------|
| 1. Stored correctly under concurrency | **Partial** | J-003 / T-003 / M-004 (histograms only) | telemetry |
| 2. Identity canonical and type-safe | **No** | J-001, J-002, J-005 / T-001, T-002, T-004 | telemetry |
| 3. Dangerous labels blocked before insertion | **No** | J-007, J-012 / T-007, M-002, M-009 | metrics (with telemetry boundary primitive T-016) |
| 4. Name and unit catalog-defined | **No** | J-004, J-008, J-013 / M-001, M-006, M-010, M-021 | metrics |
| 5. Prometheus output valid | **No** | J-009, J-010, J-013, M-005-cross-series, M-014-rendering, M-024 | metrics (with telemetry's J-002 prerequisite) |
| 6. Operators can trust the result | **No** | J-008 / M-007, M-008, M-013, M-018, M-019 | metrics |
| 7. Cannot easily bypass | **No** | J-004 / M-001, M-010, M-021 | metrics |

The minimal coordinated work to flip every "No" to "Yes":

1. Telemetry: J-001, J-002, J-003, J-005, J-006, J-011 (Phase 2 of refactor plan).
2. Metrics: J-004, J-007, J-008, J-013, J-015, J-016, plus the cumulative-vs-event naming corrections (M-018/M-019) and unit-typed adapter (M-025) (Phase 3).
3. Joint: J-009, J-010, J-019 (Phase 4).
4. Tests: J-018 (Phase 5).
5. Canon: J-012, J-014, J-022 (Phase 6).

Marking either crate "stable" before this work lands is a maturity claim the public API does not back.

## Summary Index Across the Three Audits

The Nebula observability stack is described by three documents that should be read together:

| Document | Scope | Findings |
|----------|-------|----------|
| `nebula-telemetry-architecture-audit.md` | Primitive layer | T-001..T-021 + layering check |
| `nebula-metrics-architecture-audit.md` | Shaping layer | M-001..M-026 + layering classification |
| `nebula-telemetry-metrics-joint-audit.md` (this file) | Cross-crate stack | J-001..J-022 + cross-reference matrix + 7-point answer |

The per-crate audits are the source of truth for evidence per finding. The joint audit is the source of truth for cross-crate dependencies, layering verification, and the operator-facing trust question.

## Additional Joint Findings From This Re-pass (J-023..J-026)

These four were not surfaced by either per-crate audit nor by J-001..J-022. They become visible only when reading the two crates' clone / lifetime / format contracts together.

### J-023: Critical — `MetricsRegistry::clone()` + `compact_interner` silently fork the registry

`crates/telemetry/src/metrics.rs:398-405` derives `Clone` on `MetricsRegistry`. The clone shares each field's `Arc` (interner, three `DashMap`s). Two clones see the same atomics for any series.

`crates/telemetry/src/metrics.rs:603-642` (`compact_interner`, called from `retain_recent`) does:

```rust
self.interner = new_interner;
self.counters = Arc::new(new_counters);
self.gauges = Arc::new(new_gauges);
self.histograms = Arc::new(new_histograms);
```

It replaces the **fields of `self`** with fresh `Arc`s. Any clone made before the call still points at the OLD `Arc<DashMap>`s and the OLD `LabelInterner`. After compaction, the two registry instances are independent:

- Writes through the post-compaction registry hit the new maps.
- Writes through the pre-compaction clone hit the old maps.
- Snapshots from either side see only their half.

Because `retain_recent(&mut self)` requires unique ownership, this can happen via `(*arc_reg).clone()` — taking an owned copy from an `Arc` without dropping the Arc itself, then calling `retain_recent` on the copy. The resulting "compacted" registry is a different object that no production component sees.

This is a sharper failure mode than T-005 (orphan handles) and T-012 (stale Spurs in cached LabelSets): not just handles desyncing, but **two registries diverging silently** after compaction. The `Clone` contract suggests "this is the same registry"; the `compact_interner` contract makes that false.

Owner: telemetry-own (architectural). Either:
- (a) `retain_recent` mutates the inner Arcs in-place (requires interior mutability through `Arc<RwLock<...>>` or similar), so all clones see the result, OR
- (b) `MetricsRegistry: Clone` is removed and the registry is held only as `Arc<MetricsRegistry>`, with `retain_recent` taking `Arc<Self>` and using `Arc::try_unwrap`/atomic-swap patterns, OR
- (c) the feature is dropped (paired with J-011).

Classification: Architecture correction.

### J-024: High — `LabelInterner::clone()` produces divergent interners after compaction

`crates/telemetry/src/labels.rs:131-134` derives `Clone` on `LabelInterner`. Cloning shares `Arc<ThreadedRodeo>`. The registry exposes the interner via `pub fn interner(&self) -> &LabelInterner` (`crates/telemetry/src/metrics.rs:424-426`), and external callers commonly write:

```rust
let interner = registry.interner().clone();
```

The cloned interner is `Arc`-shared with the registry's. They stay in sync until `compact_interner` runs. Then:

- The registry's `interner` field points at the NEW `Arc<ThreadedRodeo>`.
- The external clone still points at the OLD `Arc<ThreadedRodeo>`.

Spurs interned through the external clone after compaction are valid in the OLD rodeo but not in the NEW one. If those Spurs end up in a `LabelSet` passed back to the registry (via `counter_labeled(name, &labels)`), the registry's NEW interner cannot resolve them. Export then panics in `interner.resolve(spur)` (`crates/telemetry/src/labels.rs:155-161`) — exactly the T-012 panic, but now triggerable through a normal-looking pattern (`registry.interner().clone()`) that no documentation flags as dangerous.

Owner: telemetry-own. Document the contract: cloned interner becomes detached after compaction. Or remove the lifetime-extending `Clone` and force callers to `&` the registry's interner.

Classification: API correction (paired with J-011 / T-012).

### J-025: Medium — Cached metric handles cross-pollute `last_updated_ms` between unrelated holders

`Counter::clone()`, `Gauge::clone()`, `Histogram::clone()` all share `Arc<AtomicU64>` for `last_updated_ms` (`crates/telemetry/src/metrics.rs:32-35, 80-83, 159-169`). Two crates that both fetched the same `(name, labels)` series get cloned handles that share the timestamp atomic.

Consequence: `retain_recent` (if it ran — see J-021) cannot distinguish between "production code touched this metric" and "a one-off test or debug script touched it". Both are visible as a single `last_updated_ms` reading. Test isolation is impossible without dropping every cached handle.

This compounds J-017 (`inc_by(0)` semantics undefined): not only is "touched vs changed" undefined for one writer, the timestamp also conflates writers across modules.

Owner: telemetry-own. The fix is paired with J-011 / J-021 (decide retention's future). If retention stays, the `last_updated_ms` semantics need a design that survives shared ownership.

Classification: Architecture correction (telemetry).

### J-026: High — `content_type()` hardcodes legacy Prometheus text format with no OpenMetrics path

`crates/metrics/src/export/prometheus.rs:42-43, 348-352`:

```rust
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";
pub fn content_type() -> &'static str { PROMETHEUS_CONTENT_TYPE }
```

The exposition format (text 0.0.4) is the legacy one. Prometheus 2.x and the upstream ecosystem support **OpenMetrics** (`application/openmetrics-text; version=1.0.0; charset=utf-8`), which:

- Has stricter escaping rules.
- Supports exemplars (trace-to-metric correlation — useful with `nebula-tracing` if it exists).
- Supports `# UNIT` lines for unit metadata (which would let M-016 / M-019 surface units to operators directly).
- Requires `# EOF` terminator the current exporter does not emit.

The exporter is structurally tied to the legacy format. The catalog has no concept of "format target", so adding OpenMetrics later means dual-track exporters AND descriptor extension. The README mentions OTLP as planned but does not call out the text-format gap.

Operator consequence: any future migration to OpenMetrics (which most Prometheus deployments now prefer) is a breaking change that the catalog cannot mediate today.

Owner: metrics-own. The fix is paired with J-008 (`MetricDescriptor` should carry unit + stability metadata that maps cleanly to either format). The exporter should be parameterized by format target rather than hardcoded.

Classification: Architecture correction (deferred — not blocking today, but blocking the OTLP/OpenMetrics roadmap). 

---

These four extend the safe-path analysis above:

- J-023 + J-024 mean the `Arc`-share assumption that production code relies on (engine/api/resource holding `Arc<MetricsRegistry>` + cached `interner().clone()`) breaks at the first compaction. The `Clone` semantics published by both `MetricsRegistry` and `LabelInterner` are not "same shared backing"; they are "same shared backing until anyone calls `retain_recent`". Today nobody calls `retain_recent` in production (J-021 / T-020), so the bug is latent — but the API contract is a trap for anyone who reads the docstring and concludes the feature is safe to call.
- J-025 makes the J-017 `last_updated_ms` semantics question harder: even with a clean per-write contract, shared timestamps across cloned handles collapse different writers into one liveness signal.
- J-026 frames the metrics layer's format choice as a strategic decision the catalog does not currently let us defer.

All four are joint-level: each requires reading the clone semantics in telemetry alongside the consumption pattern in metrics or downstream crates to surface.
