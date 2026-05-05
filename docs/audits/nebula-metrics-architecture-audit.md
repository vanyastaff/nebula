# nebula-metrics Architecture & Correctness Audit

## Executive Summary

`nebula-metrics` is conceptually the right layer: `nebula-telemetry` owns raw in-memory primitives, while `nebula-metrics` should own names, label safety, operator-facing descriptors, and Prometheus export. The current implementation does not yet encode that boundary strongly enough to be treated as the normative production metrics contract.

Nebula can use the crate as a convenience exporter today, but should not rely on it as the stable metrics boundary until the API forces callers through a metric catalog and a label schema. The current design lets metrics export successfully while bypassing naming, bypassing cardinality protection, and producing Prometheus output that can be invalid for conflicting families or sanitized label-key collisions.

Biggest 3 risks:

1. Label safety is optional and late. `TelemetryAdapter::new` defaults to `LabelAllowlist::all()` (`crates/metrics/src/adapter.rs:34-40`), exposes the raw registry (`crates/metrics/src/adapter.rs:77-81`), and the allowlist filters an already-interned `LabelSet` (`crates/metrics/src/adapter.rs:68-75`, `crates/telemetry/src/labels.rs:196-200`). Stripped high-cardinality values can still bloat the append-only interner (`crates/telemetry/src/labels.rs:108-114`).
2. The exporter does not validate metric-family invariants. `MetricsRegistry` stores counters, gauges, and histograms in separate maps (`crates/telemetry/src/metrics.rs:399-405`), so the same raw name can exist under multiple types. `snapshot()` emits each type family independently (`crates/metrics/src/export/prometheus.rs:250-343`), which can produce duplicate `# HELP` / `# TYPE` lines for one metric name.
3. Histogram snapshots are not atomic enough for Prometheus histogram invariants. `Histogram::observe` updates bucket count, total count, and sum as separate relaxed atomics (`crates/telemetry/src/metrics.rs:235-245`), while the exporter reads `count`, `sum`, and buckets independently (`crates/metrics/src/export/prometheus.rs:315-334`). Under concurrent updates, `+Inf` / `_count` can disagree with finite buckets.

Most important design correction: replace the optional adapter and raw registry surface with a metric catalog API that registers a `MetricDescriptor` containing name, kind, unit, help, and per-metric label schema, then records only through `SafeLabels` / typed label values.

## Critical Findings

| ID | Severity | Area | Problem | Failure Scenario | Recommended Fix |
|----|----------|------|---------|------------------|-----------------|
| M-001 | Critical | Cardinality / API | Label filtering is optional and bypassable. `TelemetryAdapter` defaults to all labels, exposes `registry()`, generic `counter/gauge/histogram`, and `MetricsRegistry` is re-exported. | A plugin or call site records `execution_id` or raw URL labels directly through `MetricsRegistry`; Prometheus sees 10,000+ series and the interner grows without bound. | Architecture correction: make the safe API the only public recording path from this crate; move raw primitives behind an explicit unsafe/telemetry import. |
| M-002 | Critical | Cardinality / Memory | `LabelAllowlist` filters after labels and values are already interned. The docs claim unsafe labels are stripped before reaching the registry, but `LabelInterner::label_set` interns all pairs first. | A caller builds labels with `execution_id`, filters it out, and still retains every execution ID in the append-only interner. The exported series is safe, but memory is not. | API correction: accept raw key/value pairs into a `SafeLabels` builder that checks keys before interning values. |
| M-003 | Critical | Prometheus Export | Same metric name can be registered as multiple kinds, and `snapshot()` emits each map as an independent family. | One crate records `nebula_action_duration_seconds` as a histogram and another records a gauge of the same name. Scrape output contains conflicting `# TYPE` lines. | Architecture correction: registry wrapper must enforce one descriptor per name and reject kind conflicts before export. |
| M-004 | Critical | Histogram / Concurrency | Histogram export can violate cumulative bucket and `_count` invariants under concurrent observations. | Prometheus scrapes while an observation has incremented a finite bucket but not `total_count`; `le="5"` exceeds `le="+Inf"` and `_count`. | Refactor/API correction: provide atomic histogram snapshot data from `nebula-telemetry`, or lock/seqlock histogram reads during export. |
| M-005 | High | Prometheus Export | Sanitized label-key collisions are only disambiguated within one sample, not across series. | Series `{a-b="x"}` and `{a b="x"}` both export as `{a_b="x"}`, creating duplicate samples for the same metric/label set. | Refactor: allocate exported label keys per metric family/schema and detect cross-series collisions. |
| M-006 | High | Naming / Catalog | Naming constants are not the full exporter catalog. Several constants are not top-level re-exported, and several known metrics get `"Custom counter."` / `"Custom histogram."` HELP text. | Operators see `nebula_webhook_signature_failures_total` or refresh coordinator metrics with generic HELP, while constants claim richer semantics. Dashboards drift. | Refactor: centralize descriptors in one catalog and generate re-exports/export HELP from it. |
| M-007 | High | Metric Semantics | Workflow metrics collapse terminal outcomes. `emit_final_event` increments only completed and failed; cancelled and timed out executions only affect duration. | Shutdown cancellation or wall-clock timeout does not produce an outcome-specific counter, so alerts cannot distinguish user failure from cancellation or timeout. | API correction: replace separate completed/failed counters with `executions_completed_total{status,reason_class}` or add bounded counters for all terminal states. |
| M-008 | High | Metric Semantics | Action failures count `RuntimeError` paths, not every operator-meaningful failed action outcome. | `ActionResult::Terminate(Failure)` is an `Ok(ActionResult)` at runtime, so action failure metrics may not reflect workflow-level explicit failure. | API correction: record action outcome after interpreting `ActionResult`, with bounded `outcome` / `failure_class` labels. |
| M-009 | High | Label Model | The allowlist is global, not per metric, with no label count limit, value length limit, value validation, or strip diagnostic. | A globally allowed key like `status` can be attached to any metric with arbitrary values, or stripped labels silently collapse all series into one aggregate. | Architecture correction: per-metric label schemas plus dev-mode rejection and production diagnostic counter. |
| M-010 | High | Boundary | Production call sites use `nebula_telemetry::metrics::MetricsRegistry` directly despite the rule that metrics instrumentation goes through `nebula-metrics`. | Future call sites bypass the adapter and never see naming or label policy, while tests still pass because raw registry accepts any name. | Architecture correction: pass a `nebula_metrics` registry wrapper or domain adapter through engine/api/resource composition. |
| M-011 | High | Prometheus Export | `snapshot() -> String` is infallible and sanitizes invalid names/labels instead of surfacing invalid metric families. | A bad metric name is silently renamed in scrape output; dashboards use the sanitized name while the code records a different raw name. | API correction: add `ScrapeSnapshot` / `ExportError` or diagnostic invalid-family metrics; reject invalid catalog names at registration. |
| M-012 | High | Operator Usefulness | Resource, scheduler, queue, API RED, retry, fallback, circuit breaker, and saturation metrics are incomplete or constants-only. | Queue backlog grows, circuit breaker rejects calls, or resource pool saturates, but `/metrics` lacks the series needed to alert without logs. | Refactor/API correction: catalog must include required RED/USE metrics and wire them through domain adapters. |

## Architecture Risks

### M-001: Safety Is Optional, Not Encoded

The crate states that it provides naming and label safety (`crates/metrics/src/lib.rs:10-14`, `crates/metrics/README.md:14-21`). The public API weakens that boundary:

- `TelemetryAdapter::new` configures `LabelAllowlist::all()` by default (`crates/metrics/src/adapter.rs:34-40`).
- `TelemetryAdapter::registry()` exposes `&MetricsRegistry` for custom or legacy names (`crates/metrics/src/adapter.rs:77-81`).
- Generic `counter`, `gauge`, and `histogram` accept any `&str` (`crates/metrics/src/adapter.rs:133-151`).
- `nebula_metrics::MetricsRegistry` is re-exported from the crate root (`crates/metrics/src/lib.rs:75-76`) and prelude (`crates/metrics/src/prelude.rs:7-15`).
- `MetricsRegistry` itself accepts any name and labels (`crates/telemetry/src/metrics.rs:430-478`).

This means the safe path is one option among many, not the contract. The rule file says "`metrics` instrumentation goes through `nebula-metrics`" (`.ai-factory/rules/base.md:60-64`), but engine, api, and resource production code often import `nebula_telemetry::metrics::MetricsRegistry` directly (`crates/engine/src/engine.rs:47`, `crates/engine/src/runtime/runtime.rs:22`, `crates/api/src/services/webhook/transport.rs:39`, `crates/resource/src/metrics.rs:18`).

Classification: Architecture correction.

### M-002: Filtering After Interning Does Not Protect Memory

The allowlist strips keys from a `LabelSet` (`crates/metrics/src/filter.rs:95-101`). But `LabelSet` construction interns every key and value first (`crates/telemetry/src/labels.rs:196-200`). The registry interner is append-only (`crates/telemetry/src/labels.rs:108-114`).

The examples demonstrate the unsafe sequence:

- Build `raw_labels` using the registry interner, including `execution_id` and `workflow_id` (`crates/metrics/examples/cardinality_guard.rs:33-40`).
- Filter later with `adapter.filter_labels(&raw_labels)` (`crates/metrics/examples/cardinality_guard.rs:47-55`).

That prevents high-cardinality series keys, but it does not prevent high-cardinality strings from entering the registry interner. Sensitive or unbounded values can remain resident even when not exported.

Classification: API correction.

### M-003: The Metric Catalog Is Documentation, Not Enforcement

`naming.rs` defines many constants, but nothing requires call sites to use them. Tests in `crates/metrics/src/export/prometheus.rs` use raw strings for canonical metrics (`crates/metrics/src/export/prometheus.rs:394-400`, `458-459`, `489-492`). The telemetry example uses non-catalog names such as `nebula_executions_total`, `nebula_active_workers`, and `nebula_active_actions` (`crates/telemetry/examples/basic_metrics.rs:23-35`, `70`).

Several catalog constants are not re-exported from `nebula_metrics` root:

- `NEBULA_RESOURCE_CIRCUIT_BREAKER_OPENED_TOTAL`
- `NEBULA_RESOURCE_CIRCUIT_BREAKER_CLOSED_TOTAL`
- `NEBULA_RESOURCE_CREDENTIAL_ROTATION_SKIPPED_TOTAL`

Several catalog constants are also absent from Prometheus descriptor matching and therefore export generic HELP:

- All credential refresh coordinator metrics
- `NEBULA_CREDENTIAL_RESOLVER_REAUTH_PERSIST_CAS_EXHAUSTED_TOTAL`
- `NEBULA_WEBHOOK_SIGNATURE_FAILURES_TOTAL`
- Resource circuit breaker and rotation skipped metrics

Evidence: root re-export list in `crates/metrics/src/lib.rs:45-74`; exporter descriptor imports and help match arms in `crates/metrics/src/export/prometheus.rs:20-121`; naming constants in `crates/metrics/src/naming.rs:11-425`.

Classification: Refactor.

### M-004: Private Registries Can Make Metrics Disappear

Some components default to private registries:

- `ControlConsumer` defaults to `MetricsRegistry::new()` (`crates/engine/src/control_consumer.rs:213-219`, `237-244`) and documents that production must inject the shared registry (`crates/engine/src/control_consumer.rs:286-294`).
- `WebhookTransport::new` runs without metrics; only `with_metrics` records signature failures (`crates/api/src/services/webhook/transport.rs:112-116`, `143-153`, `527-532`).
- `AppState` has `metrics_registry: Option<Arc<MetricsRegistry>>`; `/metrics` returns 503 if absent (`crates/api/src/state.rs:89-91`, `crates/api/src/routes/metrics.rs:9-24`).

This may be acceptable for tests, but for a stable observability boundary the production composition contract needs a fail-fast path when shared metrics are not wired.

Classification: Architecture correction.

## Metric Naming Review

### Names That Follow Prometheus Expectations

Most counter constants use the `nebula_*` prefix and `_total` suffix. Duration histograms use `_seconds`, for example:

