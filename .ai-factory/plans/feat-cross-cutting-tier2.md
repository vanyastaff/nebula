# Cross-Cutting Tier 2 — Core Feature Gaps

> Implement all 9 Tier 2 milestones across config, telemetry, metrics, and log crates
> to close the feature gaps required for real-world deployment.

**Branch:** `feat/cross-cutting-tier2`
**Created:** 2026-03-12

---

## Settings

- **Testing:** yes
- **Logging:** verbose
- **Docs:** yes (mandatory checkpoint)

## Roadmap Linkage

- **Milestone:** "Tier 2 — Core Feature Gaps (Required for Real-World Deployment)"
- **Rationale:** Direct 1:1 mapping — this plan covers all 9 items in the Tier 2 milestone block.

---

## Exploration Summary

### Config
- **YAML:** `ConfigFormat::Yaml` enum variant exists but parsing returns `format_not_supported("yaml")`. `from_extension()` doesn't recognize `.yaml`/`.yml`. Follow TOML feature-flag pattern: add `serde_yaml` behind `yaml` feature.
- **Interpolation:** Zero existing code. Pipeline: Loader → parsed JSON → **interpolation hook** → merge → validate → store. Recommended: post-parse per-source pass using `${VAR}` and `${VAR:-default}` regex.

### Telemetry
- **TelemetryService trait:** 5 methods (`event_bus`, `metrics`, `event_bus_arc`, `metrics_arc`, `execution_recorder`). Only impl: `NoopTelemetry`.
- **Recorder trait:** Sync methods `record_usage()`, `record_call()`. Only impl: `NoopRecorder`. `tokio::sync::mpsc` available.
- **ExecutionEvent:** 7 variants, NO trace context fields. OpenTelemetry `0.31.0` in workspace deps but not imported in telemetry crate.
- Dependencies between features: none — all three can be developed independently.

### Metrics
- **snapshot():** Hardcoded 5 counters + 2 histograms. Missing: 13 resource + 4 eventbus metrics. No per-bucket histogram rendering.
- **Dead deps:** `metrics` and `metrics-exporter-prometheus` crates are imported but NEVER used in code.
- **Histogram:** Already rewritten to bounded buckets in Tier 1 — `buckets()` method returns `Vec<(f64, u64)>`.

### Log
- **ReloadHandle:** Fully functional (`reload(filter_str)`) but stranded in private `Inner` struct — not exposed on `LoggerGuard`. Per-module levels supported via `EnvFilter` string syntax.
- **Async File Writer:** ALREADY IMPLEMENTED. `non_blocking: bool` config (default `true`) uses `tracing_appender::non_blocking`. Only gap: documentation and test coverage.

---

## Tasks

### Phase 1 — Config: YAML Loader (Tasks 1–3)

#### Task 1: Add `serde_yaml` dependency and `yaml` feature flag
- [x] **Done**
- **Files:** `crates/config/Cargo.toml`
- **What:**
  - Add `serde_yaml = { version = "0.9", optional = true }` to `[dependencies]`
  - Add `yaml = ["dep:serde_yaml"]` to `[features]`
  - Add `"yaml"` to the `default` feature list (yaml is common enough to be default)
- **Logging:** `DEBUG` log when YAML feature is compiled in (via `cfg` in loader)
- **Tests:** Verify feature compiles with `--features yaml`

#### Task 2: Implement YAML parsing in FileLoader
- [x] **Done**
- **Files:** `crates/config/src/loaders/file.rs`, `crates/config/src/core/source.rs`
- **What:**
  - In `parse_content()`, add `ConfigFormat::Yaml` arm following TOML pattern:
    ```rust
    ConfigFormat::Yaml => {
        #[cfg(feature = "yaml")]
        { serde_yaml::from_str::<serde_json::Value>(content)
            .map_err(|e| ConfigError::parse_error(path, format!("YAML parse error: {e}"))) }
        #[cfg(not(feature = "yaml"))]
        { Err(ConfigError::format_not_supported("yaml")) }
    }
    ```
  - In `from_extension()`, add `"yaml" | "yml"` arm returning `ConfigFormat::Yaml` (feature-gated)