- `NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS` (`crates/metrics/src/naming.rs:21-23`)
- `NEBULA_ACTION_DURATION_SECONDS` (`crates/metrics/src/naming.rs:86-87`)
- `NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS` (`crates/metrics/src/naming.rs:157-159`)
- `NEBULA_CREDENTIAL_REFRESH_COORD_HOLD_DURATION_SECONDS` (`crates/metrics/src/naming.rs:385-394`)

Label value modules for several metrics are closed and documented, such as dispatch rejection reasons (`crates/metrics/src/naming.rs:98-110`), webhook signature failure reasons (`crates/metrics/src/naming.rs:128-147`), and refresh coordinator outcomes (`crates/metrics/src/naming.rs:302-383`).

### Naming Drift and Misleading Names

Issues:

- `NEBULA_CREDENTIAL_ACTIVE_TOTAL` is documented as a gauge for the number of active credentials (`crates/metrics/src/naming.rs:275-276`) but ends in `_total`, which Prometheus users read as counter semantics.
- `NEBULA_EVENTBUS_SENT` and `NEBULA_EVENTBUS_DROPPED` are exported as gauges (`crates/metrics/src/adapter.rs:225-235`, `crates/metrics/src/export/prometheus.rs:94-97`) but source stats are cumulative counts (`crates/eventbus/src/stats.rs:24-31`). They are not named `_total`.
- Cache hits, misses, and evictions are gauge snapshots (`crates/metrics/src/naming.rs:418-425`, `crates/metrics/src/export/prometheus.rs:99-102`) but event-count names without `_total` will be surprising in PromQL.
- `NEBULA_ACTION_FAILURES_TOTAL` is too broad for its actual runtime semantics. `observe_dispatched` increments it only when the Rust `Result` is `Err` (`crates/engine/src/runtime/runtime.rs:769-779`), while action flow-control failures can be represented inside `Ok(ActionResult)`.

### Constants Without Emission

Many constants are currently unused outside the metrics crate and tests. Examples include resource wait duration, resource usage duration, pool exhausted, pool waiters, circuit breaker opened/closed, credential rotation metrics, cache metrics, and credential active/expired. Resource production wiring records only acquire/acquire_error/release/create/destroy counters (`crates/resource/src/metrics.rs:57-64`).

Unused constants are not inherently wrong, but a stable metric catalog should distinguish "defined and emitted" from "reserved/planned". Otherwise dashboards may depend on metrics that never appear.

Classification: Documentation/test only for unused reserved names; API correction where names encode wrong semantics.

## Label Cardinality Review

### Current Behavior

`LabelAllowlist` is a global key allowlist:

- `all()` passes every label unchanged (`crates/metrics/src/filter.rs:56-65`).
- `only()` accepts arbitrary key strings (`crates/metrics/src/filter.rs:67-85`).
- `apply()` returns a filtered `LabelSet` and silently ignores unknown keys (`crates/metrics/src/filter.rs:87-101`).

The adapter applies the allowlist only on a subset of labeled canonical action and workflow accessors (`crates/metrics/src/adapter.rs:171-215`). It does not apply to:

- Unlabeled canonical accessors
- Generic `counter/gauge/histogram`
- `registry()`
- Direct `MetricsRegistry` imports
- Production call sites that use raw `MetricsRegistry`

### Cardinality Hazards

- No per-metric label schema. A key allowed for one metric is allowed everywhere.
- No maximum label count per sample.
- No maximum label value length.
- No safe value vocabulary for bounded labels like `status`, `outcome`, or `reason`.
- No rejection path for dangerous labels in development.
- No diagnostic counter or log for stripped labels.
- Filtering silently collapses dimensions. If `execution_id` and `workflow_id` are stripped and no safe labels remain, all events record into one unlabeled series.
- The allowlist does not stop interner growth, as covered in M-002.

Dangerous labels that the current API allows: `execution_id`, `workflow_id`, `node_id`, `user_id`, `tenant_id`, `request_id`, `trace_id`, `span_id`, raw URL, query, path, email, error message, plugin instance, resource key, credential ID, host name, and arbitrary plugin-provided labels.

### Required Design Change

`LabelAllowlist` can remain as a compatibility helper, but the production boundary should be a per-metric `LabelSchema`:

- Static allowed keys per metric.
- Static value enums where possible.
- Bounded `LabelValue` for controlled dynamic values.
- Development-mode rejection for unknown labels.
- Production-mode drop with `nebula_metrics_labels_rejected_total{metric,label_key,reason}` using a bounded label key vocabulary.

Classification: Architecture correction.

## Prometheus Export Review

### Correct Pieces

- Content type is valid for Prometheus text exposition: `text/plain; version=0.0.4; charset=utf-8` (`crates/metrics/src/export/prometheus.rs:42-43`, `348-352`).
- HELP and TYPE lines are emitted before samples within each family (`crates/metrics/src/export/prometheus.rs:265-270`, `288-293`, `311-340`).
- Label values escape backslash, quote, newline, carriage return, and tab (`crates/metrics/src/export/prometheus.rs:216-229`).
- Histogram export includes finite buckets, `+Inf`, `_sum`, and `_count` in normal cases (`crates/metrics/src/export/prometheus.rs:319-340`).

### Exporter Correctness Gaps

1. Same raw name can have conflicting types.

`MetricsRegistry` has separate maps for counters, gauges, and histograms (`crates/telemetry/src/metrics.rs:399-405`). Nothing prevents:

```rust
registry.counter("nebula_x_total").inc();
registry.gauge("nebula_x_total").set(1);
```

`snapshot()` would emit both counter and gauge families with the same exported name because `allocate_exported_metric_name` returns the existing mapping for the same raw name (`crates/metrics/src/export/prometheus.rs:194-214`), and counter/gauge/histogram loops are independent (`crates/metrics/src/export/prometheus.rs:250-343`).

2. Cross-series label-key sanitization can create duplicate series.

`render_labels` disambiguates collisions only inside a single label set using a per-sample `used_keys` set (`crates/metrics/src/export/prometheus.rs:132-150`). Two separate label sets with raw keys `a-b` and `a b` both render as `a_b` if each sample has only one of the keys. That creates duplicate samples with the same exported labels.

3. Exporter mutates invalid names instead of rejecting them.

`sanitize_metric_name` and `sanitize_label_key` rewrite invalid identifiers (`crates/metrics/src/export/prometheus.rs:158-182`). Sanitizing helps keep scrape output parseable, but it hides bugs and changes the public metric contract. For stable metrics, invalid catalog names should be impossible at registration.

4. Descriptor catalog is incomplete.

The exporter help functions know only a subset of `naming.rs`. Known production metrics such as webhook signature failures (`crates/api/src/services/webhook/transport.rs:527-532`) and credential refresh coordinator metrics (`crates/engine/src/credential/refresh/metrics.rs:68-105`) export as custom metrics because the descriptor functions do not include them.

5. Output ordering is only partially deterministic.

Families are grouped by `BTreeMap` (`crates/metrics/src/export/prometheus.rs:252`, `275`, `298`), but entries within a family come from `DashMap::iter()` snapshots (`crates/telemetry/src/metrics.rs:518-538`) and are not sorted before formatting. Tests that compare full output would be flaky; scrape output can reorder series across runs.

6. `snapshot() -> String` cannot report invalid families.

The exporter ignores all `fmt::Write` results (`crates/metrics/src/export/prometheus.rs:266-340`) and returns a plain `String`. There is no way to distinguish a clean scrape from one containing renamed metrics, duplicate series, or type conflicts.

Classification: API correction.

## Histogram Correctness Review

### Normal Semantics

The telemetry histogram validates custom boundaries as non-empty, positive, finite, sorted, and duplicate-free (`crates/telemetry/src/metrics.rs:178-199`). Bucket counts are cumulative when read through `buckets()` (`crates/telemetry/src/metrics.rs:260-277`). The exporter emits finite buckets, `+Inf`, `_sum`, and `_count` (`crates/metrics/src/export/prometheus.rs:319-340`).

### Critical Concurrency Risk

`Histogram::observe` performs three independent relaxed updates:

- finite bucket count (`crates/telemetry/src/metrics.rs:235`)
- `total_count` (`crates/telemetry/src/metrics.rs:236`)
- `sum_bits` (`crates/telemetry/src/metrics.rs:240-245`)

The exporter reads:

- `count = hist.count()` (`crates/metrics/src/export/prometheus.rs:315`)
- `sum = hist.sum()` (`crates/metrics/src/export/prometheus.rs:316`)
- `buckets = hist.buckets()` (`crates/metrics/src/export/prometheus.rs:317`)

Because the finite bucket update happens before `total_count`, a scrape can see a finite bucket include an observation while `_count` and `+Inf` do not. That violates the Prometheus invariant that `le="+Inf"` equals `_count` and is greater than or equal to every finite bucket.

### Other Histogram Risks

- Negative observations are allowed. With positive duration buckets, a negative duration lands in the first bucket and can make `_sum` negative. Duration histograms should reject negative values at the metrics boundary even if telemetry primitives allow generic histograms.
- Very large finite observations can eventually make `_sum` infinite; exporter will format Rust's float display without a policy for Prometheus special values.
- Empty histograms are not registered/exported until the histogram is created. If a dashboard expects zero-valued buckets for a metric family, it will not appear unless a caller touches the histogram handle.
- Custom bucket conflicts are detected only for the exact `(name, labels)` series in `histogram_with_buckets_labeled` (`crates/telemetry/src/metrics.rs:480-510`). The same metric name can have different bucket layouts across label sets, producing one Prometheus histogram family with incompatible bucket schemas.

Classification: Refactor/API correction.

## Snapshot Consistency and Concurrency Review

`snapshot()` takes three independent registry snapshots: counters, gauges, and histograms (`crates/metrics/src/export/prometheus.rs:250-343`). Each registry snapshot clones current map entries from a `DashMap` (`crates/telemetry/src/metrics.rs:518-538`), then the exporter reads metric atomics while formatting.

Acceptable behavior:

- A scrape is allowed to observe values that change between scrapes.
- Map locks are not held while formatting, so recording paths are not blocked for the whole scrape.
- Counter and gauge values are single atomic loads and are therefore individually coherent.

Unsafe behavior:

- A scrape is not a single registry transaction. A metric registered between the counter and gauge phases can appear in only one phase of that scrape.
- Histogram values are not internally coherent, as covered in M-004.
- Multiple concurrent scrapes each allocate their own output string and family maps.
- Series order within a family is not stable because the registry uses `DashMap::iter()`.
- No duplicate-family or duplicate-series validation runs after the three snapshots are combined.

The production requirement should not be "every value is from the same nanosecond." It should be "every exported metric family is structurally valid and self-consistent." The current code does not guarantee that for histograms or conflicting families.

Classification: Refactor/API correction.

## Adapter Review

`TelemetryAdapter` is currently a convenience wrapper, not a correctness boundary.

Strengths:

- It uses naming constants for canonical workflow/action/eventbus accessors (`crates/metrics/src/adapter.rs:85-130`, `223-264`).
- Labeled workflow/action accessors call `filter_labels` (`crates/metrics/src/adapter.rs:171-215`).
- It provides eventbus snapshot recording with clamped integer values (`crates/metrics/src/adapter.rs:249-264`).

Weaknesses:

- Default allowlist is passthrough (`crates/metrics/src/adapter.rs:37-40`).
- Generic accessors accept arbitrary names (`crates/metrics/src/adapter.rs:133-151`).
- `registry()` exposes raw recording (`crates/metrics/src/adapter.rs:77-81`).
- No domain-specific resource, scheduler, API, retry, fallback, circuit breaker, or queue methods.
- No per-metric label schema.
- No methods return diagnostic information about stripped labels.
- No fake/trait boundary for tests that assert "which metric was recorded" without reading raw registry internals.

Recommended direction: domain adapters should record semantic events, not raw metric handles. For example, `workflow_finished(status, reason_class, elapsed)`, `action_finished(outcome, failure_class, elapsed)`, `queue_backlog(queue, depth)`, and `resource_acquire_finished(outcome, wait_seconds)`.

Classification: API correction.

## nebula-telemetry Boundary Review

The intended boundary is clear in `crates/telemetry/README.md`: telemetry owns primitives; naming, adapters, and export live in metrics (`crates/telemetry/README.md:14-19`, `37-45`). The implementation blurs the boundary:

- `nebula-metrics` re-exports telemetry primitives and registry (`crates/metrics/src/lib.rs:75-76`, `crates/metrics/src/prelude.rs:7-15`).
- Production crates depend on both `nebula-metrics` and `nebula-telemetry` (`crates/engine/Cargo.toml`, `crates/api/Cargo.toml`, `crates/resource/Cargo.toml`) and often import telemetry directly.
- `nebula-telemetry` docs and examples still show raw `nebula_*` names (`crates/telemetry/src/metrics.rs:372-397`, `crates/telemetry/examples/basic_metrics.rs:23-70`), which encourages naming outside the catalog.

Classification:

- Acceptable layering: `nebula-telemetry` keeping primitive maps and atomics.
- API misuse risk: re-exporting `MetricsRegistry`, `Counter`, `Gauge`, `Histogram` from `nebula-metrics`.
- Architecture correction required: production-facing code should receive a metrics wrapper/catalog, not raw telemetry registry.

## Operator Usefulness Review

Current metrics can answer a few high-level questions:

- Are workflow executions starting? `nebula_workflow_executions_started_total`
- Are workflows completing or failing? `nebula_workflow_executions_completed_total`, `nebula_workflow_executions_failed_total`
- What is workflow/action duration? duration histograms
- Are action dispatches rejected before handler execution? `nebula_action_dispatch_rejected_total{reason}`
- Are webhook signatures failing? `nebula_webhook_signature_failures_total{reason}`
- Are credential refresh coordinator claims/coalesces/sentinels happening? refresh coordinator metrics

Current metrics cannot reliably answer:

- Are executions timing out versus being cancelled?
- Are failures caused by user action errors, engine errors, data limits, panic, lease loss, or storage CAS?
- Are retries storming?
- Did fallback hide primary failures?
- Is a circuit breaker open or rejecting calls? Constants exist, but production emission was not found.
- Is the scheduler throttling due to pressure?
- Is queue backlog growing?
- Is a resource pool saturated or exhausted? Constants exist, but current resource production metrics only include five operation counters.
- Are API endpoints healthy by route/status/method? No API RED metrics are present in the reviewed route layer.
- Are plugin-provided labels safe? The current API permits arbitrary labels.

Classification: Refactor/API correction.

## Security and Privacy Review

Metrics labels are exported to a public-ish operator surface (`crates/api/src/routes/metrics.rs:31-33` states the metrics router is unauthenticated). The current metrics boundary does not prevent sensitive labels:

- Label values can be arbitrary strings.
- No denylist catches emails, tokens, URLs, query strings, file paths, error messages, tenant IDs, trace IDs, or execution IDs.
- Unsafe labels filtered by `LabelAllowlist` are still interned first, so sensitive values may remain in process memory.
- Plugins or integrations can cause cardinality DoS by recording through re-exported `MetricsRegistry`.
- Label value escaping prevents text-format injection, but it does not prevent sensitive data export.

The crate should treat labels as an exfiltration boundary. User/plugin-controlled values belong in traces, logs with redaction, or events, not metric labels.

Classification: Architecture correction.

## Performance Review

High-impact issues:

- High-cardinality values can grow the interner even when later filtered. This affects memory directly and is not solved by Prometheus scrape behavior.
- `retain_recent(&mut self)` can compact the registry interner (`crates/telemetry/src/metrics.rs:575-583`, `603-642`), but production state stores `Arc<MetricsRegistry>` (`crates/api/src/state.rs:89-91`), making routine compaction hard to apply.
- Snapshot allocates a full `String` plus family maps and entry vectors every scrape (`crates/metrics/src/export/prometheus.rs:241-345`). This is acceptable for small catalogs but risky under cardinality attack or many concurrent scrapes.
- Snapshot clones metric handles from all three `DashMap`s (`crates/telemetry/src/metrics.rs:518-538`), then reads atomics while formatting. This avoids holding map locks for formatting, but histogram consistency is not guaranteed.
- `LabelAllowlist::apply` allocates a `Vec<&str>` on each filtered call (`crates/metrics/src/filter.rs:98-100`), and `filter_label_set` interns allowed keys (`crates/telemetry/src/labels.rs:249-255`). This is secondary to the cardinality design issue.

Do not optimize formatting before fixing label safety and family invariants.

Classification: Refactor after API correction.

## Documentation and Contract Review

The documentation is honest about some gaps but still overstates the safety contract in ways that can cause production misuse:

- `crates/metrics/README.md:43-44` says the naming constants are normative and `LabelAllowlist` is the designated guard, but the API still allows raw names and direct registry writes.
- `crates/metrics/README.md:57-59` marks the API stable while also noting manual naming enforcement and possible drift. Stable should mean the correct contract is encoded, not merely that symbols are unlikely to move.
- `crates/metrics/src/lib.rs:10-14` says the crate strips high-cardinality labels before they reach the registry, but filtering an already-interned `LabelSet` lets high-cardinality values enter the registry interner first.
- `crates/metrics/examples/cardinality_guard.rs:71-84` demonstrates `retain_recent` as a dynamic guard, but production usually holds `Arc<MetricsRegistry>` and `retain_recent` requires `&mut self`.
- `crates/telemetry/README.md:37-45` correctly says telemetry does not own naming/export policy, but telemetry examples still teach raw `nebula_*` strings.

Missing contract docs that are high severity because they affect incidents:

- Required per-metric label schemas and forbidden labels.
- Whether unknown labels are stripped, rejected, logged, or counted.
- Whether unsafe label values are interned before filtering.
- How to add a stable metric without descriptor drift.
- Histogram concurrent scrape guarantees.
- Whether `snapshot()` can fail or can silently sanitize names.
- Which metrics are emitted today versus reserved/planned.
- Operator guidance for workflow/action/resource/API dashboards and alerts.

Classification: Documentation/test only for wording that matches current behavior; API correction where docs promise safety the API does not enforce.

## Missing Invariants

| Invariant | Currently encoded in types? | Currently tested? | Risk |
|-----------|-----------------------------|-------------------|------|
| Every operator-facing metric name must be defined in the naming module. | No. Raw `&str` is accepted everywhere. | No. | Catalog drift and ad-hoc dashboards. |
| Every metric name must use the `nebula_*` prefix. | No. Exporter sanitizes arbitrary names. | Partially for resource/credential/cache constants. | Invalid or colliding public names. |
| Every counter metric must have counter semantics and `_total` where appropriate. | No. | Partially by tests for selected constants. | Gauges named like counters and counters exported as gauges. |
| Units must be encoded in metric names where applicable. | No descriptor type. | Partially by duration constants. | Seconds/milliseconds/bytes confusion. |
| A metric name must not be used with multiple incompatible metric types. | No. Separate registry maps permit it. | No. | Invalid Prometheus output and broken alerts. |
| A metric name must not be used with multiple incompatible label schemas. | No. | No. | Duplicate or semantically mixed series. |
| High-cardinality labels must not reach the registry. | No. Filtering happens after interning; raw registry bypass exists. | Only proves series key stripping in one adapter test. | Memory blowup and TSDB cardinality explosion. |
| Unsupported/stripped labels must not be silently mistaken for successful dimensional recording. | No. | No. | Operators think dimensions exist but data collapses. |
| Prometheus output must be valid for all legal label values. | Partially. Values are escaped. | Partially. Quotes/backslash tested; newline unicode not fully. | Scrape parse failures or duplicate series. |
| Histogram buckets must be cumulative and include `+Inf`, `_sum`, and `_count`. | Partially in telemetry/exporter. | Normal-case tests only. | Broken quantiles and scrape rejection under concurrency. |
| `snapshot()` must not expose partial/corrupt metric families under concurrent recording. | No. | No. | Invalid histograms under load. |
| Sensitive values must never appear in labels. | No. | No. | PII/secret leakage into Prometheus. |

## Real Nebula Scenarios

| Scenario | Expected Metric Behavior | Current Behavior | What Could Go Wrong | Proving Test |
|----------|--------------------------|------------------|---------------------|--------------|
| 1. Workflow starts, succeeds, records duration. | Increment started, increment completed with `status=completed`, observe duration seconds. | Started/completed counters and duration are emitted (`crates/engine/src/engine.rs:1083-1086`, `3391-3406`). | No status label on duration, no workflow kind. | End-to-end workflow success asserts counter, duration count, and status labels. |
| 2. Workflow fails because user action returned error. | Increment terminal counter with `status=failed`, `failure_class=user_action`. | Failed counter increments if final status is `Failed` (`crates/engine/src/engine.rs:3396-3399`). | Cannot distinguish user action failure from engine/storage failure. | Workflow with action error asserts bounded failure_class metric. |
| 3. Workflow cancelled by engine shutdown. | Increment terminal counter with `status=cancelled`; duration still observed. | `emit_final_event` ignores `Cancelled` except duration (`crates/engine/src/engine.rs:3390-3406`). | Cancellation invisible except missing success/failure delta. | Cancel test asserts `status=cancelled` counter. |
| 4. Workflow times out. | Increment `status=timed_out` and timeout reason. | `TimedOut` is ignored by final metric match. | Timeouts look like neither failures nor cancellations. | Budget timeout test asserts timed_out counter and duration. |
| 5. Action retried 5 times then succeeds. | Count attempts, retries, final success separately. | Action executions count dispatched attempts, but no retry metric was found in metrics boundary. | Retry storms can look like normal high traffic. | Retry workflow asserts attempts_total and retries_total. |
| 6. Action retried then fails permanently. | Count attempts, retry exhaustion, final failure class. | `NEBULA_ACTION_FAILURES_TOTAL` increments on runtime `Err`; workflow failed increments final failure. | Cannot distinguish final failure from transient attempts. | Retry exhaustion test asserts retry_exhausted metric. |
| 7. Fallback succeeds after primary failure. | Count primary failure and fallback success; expose fallback path. | No fallback metric in current catalog or adapter. | Primary outage hidden by successful workflow. | Fallback test asserts primary_failed_total and fallback_success_total. |
| 8. Circuit breaker rejects before execution. | Increment reject counter with `reason=circuit_open`; do not observe action duration. | Resource circuit breaker constants exist (`crates/metrics/src/naming.rs:188-193`) but emission was not found. | Saturation/rejection invisible in metrics. | Breaker-open test asserts reject counter. |
| 9. Scheduler refuses work due to pressure. | Increment scheduler rejected/throttled with bounded reason; gauge pressure. | No scheduler metrics were found. | Operators cannot see admission control. | Pressure fake test asserts scheduler_throttled_total. |
| 10. Queue backlog grows. | Gauge queue depth/backlog by controlled queue name. | No queue backlog metric in metrics crate. | Backlog is invisible without storage inspection. | Control queue test asserts depth gauge. |
| 11. Resource pool exhausted. | Increment pool_exhausted_total and set waiters. | Constants exist (`crates/metrics/src/naming.rs:174-177`) but current resource metrics do not wire them (`crates/resource/src/metrics.rs:57-64`). | Pool saturation not alertable. | Pool exhaustion test asserts counter/gauge. |
| 12. API endpoint returns many 500s. | HTTP RED metrics by method, endpoint_template, status_class. | No API RED metric call sites found; `/metrics` only exports registry. | API incidents require logs. | Route integration test asserts request counter/duration. |
| 13. Plugin emits dynamic labels. | Reject or sandbox labels before interning; diagnostic counter. | Re-exported `MetricsRegistry` allows arbitrary labels. | Plugin cardinality DoS or PII export. | Simulated plugin emits 10k labels; assert bounded registry/interner. |
| 14. User creates 10,000 workflows with unique names. | Workflow name must not be a label; workflow kind only if controlled. | No workflow label schema prevents it. | TSDB cardinality explosion. | Property test attempts workflow_name label and expects rejection. |
| 15. Execution IDs accidentally added as labels. | Reject before interning; diagnostic. | Adapter test strips from series (`crates/metrics/src/adapter.rs:410-435`) but values are already interned. | Memory grows, caller thinks label recorded or safely rejected. | Assert interner_len does not grow with rejected execution IDs. |
| 16. Prometheus scrapes during heavy metric updates. | Snapshot remains valid; histograms are self-consistent. | Histogram count/sum/buckets are read independently. | Invalid histogram bucket order or count mismatch. | Stress scrape while observing and parse monotonic histograms. |
| 17. Histogram has no observations. | If registered, export zero buckets/count/sum consistently or document absence. | Empty registry exports nothing; touched histogram without observations may be possible only by retrieving handle, but tests only observe first (`crates/metrics/src/export/prometheus.rs:441-452`). | Dashboards see missing series instead of zero. | Register histogram without observe and assert expected output policy. |
| 18. Observation on bucket boundary. | `<= le` bucket includes boundary. | `binary_search_by` should place equal values in exact bucket (`crates/telemetry/src/metrics.rs:225-233`). | Needs exporter-level proof. | Boundary observations assert cumulative bucket counts. |
| 19. Label value contains quote, slash, newline, unicode. | Escape quote/backslash/newline; preserve unicode; slash is plain value. | Quote/backslash escaped; newline escaped (`crates/metrics/src/export/prometheus.rs:216-229`). | Unicode not explicitly tested; sensitive values still possible. | Parse output with unicode/newline values. |
| 20. Same metric name with different label keys. | Either schema rejects or catalog defines optional keys consistently. | Raw registry accepts all label sets. | Queries aggregate inconsistent dimensions or duplicate sanitized labels. | Same-name different schema test expects rejection. |