- **Logging:** `tracing::debug!("parsing config file as YAML: {path}")` before parse
- **Tests:**
  - Parse simple YAML (`key: value`, nested objects, arrays)
  - Parse YAML with anchors and aliases
  - Error on malformed YAML (unclosed quote, invalid indent)
  - Error when `yaml` feature disabled

#### Task 3: YAML integration test with ConfigBuilder
- [x] **Done**
- **Files:** `crates/config/tests/` (new test file or extend existing)
- **What:**
  - End-to-end test: create temp `.yaml` file → `ConfigBuilder::new().with_source(file)` → verify values load correctly
  - Test nested structures, arrays, unicode keys
  - Test YAML overriding TOML values in composite loader
- **Logging:** N/A (test file)
- **Tests:** Integration tests behind `#[cfg(feature = "yaml")]`

> **Commit checkpoint:** `feat(config): add YAML config file support behind feature flag`

### Phase 2 — Config: Environment Variable Interpolation (Tasks 4–7)

#### Task 4: Add interpolation module with resolver
- [x] **Done**
- **Files:** `crates/config/src/interpolation.rs` (new module)
- **What:**
  - Create `interpolate(value: &serde_json::Value) -> ConfigResult<serde_json::Value>` function
  - Support two syntaxes: `${VAR}` (required) and `${VAR:-default}` (with fallback)
  - Walk JSON tree recursively; only interpolate `Value::String` leaves
  - Use `std::env::var()` for resolution (no custom resolver needed for now)
  - Return `ConfigError::InterpolationError` for unresolved required vars