## API Misuse Cases

| Misuse | Current API Allows It | Production Failure | Prevention |
|--------|-----------------------|--------------------|------------|
| Record arbitrary metric names. | `MetricsRegistry::counter(name)` and adapter generic accessors. | Catalog drift and invalid names. | `MetricName`/descriptor registry; no raw `&str` in safe API. |
| Bypass naming constants. | Raw strings accepted and examples use them. | Dashboards depend on unstable names. | Generated catalog accessors only. |
| Bypass `LabelAllowlist`. | Direct registry and `adapter.registry()`. | Cardinality explosion. | Hide raw registry behind explicit primitive crate import; lint production crates. |
| Use `execution_id` as a label. | Any label key/value accepted. | One series per execution and interner growth. | Per-metric schema rejects before interning. |
| Use workflow name as label. | No value source restriction. | User-created workflow cardinality. | Use bounded workflow kind enum only. |
| Use raw URL/path/query as endpoint label. | No endpoint_template type. | PII/tokens/cardinality. | `EndpointTemplate` newtype, route instrumentation only. |
| Use error message as label. | No forbidden label key/value policy. | PII and unbounded values. | `error_class` enum, log detailed message separately. |
| Use gauge for counter. | Same string can be registered in gauge map. | Conflicting Prometheus type. | Descriptor enforces kind per name. |
| Use counter for current state. | No semantic type. | Misleading rates. | Descriptor kind/unit review tests. |
| Emit same metric with different label schemas. | Any `LabelSet` accepted per name. | Queries lie or duplicate series. | Label schema per metric. |
| Emit milliseconds into `_seconds`. | Histogram accepts any `f64`. | Latency dashboards off by 1000x. | Domain methods accept `Duration` and convert once. |
| Treat stripped labels as recorded. | `apply()` silently drops. | Dashboards miss desired dimensions. | Return `SafeLabels { labels, rejected }`; dev rejection. |
| Expose invalid Prometheus output. | Conflicting types and sanitized collisions. | Scrape failure or duplicate samples. | Validate registry before export; return `ExportError`. |
| Duplicate series through sanitized label keys. | Cross-series collision not tracked. | Prometheus duplicate sample. | Schema-level valid label keys; family-level collision detection. |
| Put trace data in metrics. | `trace_id` accepted. | Metrics become traces with unbounded cardinality. | Forbidden keys and trace-log redirection. |

## Recommended Test Plan

### P0: Must Add Before Relying On This Crate As Stable

- Same metric name cannot be registered as counter and gauge/histogram through the metrics boundary.
- Same metric name cannot be emitted with incompatible label key sets.
- Rejected label values are not interned; `interner_len` remains bounded when 10,000 `execution_id` values are rejected.
- `LabelAllowlist` / `SafeLabels` rejects dangerous keys in development mode and reports stripped labels in production mode.
- Prometheus export fails or reports diagnostics for duplicate metric families.
- Prometheus export detects cross-series sanitized label-key collisions.
- Concurrent histogram observe plus snapshot never emits finite buckets greater than `+Inf` or `_count`.
- All naming constants have descriptors: name, kind, help, unit, label schema.
- Exported descriptor catalog covers webhook and refresh coordinator metrics.
- Workflow terminal metrics cover completed, failed, cancelled, and timed_out.

### P1: Should Add Soon

- Label value escaping for newline, carriage return, tab, quote, backslash, slash, and unicode.
- Full-output deterministic ordering by metric name and label set.
- Histogram boundary observations at exact bucket limits.
- Empty registered histogram export policy.
- Negative duration observations rejected at metrics boundary.
- Custom bucket conflict by same metric name across label sets is rejected.
- Adapter records use naming constants, not raw strings.
- Production crates do not import `nebula_telemetry::metrics::MetricsRegistry` directly except approved composition roots.
- High-cardinality plugin attack simulation with bounded series and interner counts.

### P2: Nice To Have

- Fuzz Prometheus label rendering against a parser.
- Golden snapshots for representative metric families.
- Docs examples compile and avoid non-catalog names.
- Benchmarks for snapshot under 1k, 10k, and attack-cardinality series.
- Feature matrix tests when OTLP export is added.

## Recommended Refactor Plan

### Phase 1: Define Metric Catalog And Label Schemas

Create `MetricDescriptor` with `name`, `kind`, `unit`, `help`, `labels`, and stability status. Move all constants and HELP strings into that catalog. Mark metrics as emitted, reserved, or planned.

### Phase 2: Enforce Naming And Label Safety In API

Introduce `MetricName`, `LabelKey`, `LabelValue`, `SafeLabels`, and per-metric builders. Make domain adapter methods the normal API. Keep raw registry access only through `nebula-telemetry` or an explicitly named escape hatch.

### Phase 3: Harden Prometheus Exporter Correctness

Validate one type per family, one label schema per metric, valid identifiers, duplicate series detection, deterministic ordering, and complete descriptor coverage. Consider returning `Result<ScrapeSnapshot, ExportError>` and having `/metrics` decide response policy.

### Phase 4: Add Concurrency/Cardinality/Security Tests

Add stress tests for concurrent histogram snapshots, high-cardinality rejected labels, plugin label sandboxing, duplicate family detection, and sensitive-label denylist behavior.

### Phase 5: Improve Docs And Operator Guidance

Document how to add a metric, allowed labels, forbidden labels, units, counter/gauge/histogram choice, Prometheus behavior, and examples of correct/incorrect usage. Update telemetry examples so raw primitives do not teach catalog bypass for Nebula metrics.

### Phase 6: Prepare OTLP Compatibility Without Breaking Prometheus Semantics

Keep Prometheus names stable. Add descriptor fields for OTLP units and attributes, but do not let OTLP attribute flexibility weaken Prometheus cardinality rules. Treat metric catalog as the common source of truth.

## Proposed Canon Invariants

Since `canon-invariants` is currently empty in `crates/metrics/README.md:6`, these are candidate L2 invariants.

| Proposed Invariant | Why Nebula Needs It | How To Encode | How To Test |
|--------------------|---------------------|---------------|-------------|
| Every exported metric family is registered in a static `MetricCatalog`. | Prevents ad-hoc names and descriptor drift. | `MetricDescriptor` registry; no raw `&str` in safe API. | Catalog coverage test over all exported families. |
| A metric name has exactly one kind and unit. | Prevents Prometheus type conflicts and unit lies. | Descriptor keyed by `MetricName`. | Attempt counter+gauge same name and expect rejection. |
| Each metric has exactly one label schema. | Prevents incompatible labels and duplicate series. | `LabelSchema` per descriptor. | Same-name different labels test rejects. |
| Unsafe labels are rejected before interning values. | Prevents memory cardinality and PII retention. | `SafeLabels::build(raw_pairs, schema)` checks keys first. | 10k rejected execution IDs do not increase interner length. |
| Label values for bounded labels come from closed enums or validated bounded strings. | Prevents arbitrary status/error text. | `Outcome`, `Status`, `Reason` enums; `BoundedLabelValue`. | Compile/runtime tests for invalid values. |
| Stripped/rejected labels are observable through bounded diagnostics. | Prevents silent loss of dimensions. | `nebula_metrics_labels_rejected_total{metric,label_key,reason}`. | Unknown label increments diagnostic and does not record label. |
| Histograms export self-consistent atomic snapshots. | Prevents invalid quantiles and scrape rejection. | Telemetry histogram snapshot struct or seqlock. | Concurrent stress parses every scrape and checks monotonic buckets/count. |
| Prometheus exporter never silently renames catalog metrics. | Public metric names are stable API. | Validate descriptor names at startup; error on invalid. | Invalid descriptor fixture fails. |
| Sensitive values never enter metric labels. | Metrics are shared/stored long-term. | Forbidden key list plus schema review. | Table-driven denylist test for email, URL, token, path, trace_id. |
| `/metrics` exposes only a shared production registry. | Avoids invisible private metrics. | Composition root requires `MetricsRegistry` wrapper. | API/engine integration asserts shared registry receives emitted metrics. |

## GitHub Issues

### Issue M-001: Make label safety mandatory in nebula-metrics recording APIs

Severity: Critical

Body:

`nebula-metrics` is documented as the naming and label-safety layer, but callers can bypass it through `TelemetryAdapter::registry()`, generic `counter/gauge/histogram`, and the root/prelude `MetricsRegistry` re-export. `TelemetryAdapter::new` also defaults to `LabelAllowlist::all()`.

Failure scenario: plugin or engine code records `execution_id`, raw URL, or error message labels directly. Metrics export successfully, but Prometheus cardinality explodes and sensitive data may be exported.

Recommended fix: Architecture correction. Introduce a catalog-backed metrics wrapper and make raw registry access an explicit escape hatch outside the normal `nebula-metrics` prelude.

Acceptance tests:

- Production crates cannot use raw `MetricsRegistry` except approved composition roots.
- Unknown label keys are rejected or diagnostically dropped by default.
- Direct raw-name recording is absent from examples.

### Issue M-002: Reject unsafe labels before interning values

Severity: Critical

Body:

`LabelAllowlist` filters an already-built `LabelSet`. `LabelInterner::label_set` interns all key/value pairs before filtering, and the interner is append-only. This means stripped high-cardinality values still enter registry memory.

Failure scenario: 10,000 executions attach `execution_id`; the allowlist strips the key from exported series, but all 10,000 IDs remain interned.

Recommended fix: API correction. Replace `LabelAllowlist::apply(LabelSet)` for untrusted input with `SafeLabels::build(raw_pairs, schema, interner)`, checking keys before interning values.

Acceptance tests:

- Rejected execution IDs do not increase `interner_len`.
- Rejected sensitive labels do not appear in exported output.
- Diagnostic counter records rejected keys using bounded labels.

### Issue M-003: Enforce one metric kind per metric name

Severity: Critical

Body:

`MetricsRegistry` stores counters, gauges, and histograms in separate maps and accepts the same name in all three. `snapshot()` renders each type independently, so a single metric name can produce duplicate conflicting HELP/TYPE lines.

Failure scenario: one call site records `nebula_resource_pool_waiters` as a gauge and another records it as a counter. Prometheus scrape output is invalid or misleading.

Recommended fix: Architecture correction. Add descriptor registration that binds name to kind and rejects conflicts before recording/export.

Acceptance tests:

- Same name as counter+gauge is rejected.
- Same name as histogram+counter is rejected.
- Exporter cannot produce two TYPE lines for one metric family.

### Issue M-004: Make histogram snapshots atomic enough for Prometheus

Severity: Critical

Body:

`Histogram::observe` updates bucket, total count, and sum with separate relaxed atomics. `snapshot()` reads count, sum, and buckets independently. A scrape can observe finite buckets that include an observation while `+Inf` / `_count` do not.

Failure scenario: under load, Prometheus sees `bucket{le="5"} 100` and `bucket{le="+Inf"} 99`, breaking histogram monotonicity.

Recommended fix: Refactor/API correction. Add a telemetry histogram snapshot operation that returns count, sum, and cumulative buckets from a consistent read, using locking, seqlock, or another atomic snapshot design.

Acceptance tests:

- Stress test concurrent observe/snapshot and verify `+Inf == _count >= all finite buckets`.
- Validate `_sum` and `_count` are from the same snapshot epoch.

### Issue M-005: Detect sanitized label-key collisions across series

Severity: High

Body:

The Prometheus exporter disambiguates sanitized label-key collisions only within a single sample. Two separate series with keys `a-b` and `a b` both export as `a_b` if each sample has only one key.

Failure scenario: two raw series become identical after export, producing duplicate samples in one scrape.

Recommended fix: Refactor. Validate label keys at schema definition time, or allocate exported label keys per metric family and reject cross-series collisions.

Acceptance tests:

- `{a-b="x"}` and `{a b="x"}` for the same metric cannot export as duplicate series.
- Invalid label keys are rejected in catalog descriptors.

### Issue M-006: Generate Prometheus descriptors from the metric catalog

Severity: High

Body:

Several `naming.rs` constants are missing from top-level re-exports and exporter HELP match arms. Known production metrics such as webhook signature failures and credential refresh coordinator counters export as generic custom metrics.

Failure scenario: operators see `# HELP nebula_webhook_signature_failures_total Custom counter.` instead of the documented semantics and label contract.

Recommended fix: Refactor. Create a single metric catalog that owns constants, HELP text, type, unit, and label schema. Generate exporter descriptors and public re-exports from it.

Acceptance tests:

- Every `NEBULA_*` constant has a descriptor.
- Every descriptor is exported with non-generic HELP.
- Root re-exports cover all public stable constants or intentionally mark internal/reserved constants.

### Issue M-007: Add outcome-complete workflow execution metrics

Severity: High

Body:

Workflow final metrics currently increment only completed or failed counters. `Cancelled` and `TimedOut` statuses are ignored except for duration observation.

Failure scenario: engine shutdown cancels many executions; started count rises, duration rises, but no cancellation counter alerts operators.

Recommended fix: API correction. Record terminal executions with bounded `status` and `reason_class`, or add explicit counters for cancelled and timed out outcomes.

Acceptance tests:

- Completed, failed, cancelled, and timed_out executions each emit terminal metrics.
- Duration histogram can be queried by bounded status if that label is adopted.

### Issue M-008: Align action metrics with action outcome semantics

Severity: High

Body:

`NEBULA_ACTION_FAILURES_TOTAL` increments on `Result::Err`, but action flow-control failures can be represented in `Ok(ActionResult)`, such as explicit terminate failure. Current metrics can undercount operator-visible action failures.

Failure scenario: a Stop/Fail action terminates the workflow with failure, but action failure metrics remain flat.

Recommended fix: API correction. Record action outcomes after interpreting `ActionResult`, with bounded labels like `outcome`, `failure_class`, and `dispatch_stage`.

Acceptance tests:

- Runtime error, data limit rejection, explicit terminate failure, cancellation, and success produce distinct bounded outcomes.
- Rejection paths remain excluded from duration histograms.

### Issue M-009: Replace global allowlist with per-metric label schemas

Severity: High

Body:

`LabelAllowlist` is global. It allows keys without regard to metric semantics, accepts arbitrary values, and silently strips unsupported labels.

Failure scenario: `status` is globally allowed and a plugin emits thousands of unique status messages. Prometheus cardinality grows while all APIs appear to use an allowlist.

Recommended fix: Architecture correction. Define `LabelSchema` per metric, closed value enums for bounded labels, value length limits for controlled dynamic labels, and diagnostic rejection.

Acceptance tests:

- Unsafe key rejected for every metric.
- Allowed key on wrong metric rejected.
- Arbitrary status text rejected unless in closed set.

### Issue M-010: Route production instrumentation through nebula-metrics boundary

Severity: High

Body:

Production code frequently imports `nebula_telemetry::metrics::MetricsRegistry` directly. That bypasses the intended metrics boundary and makes `LabelAllowlist`/naming policy optional.

Failure scenario: new engine or API metric records a raw string and unsafe label set; tests pass because the primitive registry accepts it.

Recommended fix: Architecture correction. Inject a `nebula_metrics` wrapper/domain adapter into engine, api, and resource. Use lint or deny rule to restrict direct telemetry metrics imports.

Acceptance tests:

- Engine/api/resource production code no longer directly imports telemetry `MetricsRegistry` except composition roots.
- Domain adapter methods cover current metric call sites.

### Issue M-011: Stop silently sanitizing invalid stable metric names

Severity: High

Body:

The Prometheus exporter sanitizes metric names and label keys. This keeps text parseable, but for stable Nebula metrics it silently changes public API names and hides bugs.

Failure scenario: a typo or invalid character records `nebula action executions total`; scrape output becomes `nebula_action_executions_total`, colliding with a real metric or creating a name no source code owns.

Recommended fix: API correction. Validate catalog descriptors and safe label keys at registration. Reserve sanitization only for explicitly marked custom/legacy metrics with diagnostics.

Acceptance tests:

- Invalid catalog metric name fails registration.
- Invalid label key fails schema definition.
- Exporter reports custom sanitization diagnostics if legacy support remains.

### Issue M-012: Add operator-complete RED/USE metrics for workflow engine operation

Severity: High

Body:

Current metrics do not cover queue backlog, scheduler pressure, retries, fallback, API route RED, resource saturation, or circuit breaker rejection. Several constants exist but are not wired.

Failure scenario: resource pool exhaustion or retry storm causes an incident, but `/metrics` lacks the series needed to identify saturation without logs/traces.

Recommended fix: Refactor/API correction. Expand the metric catalog around operator scenarios and wire through domain adapters.

Acceptance tests:

- Queue backlog, resource pool exhaustion, scheduler throttle, retry attempts/exhaustion, fallback outcomes, circuit breaker state/rejections, and API RED metrics appear in integration tests.
- All labels are bounded by schema.

---

# Independent Re-pass (2026-05-05)

This section is a fresh audit pass against the codebase as of 2026-05-05. It (1) verifies which findings from the original 2026-04-17 audit still hold against current code, (2) records partial fixes that have landed since, and (3) adds new findings the first pass missed.

The original audit's structure and numbering are preserved verbatim above. New findings continue the M-### scheme starting at M-013.

## Status of Original Findings

| ID | Status | Evidence (current code) |
|----|--------|--------------------------|
| M-001 | **CONFIRMED** | `crates/metrics/src/adapter.rs:37-40` — `TelemetryAdapter::new` still defaults to `LabelAllowlist::all()`. `crates/metrics/src/adapter.rs:79-81` exposes `registry()`. `crates/metrics/src/adapter.rs:137-151` accepts arbitrary `&str`. `crates/metrics/src/lib.rs:76` re-exports `MetricsRegistry`/`Counter`/`Gauge`/`Histogram`. Engine runtime uses raw telemetry directly: `crates/engine/src/runtime/runtime.rs:22`. |
| M-002 | **CONFIRMED** | `crates/metrics/src/filter.rs:95-101` calls `interner.filter_label_set(...)`. `crates/telemetry/src/labels.rs:249-256` filters AFTER values were already interned in `label_set` (`crates/telemetry/src/labels.rs:196-218`). The interner is append-only by design (`crates/telemetry/src/labels.rs:108-114`). Additional foot-gun: `filter_label_set` itself calls `self.intern(k)` on every allowed key, so the filter path also writes into the interner — bounded by allowlist size, but worth noting. |
| M-003 | **CONFIRMED AND DEEPENED** | `crates/telemetry/src/metrics.rs:402-405` keeps three independent `DashMap`s. `crates/metrics/src/export/prometheus.rs:194-214` (`allocate_exported_metric_name`) shares `metric_raw_to_exported` and `taken` across the counter / gauge / histogram phases (`crates/metrics/src/export/prometheus.rs:247-310`). The shared map is intended to dedupe sanitized name collisions, but it has the side effect of **forcing the same raw name to map to the same exported name across types**. So if `"nebula_x_total"` is registered as both counter and gauge, the gauge phase's `allocate_exported_metric_name` finds the entry from the counter phase and reuses `"nebula_x_total"` — and then writes a second `# TYPE nebula_x_total gauge` line. The "fix for sanitized collisions" actively enables the multi-type duplication risk. |
| M-004 | **CONFIRMED** | `crates/telemetry/src/metrics.rs:220-246` — three independent Relaxed atomics: `counts[idx].fetch_add`, `total_count.fetch_add`, `sum_bits.update`. Exporter reads `count() / sum() / buckets()` independently (`crates/metrics/src/export/prometheus.rs:315-317`). Note: the existing test `histogram_concurrent_observe` (`crates/telemetry/src/metrics.rs:783-803`) only counts join-final values; it does not stress concurrent observe + snapshot. |
| M-005 | **PARTIALLY FIXED — within-sample only** | `crates/metrics/src/export/prometheus.rs:132-150` now disambiguates colliding sanitized label keys *inside one sample* using a per-sample `used_keys` set with a `__{hash:016x}` suffix. Tests `snapshot_disambiguates_sanitized_label_key_collisions` and `snapshot_disambiguates_sanitized_metric_name_collisions` cover this path. **Cross-series collision is still unfixed**: series A `{a-b="x"}` and series B `{a b="x"}` (same metric name) both render `a_b="x"` because the per-sample disambiguation never sees the other series' key. No test asserts cross-series uniqueness. |
| M-006 | **CONFIRMED** | `crates/metrics/src/export/prometheus.rs:47-121` — `counter_help`/`gauge_help`/`histogram_help` match arms still miss: `NEBULA_WEBHOOK_SIGNATURE_FAILURES_TOTAL`, all four refresh-coord counters (`_CLAIMS_TOTAL`, `_COALESCED_TOTAL`, `_SENTINEL_EVENTS_TOTAL`, `_RECLAIM_SWEEPS_TOTAL`), `NEBULA_CREDENTIAL_REFRESH_COORD_HOLD_DURATION_SECONDS`, `NEBULA_CREDENTIAL_RESOLVER_REAUTH_PERSIST_CAS_EXHAUSTED_TOTAL`, both circuit-breaker counters, and `NEBULA_RESOURCE_CREDENTIAL_ROTATION_SKIPPED_TOTAL`. They all export as `Custom counter.` / `Custom histogram.`. |
| M-007 | **CONFIRMED** | `crates/engine/src/engine.rs:3389-3402` — `match status` only handles `Completed` and `Failed`. `Cancelled`, `TimedOut`, and any future `ExecutionStatus` variants fall through to `_ => {}` and only the duration histogram is observed. |
| M-008 | **CONFIRMED** | `crates/engine/src/runtime/runtime.rs:769-780` — `observe_dispatched` increments failures only when the Rust `Result` is `Err`. `ActionResult::Terminate(Failure)` flows through as `Ok(...)` and is invisible to the failures counter. |
| M-009 | **CONFIRMED** | `crates/metrics/src/filter.rs:43-117` — `LabelAllowlist` is still a flat global key list with no per-metric schema, no value validation, no length limit, no diagnostic on strip. |
| M-010 | **CONFIRMED** | `crates/engine/src/runtime/runtime.rs:22` imports `nebula_telemetry::metrics::{Counter, Histogram, MetricsRegistry}` directly. `crates/resource/src/metrics.rs:18` does the same despite using naming constants from `nebula-metrics`. The boundary the README contract claims is not enforced. |
| M-011 | **CONFIRMED** | `crates/metrics/src/export/prometheus.rs:158-182` — `sanitize_metric_name` / `sanitize_label_key` still rewrite invalid identifiers silently; `snapshot()` returns plain `String`. |
| M-012 | **CONFIRMED** | No scheduler / queue / retry / fallback / circuit-breaker emission paths exist; `nebula_resource_circuit_breaker_*` constants still have zero call sites outside the constants module. |