- **Logging:**
  - `tracing::debug!("interpolating config value: found {count} variable references")`
  - `tracing::trace!("resolved ${VAR} from environment")` per variable (trace level for security — don't log values)
  - `tracing::warn!("unresolved variable ${VAR}, using default: {default}")` for fallback usage
- **Tests:**
  - `${VAR}` resolves to env value
  - `${VAR:-fallback}` uses fallback when VAR unset
  - `${VAR:-fallback}` uses VAR when set (fallback ignored)
  - Nested `${A}_${B}` in same string
  - Non-string values pass through unchanged
  - Deep nested JSON objects/arrays interpolated correctly
  - Missing required var returns `InterpolationError`
  - Empty `${}` returns error
  - Escaped `$$` not interpolated (literal `$`)

#### Task 5: Add InterpolationError variant
- [x] **Done**
- **Files:** `crates/config/src/core/error.rs`
- **What:**
  - Add `#[error("environment variable interpolation failed: {message}")]` variant with `message: String` and `key: Option<String>` fields
  - Add factory method `ConfigError::interpolation_error(msg: impl Into<String>, key: Option<String>)`
- **Logging:** N/A (error type only)
- **Tests:** Include in existing error tests

#### Task 6: Wire interpolation into ConfigBuilder pipeline
- [x] **Done**
- **Files:** `crates/config/src/core/builder.rs`, `crates/config/src/lib.rs`
- **What:**
  - After each source loads (in `build()` method), call `interpolate(&loaded_value)?` before merge
  - Add `with_interpolation(enabled: bool)` method to `ConfigBuilder` (default: `true`)
  - Re-export `interpolation` module in `lib.rs`
- **Logging:** `tracing::debug!("interpolation pass complete for source: {source_name}")`
- **Tests:**
  - Builder with interpolation enabled resolves vars
  - Builder with interpolation disabled passes through `${VAR}` literally
  - Interpolation on TOML source, JSON source, env source (all formats)

#### Task 7: Interpolation integration tests
- [x] **Done**
- **Files:** `crates/config/tests/` (new or extend)
- **What:**
  - End-to-end: set env vars → load TOML with `${VAR}` references → verify resolved values
  - Composite loader: TOML base + env override + interpolation
  - Hot-reload: change env var → trigger reload → verify new value propagated
  - Edge cases: recursive interpolation `${${VAR}}` should NOT be supported (single pass)
- **Logging:** N/A (test file)
- **Tests:** Integration tests

> **Commit checkpoint:** `feat(config): add environment variable interpolation with ${VAR} and ${VAR:-default} syntax`

### Phase 3 — Telemetry: ProductionTelemetry + BufferedRecorder (Tasks 8–11)

#### Task 8: Implement BufferedRecorder
- [x] **Done**
- **Files:** `crates/telemetry/src/trace.rs` (or new `crates/telemetry/src/recorder.rs`)
- **What:**
  - `BufferedRecorder` struct with `tokio::sync::mpsc::Sender` for sending records
  - `BufferedRecorderConfig`: `buffer_size: usize` (default 1024), `flush_interval: Duration` (default 5s)
  - Background task: receives from channel, batches up to `buffer_size`, flushes on interval or size trigger
  - Flush sink: trait `RecordSink: Send + Sync` with `async fn flush(records: Vec<RecordEntry>)` method
  - `RecordEntry` enum: `Usage(ResourceUsageRecord)` | `Call(CallRecord)`
  - Implement `Recorder` trait — `record_usage` and `record_call` do non-blocking `try_send()`
  - On channel full: log warning and drop record (back-pressure)
  - Provide `LogSink` default implementation (writes to `tracing::info!`)
  - Provide `shutdown()` method: closes sender, awaits final drain from receiver
- **Logging:**
  - `tracing::debug!("buffered recorder started: buffer_size={}, flush_interval={:?}", ...)`
  - `tracing::warn!("buffered recorder channel full, dropping record")` on back-pressure
  - `tracing::debug!("flushing {count} buffered records")` on flush
  - `tracing::info!("buffered recorder shutdown complete, drained {count} remaining records")`
- **Tests:**
  - Records are batched and flushed after interval
  - Records are flushed when buffer fills
  - Back-pressure: channel full → drops without panic
  - Shutdown drains remaining records
  - `LogSink` writes to tracing
  - Multiple concurrent producers

#### Task 9: Implement ProductionTelemetry
- [x] **Done**
- **Files:** `crates/telemetry/src/service.rs`
- **What:**
  - `ProductionTelemetry` struct implementing `TelemetryService`
  - Builder: `ProductionTelemetryBuilder` (consuming self, `#[must_use]`)
    - `.with_event_bus(Arc<EventBus>)` (default: `EventBus::new(1024)`)
    - `.with_metrics(Arc<MetricsRegistry>)` (default: `MetricsRegistry::new()`)
    - `.with_recorder(Arc<dyn Recorder>)` (default: BufferedRecorder with LogSink)
    - `.with_buffer_config(BufferedRecorderConfig)` (shortcut for BufferedRecorder)
    - `.build() -> ProductionTelemetry`
  - Implement all 5 `TelemetryService` methods (same as NoopTelemetry but with configurable components)
  - Constructor: `ProductionTelemetry::builder() -> ProductionTelemetryBuilder`
- **Logging:**
  - `tracing::info!("production telemetry service initialized")`
  - `tracing::debug!("telemetry: event_bus_capacity={}, recorder_type={}", ...)`
- **Tests:**
  - Builder with defaults creates working service
  - Builder with custom components wires correctly
  - `event_bus()` and `metrics()` return shared instances
  - `execution_recorder()` returns configured recorder

#### Task 10: Wire ProductionTelemetry into engine/runtime
- [x] **Done** (no changes needed — `Arc<dyn TelemetryService>` already works)
- **Files:** `crates/engine/src/`, `crates/runtime/src/` (find `with_telemetry` call sites)
- **What:**
  - Update examples/docs to show `ProductionTelemetry::builder().build()` usage
  - Ensure `Arc<dyn TelemetryService>` works for both Noop and Production
  - NO breaking changes — `NoopTelemetry` remains available and is still the default
- **Logging:** N/A (wiring only)
- **Tests:** Existing engine/runtime tests should pass unchanged (they use Noop)

#### Task 11: Re-export new types in prelude
- [x] **Done**
- **Files:** `crates/telemetry/src/prelude.rs`, `crates/telemetry/src/lib.rs`
- **What:**
  - Add to prelude: `ProductionTelemetry`, `ProductionTelemetryBuilder`, `BufferedRecorder`, `BufferedRecorderConfig`, `RecordSink`, `LogSink`, `RecordEntry`
  - Update lib.rs crate docs to mention ProductionTelemetry
- **Logging:** N/A
- **Tests:** N/A

> **Commit checkpoint:** `feat(telemetry): add ProductionTelemetry service and BufferedRecorder`

### Phase 4 — Telemetry: Trace Context Propagation (Tasks 12–14)

#### Task 12: Define TraceContext types
- [x] **Done**
- **Files:** `crates/telemetry/src/context.rs` (new module)
- **What:**
  - `TraceId(u128)` — W3C trace-id (16 bytes, hex-encoded)
  - `SpanId(u64)` — W3C span-id (8 bytes, hex-encoded)
  - `TraceContext { trace_id: TraceId, span_id: SpanId, parent_span_id: Option<SpanId>, sampled: bool }`
  - Implement `Display` (hex format), `FromStr` (parse hex), `Serialize`/`Deserialize`
  - `TraceContext::generate()` — creates new random IDs
  - `TraceContext::from_traceparent(header: &str) -> Result<Self>` — W3C `traceparent` header parsing
  - `TraceContext::to_traceparent(&self) -> String` — W3C `traceparent` header generation
  - Format: `00-{trace_id:032x}-{span_id:016x}-{flags:02x}`
- **Logging:** N/A (pure data types)
- **Tests:**
  - Round-trip: generate → to_traceparent → from_traceparent → assert equal
  - Parse valid traceparent header
  - Parse invalid traceparent → error
  - Display shows correct hex format
  - Serialization round-trip (JSON)

#### Task 13: Add TraceContext to ExecutionEvent
- [x] **Done**
- **Files:** `crates/telemetry/src/event.rs`
- **What:**
  - Add `trace_context: Option<TraceContext>` field to all 7 `ExecutionEvent` variants
  - This is a breaking change (allowed per project policy), but use `Option` to minimize migration pain
  - Update all test constructors to include `trace_context: None` (or use a helper)
  - Add helper: `ExecutionEvent::with_trace(self, ctx: TraceContext) -> Self`
  - Update `ScopedEvent` implementation if trace_context affects scope
- **Logging:** `tracing::debug!("execution event emitted: trace_id={}", trace_context.trace_id)` when present
- **Tests:**
  - Events with `None` trace context work as before
  - Events with `Some(trace_context)` carry context through EventBus
  - `with_trace()` helper works on all variants

#### Task 14: Add TraceContext to CallRecord
- [x] **Done**
- **Files:** `crates/telemetry/src/trace.rs`, `crates/telemetry/src/lib.rs`
- **What:**
  - Add `trace_context: Option<TraceContext>` field to `CallRecord`
  - Update `CallRecord` builder/constructor
  - Export `context` module from lib.rs, add to prelude
- **Logging:** N/A
- **Tests:**
  - CallRecord with trace context serializes correctly
  - BufferedRecorder preserves trace context through buffer

> **Commit checkpoint:** `feat(telemetry): add W3C TraceContext propagation to execution events and call records`

### Phase 5 — Metrics: Full Prometheus Export (Tasks 15–17)

#### Task 15: Render per-bucket histogram lines in snapshot
- [x] **Done**
- **Files:** `crates/metrics/src/export/prometheus.rs`
- **What:**
  - Update histogram rendering to use `Histogram::buckets()` method (from Tier 1)
  - Output per-bucket lines: `{name}_bucket{le="{boundary}"} {count}` for each boundary
  - Add final `{name}_bucket{le="+Inf"} {total_count}`
  - Keep `_sum` and `_count` lines
  - Full Prometheus-compliant histogram output
- **Logging:** N/A (export function)
- **Tests:**
  - Histogram with observations renders correct bucket counts
  - Empty histogram renders all zeros
  - Custom bucket boundaries render correctly

#### Task 16: Add resource and eventbus metrics to snapshot
- [x] **Done**
- **Files:** `crates/metrics/src/export/prometheus.rs`, `crates/metrics/src/naming.rs`
- **What:**
  - Add all 13 resource metrics to snapshot: 8 counters, 2 histograms, 3 gauges
  - Add all 4 eventbus metrics to snapshot: 4 gauges
  - Refactor from hardcoded arrays to dynamic registry iteration:
    - `registry.counters()`, `registry.gauges()`, `registry.histograms()` methods (if available)
    - Or iterate all metrics by name from the naming constants
  - Add `# HELP` and `# TYPE` metadata lines per Prometheus convention
- **Logging:** N/A (export function)
- **Tests:**
  - Snapshot includes resource metrics when present
  - Snapshot includes eventbus metrics when present
  - `# HELP` and `# TYPE` lines are correct
  - Empty registry produces valid (empty or zero-valued) output

#### Task 17: Clean up Prometheus feature flag and dead deps
- [x] **Done**
- **Files:** `crates/metrics/Cargo.toml`, `crates/metrics/src/export/prometheus.rs`
- **What:**
  - Remove `metrics` and `metrics-exporter-prometheus` from dependencies (dead code — confirmed unused)
  - Remove `prometheus = ["dep:metrics", "dep:metrics-exporter-prometheus"]` feature definition
  - Remove `#[cfg(feature = "prometheus")]` guards from export module (make always available)
  - Update `default` features if needed
  - The snapshot function is hand-written and doesn't use these crates
- **Logging:** N/A
- **Tests:** Verify `cargo check -p nebula-metrics` still compiles without the dead deps

> **Commit checkpoint:** `feat(metrics): full Prometheus export with per-bucket histograms and all metric domains`

### Phase 6 — Log: Dynamic Reconfiguration + Async Writer Docs (Tasks 18–20)

#### Task 18: Expose ReloadHandle on LoggerGuard
- [x] **Done**
- **Files:** `crates/log/src/builder/mod.rs`
- **What:**
  - Add `pub fn reload_handle(&self) -> Option<&ReloadHandle>` method to `LoggerGuard`
  - Returns `None` when `reloadable: false`, `Some(&handle)` when `reloadable: true`
  - Document the method with examples showing per-module level changes:
    ```rust
    if let Some(handle) = guard.reload_handle() {
        handle.reload("info,nebula_engine=debug,hyper=warn")?;
    }
    ```
- **Logging:** `tracing::info!("log filter reloaded: {new_filter}")` in `reload()` (if not already there)
- **Tests:**
  - `reload_handle()` returns `Some` when config `reloadable: true`
  - `reload_handle()` returns `None` when config `reloadable: false`
  - Reload changes effective log level (verify with tracing test subscriber)

#### Task 19: Add config watcher integration for auto-reload
- [x] **Done**
- **Files:** `crates/log/src/builder/mod.rs` or new `crates/log/src/reload.rs`
- **What:**
  - Add `watch_config(config_path: &Path, reload_handle: ReloadHandle)` function
  - Uses `notify` crate (already a dep of nebula-config) or `tokio::fs::watch` to watch a config file
  - On file change: read new level string → call `reload_handle.reload(level)`
  - Spawns background tokio task, returns `JoinHandle` or integrates into `LoggerGuard`
  - Config file format: simple text file with filter string, or JSON `{"level": "..."}`
  - Guard cleanup: task is cancelled on `LoggerGuard` drop
- **Logging:**
  - `tracing::info!("watching config file for log level changes: {path}")`
  - `tracing::info!("detected config change, reloading log filter: {new_filter}")`
  - `tracing::warn!("failed to read config file for reload: {err}")`
- **Tests:**
  - Write config file → start watcher → modify file → verify level changed
  - Invalid config content → logs warning, doesn't crash
  - Watcher task cancelled on guard drop

#### Task 20: Document async file writer and add test coverage
- [x] **Done**
- **Files:** `crates/log/src/config/writer.rs`, crate-level docs
- **What:**
  - Add doc comments to `non_blocking` field explaining sync vs async tradeoffs
  - Document in lib.rs `## Quick Start` that async is the default
  - Add unit tests verifying:
    - `default_non_blocking()` returns `true`
    - File writer config deserializes with non_blocking defaulting to true
    - File writer config with explicit `non_blocking: false` works
- **Logging:** N/A (documentation task)
- **Tests:** Config deserialization tests

> **Commit checkpoint:** `feat(log): expose ReloadHandle and add config watcher for dynamic log level changes`

### Phase 7 — CI Verification + Documentation (Tasks 21–22)

#### Task 21: Full CI check
- [x] **Done**
- **Files:** N/A (commands only)
- **What:**
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace -- -D warnings`
  - `cargo check --workspace --all-targets`
  - `cargo test --workspace`
  - `cargo doc --no-deps --workspace`
- **Logging:** N/A
- **Tests:** All crate tests pass

#### Task 22: Update crate documentation
- [x] **Done**
- **Files:** `crates/config/src/lib.rs`, `crates/telemetry/src/lib.rs`, `crates/metrics/src/lib.rs`, `crates/log/src/lib.rs`
- **What:**
  - Update `//!` crate docs for each affected crate to mention new features
  - Ensure `## Quick Start` examples reference new APIs (YAML config, ProductionTelemetry, etc.)
  - Update prelude re-exports for any newly added public types
- **Logging:** N/A
- **Tests:** `cargo doc --no-deps --workspace` passes with no warnings

> **Commit checkpoint:** `docs(config,telemetry,metrics,log): update crate documentation for Tier 2 features`

---

## Commit Plan

| Commit | Tasks | Message |
|--------|-------|---------|
| 1 | 1–3 | `feat(config): add YAML config file support behind feature flag` |
| 2 | 4–7 | `feat(config): add environment variable interpolation with ${VAR} and ${VAR:-default} syntax` |
| 3 | 8–11 | `feat(telemetry): add ProductionTelemetry service and BufferedRecorder` |
| 4 | 12–14 | `feat(telemetry): add W3C TraceContext propagation to execution events and call records` |
| 5 | 15–17 | `feat(metrics): full Prometheus export with per-bucket histograms and all metric domains` |
| 6 | 18–20 | `feat(log): expose ReloadHandle and add config watcher for dynamic log level changes` |
| 7 | 21–22 | `docs(config,telemetry,metrics,log): update crate documentation for Tier 2 features` |

---

## Dependencies

```
Task 1 → Task 2 → Task 3          (YAML: dep → parse → tests)
Task 5 → Task 4 → Task 6 → Task 7 (Interpolation: error → impl → wire → tests)
Task 8 → Task 9 → Task 10 → Task 11 (Telemetry: recorder → service → wiring → prelude)
Task 12 → Task 13, Task 14         (TraceContext: types → events, call records)
Task 15, Task 16, Task 17          (Metrics: independent, can be parallelized)
Task 18 → Task 19 → Task 20       (Log: expose handle → watcher → docs)
Task 1–20 → Task 21 → Task 22     (CI + docs: last)
```

Cross-phase dependencies:
- Phase 3 (Task 8, BufferedRecorder) is independent of Phases 1–2
- Phase 4 (Task 12–14) depends on Phase 3 (Task 8) only if we want BufferedRecorder to preserve trace context
- Phase 5 (Metrics) is independent of all other phases
- Phase 6 (Log) is independent of all other phases

---

## Notes

- **Log: Async File Writer (ROADMAP item):** Already implemented — `non_blocking: bool` config exists, defaults to `true`, uses `tracing_appender::non_blocking`. Task 20 covers documentation and test coverage only.
- **Metrics: Dead deps removal (Task 17):** The `metrics` and `metrics-exporter-prometheus` crates are confirmed unused — snapshot is hand-written. Safe to remove.
- **Breaking changes:** Task 13 (adding `trace_context` to `ExecutionEvent` variants) is a breaking change. Using `Option<TraceContext>` minimizes migration pain. All direct consumers (engine, runtime) will be updated in the same commit.
- **Histogram buckets:** Tier 1 already converted Histogram to bounded atomic buckets. Task 15 leverages the existing `buckets()` method for per-bucket Prometheus output.