## New Findings

### M-013: Critical — `nebula_resource_health_state` documents fractional values that the type system cannot represent

The naming docstring (`crates/metrics/src/naming.rs:172-173`) and the exported HELP line (`crates/metrics/src/export/prometheus.rs:90-92`) both promise:

> `Resource health state (1=healthy, 0.5=degraded, 0=unhealthy).`

But the underlying type is `Gauge` backed by `AtomicI64` (`crates/telemetry/src/metrics.rs:80-83`); `Gauge::set(v: i64)` (`crates/telemetry/src/metrics.rs:108-110`) cannot accept `0.5`. **There is no API path that produces the documented "degraded = 0.5" value.** An operator building a "degraded" alert based on the HELP text will never fire it, and a "healthy = 1" alert will silently treat degraded as healthy if a future patch starts emitting `1` for both states.

This is a documentation-vs-API contract violation, not a runtime bug — the metric is currently unwired (no production emission). It is critical because the lying contract is in the public docs and every operator who reads them ends up with a broken dashboard plan.

Failure scenario: a future engineer adds emission (`gauge.set(0)` for degraded because 0.5 won't compile) and the dashboard quietly conflates degraded with unhealthy.

Classification: API correction (gauge semantic) or Documentation/test only (drop the half-value tier from the docs and HELP).

### M-014: High — `_sum` can become Inf and break Prometheus parse

`Histogram::observe` filters non-finite *inputs* (`crates/telemetry/src/metrics.rs:220-223`) but does not bound the running sum. `f64` arithmetic on cumulative sums can overflow to `±Inf` for legitimately large finite observations (e.g., a `_seconds` metric used to measure month-long workflow durations, or a `_bytes` metric on a long-running process). Once `sum_bits` is `Inf`, every subsequent observation keeps it at `Inf` (`Inf + finite = Inf`).

The exporter writes `_sum` via Rust's `Display` for `f64` (`crates/metrics/src/export/prometheus.rs:333, 339`). Rust's `Display` renders `f64::INFINITY` as `inf` (lowercase). Prometheus text exposition expects `+Inf` / `-Inf` / `NaN` (capitalized). Strict scrapers reject lowercase `inf`; tolerant scrapers parse it as a different value or silently drop the sample.

Failure scenario: long-running workflow histogram accumulates a sum that overflows; scrape now emits `nebula_workflow_execution_duration_seconds_sum inf`; downstream Prometheus rejects the family and the entire histogram disappears from queries.

Classification: API correction. The metrics-boundary observe path should reject non-finite **outcomes** (after the addition), not just non-finite **inputs**, and should expose a saturation diagnostic counter.

### M-015: High — Negative observations silently distort `_seconds` histograms

`observe()` accepts negative finite values. With default buckets (all positive), `binary_search_by` returns insertion position 0, so the value lands in the lowest finite bucket. `_sum` becomes negative.

Production trigger: any caller that derives durations from `SystemTime::now() - then` (rather than `Instant::elapsed()`) can produce a negative `f64` if the wall clock steps backward (NTP correction, VM time-travel, container clock drift). The metrics boundary currently has no defense.

For `_seconds` semantics this is operationally wrong — a duration cannot be negative. Dashboards relying on `rate(_sum) / rate(_count)` for average latency see a wrong signal.

Classification: API correction. The domain methods should accept `Duration` (not `f64`), reject NaN/negative at the typed boundary, and route the rejection through a diagnostic counter so the discard is observable.

### M-016: High — Default histogram buckets are wrong for several `_seconds` metrics

`DEFAULT_BUCKETS = [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]` (`crates/telemetry/src/metrics.rs:133-135`) is designed for HTTP-request-like sub-10-second latencies. It is applied verbatim to:

- `NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS` — workflows can run minutes to hours; everything beyond 10s lands in `+Inf` and `histogram_quantile(0.99, ...)` cannot recover real latency.
- `NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS` — under saturation, pool wait can be tens of seconds to minutes.
- `NEBULA_CREDENTIAL_REFRESH_COORD_HOLD_DURATION_SECONDS` — full IdP round-trip plus retries can be tens of seconds.
- `NEBULA_RESOURCE_CREDENTIAL_ROTATION_DISPATCH_LATENCY_SECONDS` — same.

Bucket layout is per-(name, labels) at first registration (`crates/telemetry/src/metrics.rs:489-511`); changing it later silently keeps the original layout and emits a `tracing::warn!` that operators will not see. SLO definitions then either use the wrong buckets or break when the metric is re-bucketed in a future version.

Classification: API correction. Per-metric bucket schemas in the catalog with extended boundaries (e.g., logarithmic spacing up to 3600s) for known long-running durations.

### M-017: High — `Counter::inc_by(0)` keeps stale series alive

`Counter::inc_by(n: u64)` always stores `now_ms()` into `last_updated_ms` (`crates/telemetry/src/metrics.rs:54-57`), even when `n == 0`. `MetricsRegistry::retain_recent` (`crates/telemetry/src/metrics.rs:575-583`) decides whether to evict a series by `last_updated_ms() >= cutoff`.

Pattern: a per-loop "tick" path that calls `counter.inc_by(0)` to "record liveness" — or a defensive caller that increments by a deferred count which can be zero — will pin every involved series in memory forever, defeating the only cardinality-compaction tool the registry exposes.

Compare with `Histogram::observe`, which short-circuits non-finite values BEFORE touching `last_updated_ms` (`crates/telemetry/src/metrics.rs:220-246`). Counter has no equivalent guard.

Classification: Patch in `nebula-telemetry` (skip the timestamp store on `n == 0`). But the deeper architectural point — that `retain_recent` requires `&mut self` while production uses `Arc<MetricsRegistry>` and so is unreachable in prod composition — turns this into Architecture correction territory: the only cardinality compaction is unreachable, so the timestamp-update cost is paid for a feature that cannot run.

### M-018: High — Cumulative event totals exposed as gauges

Two pairs of metrics use gauge type for cumulative event counts, which contradicts Prometheus conventions:

**Cache events** (`crates/metrics/src/naming.rs:418-425`, `crates/metrics/src/export/prometheus.rs:99-102`):
- `NEBULA_CACHE_HITS` / `NEBULA_CACHE_MISSES` / `NEBULA_CACHE_EVICTIONS` — gauges, HELP "snapshot".
- These are event counts, not states. `rate()` over a gauge of cumulative count works only until the source restarts (then the gauge resets to 0 and `rate()` returns a large negative or jumps).

**EventBus snapshots** (`crates/metrics/src/naming.rs:255-261`, `crates/metrics/src/adapter.rs:223-264`):
- `NEBULA_EVENTBUS_SENT` / `NEBULA_EVENTBUS_DROPPED` are gauges. The source `EventBusStats::sent_count` / `dropped_count` are u64 cumulative counters since bus start (`crates/eventbus/src/stats.rs:24-31`). Same problem.
- `NEBULA_EVENTBUS_SUBSCRIBERS` is correctly a gauge (it's a state).

Failure scenario: bus restarts, `sent_count` resets to 0, gauge is set to 0; `rate(nebula_eventbus_sent[5m])` returns a negative value; alert misfires or silently breaks.

Classification: API correction. Rename to `_total` counters where the source is cumulative, keep gauges only for true point-in-time states (`_size`, `_subscribers`, `_active_total` — though the latter's `_total` suffix is misleading; see M-019).

### M-019: High — `_total` suffix on a gauge — `nebula_credential_active_total`

`NEBULA_CREDENTIAL_ACTIVE_TOTAL` (`crates/metrics/src/naming.rs:275-276`) is a gauge of currently-active credentials. The `_total` suffix is reserved by Prometheus convention for monotonically-increasing counter totals. A non-monotonic value carrying `_total` will mislead PromQL authors who write `rate(nebula_credential_active_total[5m])` and get "rate of how active count changed", not what they meant.

Classification: API correction (rename to `nebula_credentials_active`, accept the breaking change before this is in any production dashboard). Public API stability for a misleading name is a worse outcome than the rename.

### M-020: High — `histogram_help` does not cover the refresh-coord hold-duration histogram

`NEBULA_CREDENTIAL_REFRESH_COORD_HOLD_DURATION_SECONDS` (`crates/metrics/src/naming.rs:385-394`) is a normative spec metric (sub-spec §6) with rich, operator-actionable docs in the constants module. But `histogram_help` (`crates/metrics/src/export/prometheus.rs:107-121`) does not match it, so it exports as `# HELP nebula_credential_refresh_coord_hold_duration_seconds Custom histogram.`

Operators cannot read "P99 should sit below `claim_ttl`" from the scrape output, and the spec-mandated SLO check is invisible from the metrics layer alone.

Classification: Patch (add the match arm), but really part of M-006 — the catalog descriptor system needs to be the single source of truth, not a hand-maintained list duplicated across `naming.rs` constants and `prometheus.rs` match arms.

### M-021: High — Sanitization can collapse third-party raw names onto stable catalog names

`sanitize_metric_name` (`crates/metrics/src/export/prometheus.rs:158-182`) and `allocate_exported_metric_name` (`crates/metrics/src/export/prometheus.rs:194-214`) silently rewrite invalid identifiers. Catalog constants are all valid identifiers, so the collision target is normally another raw string.

But: a third-party crate or plugin that records `"nebula action_executions_total"` (note the space) will sanitize to `nebula_action_executions_total` — the same exported name as the canonical action counter. `allocate_exported_metric_name` shares the `metric_raw_to_exported` map across kinds, so:

- If both raw names land in the same kind map (e.g., counters), the second registration gets a `__{hash}` suffix, splitting the dashboard's series into a base and a hash-suffixed shadow.
- If the third-party records as gauge while the catalog records as counter, the gauge family inherits the counter's exported name (per M-003) and the scrape carries duplicate `# TYPE` lines.

This is a sharper version of M-011: not just renaming bad names, but **the existence of a similar valid catalog name turns a bad input into a public-API hazard**.

Classification: Architecture correction. Reject raw `&str` recording in the safe API; require typed `MetricName` validated against the catalog at compile time.

### M-022: Medium — Per-sample collision disambiguation depends on Spur allocation order

`render_labels` (`crates/metrics/src/export/prometheus.rs:128-156`) iterates `labels.iter()` (sorted by interner `Spur`, which is allocation order — `crates/telemetry/src/labels.rs:196-218`). When two raw keys sanitize to the same string, the *first-iterated* one keeps the unsuffixed name and the second gets `__{hash}`.

If two registry instances intern the keys in different order (e.g., test setup vs. production startup, or after a `compact_interner` rebuild), the same logical series exports with different label keys: in one run `a_b` is the "real" key, in another it is `a_b__abcdef`. Tests that compare full output (P1 in original audit) will be flaky on top of being wrong.

Classification: Refactor — sort by raw key string before rendering, not by Spur.

### M-023: Medium — Hot-path `now_ms()` cost paid for unreachable feature

Every `Counter::inc` / `Counter::inc_by` / `Gauge::set` / `Histogram::observe` performs a `SystemTime::now()` call to update `last_updated_ms` (`crates/telemetry/src/metrics.rs:21-28, 48-57, 95-110, 215-246`). On Windows this is a syscall; on Linux it is a vDSO read but still cache-perturbing.

The only consumer of `last_updated_ms` is `MetricsRegistry::retain_recent`, which requires `&mut self` (`crates/telemetry/src/metrics.rs:575-583`). Production composition wraps the registry in `Arc<MetricsRegistry>` (`crates/api/src/state.rs:91`), so `retain_recent` can never be called in production without dropping all clones first — i.e., never.

We pay the cost on every metric write to support a feature that no production path can invoke.

Classification: Architecture correction. Either (a) provide a copy-on-write or `Arc<RwLock>` registry that allows compaction without `&mut self`, or (b) gate `last_updated_ms` updates behind a `cfg(feature = "compaction")` and document the trade-off.

### M-024: High — Registered-but-unobserved histograms produce no scrape entries

`crates/metrics/src/export/prometheus.rs:441-453` (`empty_histogram_renders_all_zeros`) tests that an empty registry emits nothing — but the test name is misleading. The actual semantic is: **a histogram that has been created via `registry.histogram(name)` but never observed produces no scrape lines** because the snapshot iterates the histograms `DashMap`, and an unobserved histogram still has an entry but the test only checks the `count > 0` case.

Reading more carefully: `registry.histogram("name")` *does* insert an entry (`crates/telemetry/src/metrics.rs:443-446`, `entry().or_default()`), so the histogram WILL appear in `snapshot_histograms()` with all bucket counts at zero. So registered-but-unobserved DOES emit `_bucket{le="..."} 0`, `_sum 0`, `_count 0`. **But:** if no caller ever touches the handle, the histogram is never created in the registry at all — there is no eager pre-registration of catalog metrics. So a dashboard expecting `nebula_workflow_execution_duration_seconds` to be present in zero-traffic scenarios will see nothing.

This is a contract gap: the catalog promises a stable set of metrics, but only call-site activity creates the registry entries. A fresh process with zero workflow runs has no `_count` / `_sum` series for any duration histogram.

Classification: API correction. Provide a "register all catalog descriptors at startup" path so dashboards can rely on the family existing even at zero traffic.

### M-025: Medium — `inc_by(0)` and `Gauge::set` have no `Duration`-typed wrappers; unit confusion is on the caller

The metric-name suffix (`_seconds`, `_bytes`, `_milliseconds`) is purely conventional. `Histogram::observe(value: f64)` accepts whatever the caller passes. Engine code at `crates/engine/src/runtime/runtime.rs:775` uses `started.elapsed().as_secs_f64()` — correct, but the type system did not enforce it. A future caller writing `as_millis() as f64` into the same metric is undetectable until a dashboard query returns 1000× what was expected.

Classification: API correction. Domain methods on the adapter must accept `Duration` and convert internally; raw `f64` should not be the public surface for `_seconds`.

### M-026: Medium — `compact_interner` rebuild is not visible to other `Arc<MetricsRegistry>` clones

`MetricsRegistry::retain_recent` (`crates/telemetry/src/metrics.rs:575-583`) calls `compact_interner` (`crates/telemetry/src/metrics.rs:603-642`), which **replaces** `self.interner` with a fresh one and rebuilds the maps. But because production holds `Arc<MetricsRegistry>` (`crates/api/src/state.rs:91`), and `retain_recent(&mut self)` requires unique ownership, the compaction can only run when no clones exist.

Even setting that aside: any holder of a `Counter` / `Gauge` / `Histogram` clone (each of which holds an `Arc<AtomicU64>` etc., not an interner Spur) keeps the OLD atomic alive. After `compact_interner` rebuilds, new lookups into the registry create *fresh* atomics for the same name — and the cached handle on the call site continues writing into the old, now-detached atomic. Recordings vanish from the new registry view.

This compounds M-023: not only is `retain_recent` unreachable in prod, but if it ever does run, it silently desyncs cached metric handles.

Classification: Architecture correction. The cached-handle pattern is incompatible with rebuild-style compaction. Need either handle invalidation (force re-fetch) or a stable indirection layer.

## New Critical Findings Table

| ID | Severity | Area | Problem | Failure Scenario | Recommended Fix |
|----|----------|------|---------|------------------|-----------------|
| M-013 | Critical | Docs vs Type | Health-state gauge documents 0.5 but type is i64 | Operator builds "degraded" alert that can never fire | API correction or doc fix |
| M-014 | High | Histogram | `_sum` can overflow to lowercase `inf`, breaking scrape | Long-running histogram disappears from Prometheus | API correction |
| M-015 | High | Histogram | Negative observations distort `_seconds` semantics | NTP step backward → negative `_sum` → wrong avg latency | API correction |
| M-016 | High | Histogram | Default buckets miss long-running workflow / wait / refresh durations | P99 latency unrecoverable above 10s | API correction |
| M-017 | High | Counter | `inc_by(0)` pins series in `retain_recent` | Cardinality compaction defeated by liveness pings | Patch + Architecture |
| M-018 | High | Naming | Cache and EventBus cumulative counts exposed as gauges | `rate()` breaks on source restart | API correction |
| M-019 | High | Naming | `_total` suffix on a non-monotonic gauge | PromQL authors compute wrong rate | API correction |
| M-020 | High | Catalog | `histogram_help` missing refresh-coord hold-duration | Spec-mandated SLO invisible in HELP | Patch (subsumed by M-006) |
| M-021 | High | Catalog | Sanitization can collide third-party raw names with catalog names | Plugin pollutes `nebula_action_executions_total` | Architecture correction |
| M-022 | Medium | Export | Per-sample collision disambiguation depends on Spur allocation order | Same logical series exports differently across runs | Refactor |
| M-023 | Medium | Performance | Hot-path `now_ms()` for unreachable compaction feature | Per-record syscall with no payoff in prod | Architecture correction |
| M-024 | High | Catalog | Registered-but-unobserved metrics silently absent from scrape | Zero-traffic process has no `_count`/`_sum` for stable metrics | API correction |
| M-025 | Medium | Types | `_seconds` metric accepts raw `f64`; unit confusion not caught | Dashboard reads 1000× the truth after a unit refactor | API correction |
| M-026 | Medium | Concurrency | `compact_interner` desyncs cached metric handles | Compaction silently drops recordings on the floor | Architecture correction |

## Aggregated Recommendation Sharpened

The original audit's six-phase plan stands. The new findings tighten three priorities:

1. **The catalog must own bucket schemas, not just names.** M-016 plus M-014/M-015 mean histogram correctness depends on per-metric boundary configuration that `naming.rs` does not currently hold. Phase 1 (Define Metric Catalog and Label Schemas) must include bucket layout and unit type, not just name+kind+labels.

2. **The metrics boundary must accept typed values, not raw `f64` / `&str`.** M-013/M-015/M-019/M-021/M-025 all reduce to "the public API takes weakly-typed primitives and the caller is trusted to honor name-encoded contracts". Phase 2 should introduce `Duration`-only duration recording, `MetricName` validated against the catalog at compile time, and reject `Gauge` use for monotonically-increasing event counts.

3. **`retain_recent` is fundamentally incompatible with the production composition pattern.** M-017/M-023/M-026 form a coherent thread: compaction requires `&mut`, production holds `Arc`, cached handles desync after rebuild, and the `last_updated_ms` overhead pays for a feature nobody can call. Either remove the feature or replace it with a compaction-safe registry. Don't ship the half-finished version into "stable" maturity.

## New GitHub Issues

### Issue M-013: Resolve health-state contract — gauge cannot represent 0.5

Severity: Critical (contract integrity)

The naming docstring and HELP line on `NEBULA_RESOURCE_HEALTH_STATE` document `1=healthy, 0.5=degraded, 0=unhealthy`, but `Gauge` is `AtomicI64` and cannot accept `0.5`. Either change the gauge to f64 (or to a typed enum gauge) and wire emission, or remove the half-value tier from docs and HELP.

Acceptance tests:

- HELP text and naming docstring agree with what `Gauge::set` accepts.
- An emission-side test sets each documented value and a scrape parser reads it back.

### Issue M-014: Reject non-finite histogram outcomes (not just inputs)

Severity: High

`Histogram::observe` filters non-finite inputs but allows `sum_bits` to overflow to `Inf` over many large finite observations. Once `Inf` is in the accumulator, every scrape emits lowercase `inf`, which Prometheus rejects.

Recommended fix: detect non-finite outcome after the atomic update and either saturate, route through a diagnostic counter, or render in Prometheus form (`+Inf` / `-Inf`).

Acceptance tests:

- Stress observe with very large finite values; assert `_sum` never renders as lowercase `inf`.
- Diagnostic counter increments on saturation.

### Issue M-015: Reject negative observations at the metrics boundary

Severity: High

The metrics boundary should refuse negative durations for `_seconds` histograms. Today `observe(-0.1)` lands in bucket 0 and corrupts `_sum`.

Recommended fix: adapter accepts `Duration` only; `f64`-taking accessors validate `>= 0.0 && is_finite()` and route the rejection through a diagnostic counter.

Acceptance tests:

- Negative `f64` rejected; diagnostic counter increments; metric count unchanged.
- `Duration`-typed adapter API does not compile when called with `i64` ms by accident.

### Issue M-016: Per-metric bucket schemas in the catalog

Severity: High

Workflow / acquire-wait / refresh-hold / rotation-dispatch histograms all use the default HTTP-latency buckets and lose resolution above 10s. A catalog descriptor should specify boundaries per metric.

Recommended fix: `MetricDescriptor` includes `BucketSchema`; pre-register descriptors at registry construction so first-emit semantics do not lock in default buckets.

Acceptance tests:

- Workflow duration p99 of 600s observation reads back as 600s, not `+Inf`.
- Schema mismatch at registration (different boundaries) is detected and rejected at startup, not at scrape time.

### Issue M-017: Skip timestamp update for `inc_by(0)` and decide compaction strategy

Severity: High

`Counter::inc_by(0)` keeps a series alive in `retain_recent`. Patch the immediate behavior, but also resolve the deeper architectural question: `retain_recent` requires `&mut self` and production holds `Arc`, so the feature is unreachable today.

Recommended fix:

- Patch: `inc_by(0)` early-return.
- Architecture: replace `retain_recent` with a compaction-safe design (see M-026) or remove the feature and stop paying the `now_ms()` cost on every record (M-023).

Acceptance tests:

- `inc_by(0)` does not move `last_updated_ms` forward.
- Compaction proposal is decided in an ADR; either feature is supported in production composition or it is removed from the public API.

### Issue M-018: Convert cumulative-event gauges to counters

Severity: High

Cache hits/misses/evictions and EventBus sent/dropped are cumulative event counts exposed as gauges. Rename to `_total` counters; keep gauges only for true point-in-time state (`_size`, `_subscribers`).

Acceptance tests:

- New `_total` counter exposed; old gauge remains until callers migrate or is removed in the same breaking release.
- A dashboard query `rate(_total[5m])` returns sane values across a simulated source restart.

### Issue M-019: Rename `nebula_credential_active_total` to drop the `_total` suffix

Severity: High

A gauge with `_total` suffix is a Prometheus naming-convention violation that misleads PromQL authors. Rename to `nebula_credentials_active` (gauge) before downstream dashboards take a hard dependency.

Acceptance tests:

- Catalog test rejects `_total` suffix on non-monotonic kinds.
- Old name continues to be emitted only if a deprecation window is required; otherwise the rename is a hard break.

### Issue M-020: Add `histogram_help` arm for refresh-coord hold-duration

Severity: High (reduces to Patch once M-006 catalog refactor lands)

`NEBULA_CREDENTIAL_REFRESH_COORD_HOLD_DURATION_SECONDS` is a spec metric but exports as `Custom histogram.`. Patch the match arm now; track the descriptor-catalog refactor as the durable fix (M-006).

### Issue M-021: Forbid raw `&str` metric registration in the safe API

Severity: High

Raw-name recording allows third-party code to land samples on the same exported name as a stable catalog metric (after sanitization), splitting series or duplicating `# TYPE` lines.

Recommended fix: the safe public API takes a typed `MetricName` validated against the catalog. The raw `&str` path stays in `nebula-telemetry` as an explicit escape hatch and is no longer re-exported from `nebula-metrics`.

Acceptance tests:

- Third-party crate cannot record `"nebula action_executions_total"` (with a space) through the safe API.
- The escape-hatch path is explicit and not in the prelude.

### Issue M-022: Stable cross-run label-key collision disambiguation

Severity: Medium

Per-sample collision disambiguation currently depends on Spur allocation order, which is not stable across registry rebuilds. Sort by raw key string before rendering.

Acceptance tests:

- Same logical labels exported identically across two registry instances with different intern history.

### Issue M-023: Decide on `last_updated_ms` future

Severity: Medium (architectural)

Every metric write pays a `now_ms()` cost to support `retain_recent`, which production cannot call. Either provide a compaction-safe registry or remove the timestamp-update overhead. Coupled to M-017 and M-026.

Recommended fix: ADR documenting the choice; do not leave the half-finished feature in `stable`-maturity API.

### Issue M-024: Pre-register catalog metrics at startup

Severity: High

Zero-traffic processes do not expose stable catalog families because metric entries are created on first call. Dashboards relying on `nebula_workflow_execution_duration_seconds_count == 0` for a fresh process see no series at all.

Recommended fix: at registry construction, walk the catalog descriptors and create entries with zero values; subsequent recording flows fill them in.

Acceptance tests:

- Fresh process scrape contains every catalog family with zero values.
- Adding a new catalog metric automatically appears in the zero-traffic baseline.

### Issue M-025: `Duration`-typed adapter methods for `_seconds` metrics

Severity: Medium

Raw `f64` recording for `_seconds` allows silent unit confusion (`as_millis() as f64` instead of `as_secs_f64()`). Adapter domain methods should accept `Duration`.

Acceptance tests:

- Adapter signature requires `Duration`; passing `u64` ms by accident does not compile.
- Internal conversion is explicit (`d.as_secs_f64()`), not duplicated in callers.

### Issue M-026: Resolve compaction-vs-cached-handle desync

Severity: Medium (architectural)

If `compact_interner` ever runs on a registry that has cached `Counter` / `Gauge` / `Histogram` clones outside the registry maps, those handles continue writing to detached atomics. New scrapes will not see those updates.

Recommended fix (paired with M-023):

- If compaction stays: add handle invalidation (e.g., Counter holds `Weak<Inner>` + a generation counter checked on write).
- If compaction goes: drop `last_updated_ms` and the cached-detach hazard goes with it.

Acceptance tests:

- Hold a counter handle across a `retain_recent` call that prunes its series; record more values; assert that either (a) the new registry sees the writes, or (b) the writes are explicitly diagnosed as discarded — not silently dropped.

## Layering Responsibility Classification (2026-05-05)

The original audit's "Patch / Refactor / API correction / Architecture correction / Documentation/test only" axis answers *what shape the fix takes*. This section answers *which crate owns the fix*, applying the layering rule:

1. **Own responsibility** — `nebula-metrics` enforces or fixes it internally.
2. **Boundary contract issue** — `nebula-metrics` must expose clearer types/docs/errors so the adjacent layer can enforce its responsibility. The catalog owns the contract; emission lives elsewhere.
3. **Downstream responsibility** — `nebula-metrics` is correct; the consuming crate (engine / api / resource / plugin author) must adapt.
4. **Upstream responsibility** — `nebula-metrics` depends on a stronger primitive/contract from `nebula-telemetry`.

The layering rule prohibits fixing a boundary problem by relocating the policy into the wrong crate. Recording semantics belong to the producer; cardinality, naming, label safety, and Prometheus correctness belong to `nebula-metrics`; primitive atomic correctness and bucket data structures belong to `nebula-telemetry`. HTTP serving belongs to `nebula-api`. Logs belong to `nebula-log`.

### Classification Table

| Finding | Class | `nebula-metrics` action | Other-crate action |
|---------|-------|--------------------------|--------------------|
| M-001 | Own | Stop re-exporting `MetricsRegistry` from prelude/root; remove `registry()`; replace generic `&str` accessors with catalog-typed ones. | none |
| M-002 | Boundary contract | Provide `SafeLabels::build(raw_pairs, schema)` API that checks keys before any value reaches the interner. The catalog owns the schema. | `nebula-telemetry` may help with a `filter_then_intern(pairs, allowed_keys)` primitive (Upstream-coordinated), but the *contract* "high-cardinality values do not enter the registry" is owned here. |
| M-003 (registration side) | Boundary contract | Catalog `MetricDescriptor` binds name to kind; recording API rejects cross-kind reuse before reaching telemetry. | `nebula-telemetry` may add `register_kind(name, kind)` returning conflict — but the *policy* (one kind per name) is metric-catalog policy, not primitive policy. Don't push policy down. |
| M-003 (export side) | Own | Exporter detects `# TYPE` family duplication after the three phases; emits `ExportError` rather than rendering invalid output. | none |
| M-004 | Upstream | Use atomic snapshot when telemetry exposes one. | `nebula-telemetry`: `Histogram::snapshot() -> HistogramSnapshot { count, sum, buckets }` reading via seqlock or equivalent. The atomicity contract is a primitive-correctness concern owned by telemetry. |
| M-005 (cross-series) | Own | Detect cross-series sanitized label-key collision in the exporter; reject or diagnose. | none |
| M-006 | Own | Single catalog source of truth — generate exporter HELP/TYPE from descriptors; remove the hand-maintained `match` arms. | none |
| M-007 | Boundary contract | Replace 3 disjoint workflow counters with `executions_terminated_total{status, failure_class}` (or similar bounded labels) so the engine cannot ignore terminal states. | `nebula-engine`: emit through the new typed API; map every `ExecutionStatus` variant to a bounded outcome. The choice "what is a status" stays in engine; the metric *shape* is owned here. |
| M-008 | Boundary contract | Expose `action_finished(outcome: ActionOutcome, failure_class: …)` with `ActionOutcome` as a closed enum that includes `Success`, `RuntimeError`, `TerminateFailure`, `Cancelled`, etc. | `nebula-engine`: interpret `ActionResult` and call the typed adapter method. The mapping `ActionResult → ActionOutcome` is engine-domain. |
| M-009 | Own | Replace global `LabelAllowlist` with per-metric `LabelSchema` carried by descriptors; emit diagnostic counter for rejected keys. | none (downstream call sites adopt the typed API once it exists) |
| M-010 | Boundary contract | (a) Stop re-exporting `MetricsRegistry`. (b) Provide a `MetricsCatalog` wrapper that production composition wires. | `nebula-engine` / `nebula-api` / `nebula-resource`: migrate from raw `nebula_telemetry::metrics::MetricsRegistry` to the catalog wrapper. Pure downstream migration once the wrapper exists. |
| M-011 | Own | Reject invalid catalog names at descriptor registration; reserve sanitization for an explicitly marked "legacy" path with diagnostic output. | none |
| M-012 | Boundary contract | Catalog must define descriptors for queue / scheduler / retry / fallback / circuit-breaker / API-RED metrics. | `nebula-engine` / `nebula-api` / `nebula-resource`: emit through the new descriptors. Downstream-side wiring once the catalog publishes the contract. |
| M-013 | Boundary contract | If 0.5 stays in the contract, demand an f64 gauge from telemetry; otherwise change the docstring/HELP to drop the half-tier. The catalog decides the value space. | If 0.5 stays: `nebula-telemetry` provides `FloatGauge`. Don't fake fractional values via `*100` scaling on the metrics side — that hides the missing primitive. |
| M-014 (rendering) | Own | Exporter must render `f64::INFINITY` as `+Inf`, `-Inf`, or `NaN` (Prometheus form), not Rust's lowercase `inf`. | none |
| M-014 (overflow) | Upstream | Use telemetry's saturation-aware sum once it exists. | `nebula-telemetry`: `Histogram::sum_state() -> Finite(f64) | Saturated` or saturating add. Primitive-correctness concern. |
| M-015 | Own | Adapter accepts `Duration` (not `f64`) for `_seconds` metrics; rejects NaN/negative at the typed boundary; routes rejection through a diagnostic counter. | none — telemetry's general-purpose histogram is correct to accept any f64; the *domain restriction* "negative is meaningless for `_seconds`" is a metric-catalog concern. |
| M-016 | Own | `MetricDescriptor` carries bucket schema; catalog pre-registers histograms with correct boundaries before first emission. | none — `Histogram::with_buckets` already exists upstream. |
| M-017 | Upstream | none directly. | `nebula-telemetry`: `Counter::inc_by(0)` short-circuits without updating `last_updated_ms`. The atomic-write timestamp is a primitive-internal concern. |
| M-018 | Own | Rename to `_total` counters; refactor `record_eventbus_stats` to compute deltas internally. | `nebula-eventbus` is allowed to keep its cumulative `EventBusStats` shape — the adapter does the cumulative→delta translation. |
| M-019 | Own | Rename `nebula_credential_active_total` → `nebula_credentials_active`. Pure naming. | none |
| M-020 | Own | Add the missing `histogram_help` match arm (interim); subsumed by the catalog refactor (M-006). | none |
| M-021 | Own | Forbid raw `&str` in the safe public API; require `MetricName` validated at compile time against the catalog. | none. `nebula-telemetry`'s raw `&str` API stays as the explicit escape hatch — it is correct for telemetry's role. |
| M-022 | Own | Sort by raw key string before rendering disambiguated label sets. Pure exporter. | none |
| M-023 | Upstream | Use whatever telemetry decides. | `nebula-telemetry` ADR: either compaction-safe registry, or gate `last_updated_ms` behind a feature, or drop compaction. The hot-path cost is paid in the primitive. |
| M-024 | Own | Catalog walks descriptors at registry construction and pre-registers entries so zero-traffic processes still expose the families. | none — telemetry already supports zero-valued atomics. |
| M-025 | Own | Adapter typed signatures take `Duration`. Pure adapter API. | none |
| M-026 | Upstream | Use whatever telemetry decides (paired with M-023). | `nebula-telemetry`: handle invalidation (Weak + generation) or drop the compaction feature. The cached-handle / rebuild incompatibility is intrinsic to telemetry's primitive design. |

### Layering Anti-patterns to Avoid in the Refactor

- **Do not** push cardinality enforcement into `nebula-telemetry`. Cardinality safety is a metric-catalog concern; primitives must remain general-purpose. Telemetry's role is "store atomics keyed by interned strings", not "decide which strings are safe."
- **Do not** push naming policy into `nebula-engine` / `nebula-api` / `nebula-resource`. Those crates are *callers* of the catalog. If the catalog leaks the responsibility (e.g., naming constants only, no typed registration), downstream crates accumulate ad-hoc strings (M-010 today). Catalog owns the contract; downstream owns the emission semantics.
- **Do not** put HTTP-status-code logic in `nebula-metrics`. The 503 fallback when `metrics_registry` is absent (`crates/api/src/routes/metrics.rs:9-29`) belongs to `nebula-api`. The `snapshot() -> String` plus `content_type()` boundary is correct; do not extend it with HTTP concerns.
- **Do not** "fix" M-007 / M-008 by adding more concrete counters in engine without changing the catalog. That trades one form of drift (catalog vs emission) for another (engine vs other engines/plugins). The boundary fix is to make the catalog the only legal recording shape.
- **Do not** "fix" M-002 / M-004 / M-014 / M-017 / M-023 / M-026 by working around telemetry from inside metrics. Each one has a primitive-shaped subproblem that belongs upstream. Pretending to solve it in the wrong crate produces buggy half-fixes (e.g., a sum-saturation check at scrape time that misses observations between two scrapes).
- **Do not** silently sanitize invalid descriptors at any layer. Reject at registration; the catalog is the contract source of truth.

### Net Layering Picture

Of 26 findings:

- **Own (12)**: M-001, M-003-export, M-005, M-006, M-009, M-011, M-014-rendering, M-015, M-016, M-018, M-019, M-020, M-021, M-022, M-024, M-025 — `nebula-metrics` fixes these without touching adjacent crates.
- **Boundary contract (8)**: M-002, M-003-registration, M-007, M-008, M-010, M-012, M-013 — `nebula-metrics` publishes a stronger typed API; downstream emitters migrate.
- **Upstream (5)**: M-004, M-014-overflow, M-017, M-023, M-026 — require `nebula-telemetry` primitive changes. `nebula-metrics` cannot honestly close these without coordinated upstream work.
- **Downstream-only (0)**: nothing in this audit is "metrics is fine, downstream must fix" alone — every recording-side gap also reveals a missing catalog contract.

The ratio (8 Boundary + 5 Upstream out of 26) tells the structural story: this crate is currently described as the metric boundary, but a third of its problems require either a stronger telemetry primitive or a reshaped own API before downstream crates can do the right thing. **Closing the Upstream-class items is a prerequisite for marking the metrics boundary "stable" in any honest sense.**
