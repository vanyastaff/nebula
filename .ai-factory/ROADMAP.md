# Project Roadmap — Cross-Cutting Crates to Production

> Bring all 7 cross-cutting crates (config, log, system, resilience, eventbus, metrics, telemetry)
> to final production quality with complete APIs, comprehensive tests, and production-grade features.

---

## Milestones

### Tier 1 — Critical Fixes (Blockers for Production Use)

- [x] **EventBus: Lock Poisoning Recovery** — Replace all 8 `.expect("lock poisoned")` calls in `registry.rs` with `unwrap_or_else(PoisonError::into_inner)` to prevent cascading panics in production; add tests for poisoned-lock recovery scenarios
- [x] **EventBus: Add Prelude Module** — Create `pub mod prelude` exporting `EventBus`, `Subscriber`, `EventFilter`, `PublishOutcome`, `ScopedEvent`, `SubscriptionScope`, `BackPressurePolicy`, `EventBusStats`, `EventBusRegistry`; align with workspace convention
- [x] **Telemetry: Histogram OOM Fix** — Replace `Vec<f64>` in `Histogram` with bounded bucket-based storage (pre-defined or exponential buckets); add `percentile(f64)` method returning p50/p95/p99; cap memory usage regardless of observation count
- [x] **Telemetry: Add Prelude Module** — Create `pub mod prelude` exporting `TelemetryService`, `NoopTelemetry`, `EventBus`, `ExecutionEvent`, `MetricsRegistry`, `Counter`, `Gauge`, `Histogram`, `Recorder`, `CallRecord`, `ResourceUsageRecord`
- [x] **Metrics: Add Prelude Module** — Create `pub mod prelude` exporting `TelemetryAdapter`, `PrometheusExporter`, `Counter`, `Gauge`, `Histogram`, `MetricsRegistry`, and key naming constants

### Tier 2 — Core Feature Gaps (Required for Real-World Deployment)

- [x] **Config: YAML Loader** — Implement `ConfigFormat::Yaml` parser (currently returns error); add `serde_yaml` dependency behind `yaml` feature flag; add tests including nested structures, anchors, and edge cases
- [x] **Config: Environment Variable Interpolation** — Support `${VAR}` and `${VAR:-default}` syntax in config values for all formats (TOML, JSON, YAML); add interpolation pass after parsing, before type conversion
- [x] **Telemetry: Real TelemetryService Implementation** — Create `ProductionTelemetry` struct wrapping `EventBus` + `MetricsRegistry` + configurable `Recorder`; support builder pattern for selecting backends; wire shutdown signal for graceful flush
- [x] **Telemetry: Buffered Recorder** — Implement `BufferedRecorder` with channel-based buffering (`tokio::sync::mpsc`); background flush task; configurable buffer size and flush interval; graceful drain on shutdown
- [x] **Telemetry: Trace Context Propagation** — Add `TraceId`, `SpanId`, `ParentSpanId` fields to `ExecutionEvent` and `CallRecord`; implement W3C TraceContext (`traceparent` header) generation and parsing; enable cross-service correlation
- [x] **Metrics: Full Prometheus Export** — Extend `snapshot()` to include all domains (Resource: 13 metrics, EventBus: 4 metrics) dynamically from the registry instead of hardcoded arrays; add per-bucket histogram rendering with configurable bucket boundaries
- [x] **Metrics: Prometheus as Default Feature** — Move `prometheus` from optional to default feature; remove dead `metrics`/`metrics-exporter-prometheus` dependencies if not actually used in code
- [x] **Log: Dynamic Log Level Reconfiguration** — Support runtime log level changes without restart via `reload::Handle`; expose API to change level per-module; integrate with `nebula-config` hot-reload watcher
- [x] **Log: Async File Writer** — Add non-blocking file writer backend using `tracing-appender::non_blocking` to prevent Tokio runtime thread blocking on I/O; add configuration option to select sync vs async mode

### Tier 3 — Quality & Coverage (Tests, Docs, Benchmarks)

- [ ] **EventBus: Integration Test Suite** — Add `tests/` directory with multi-bus registry tests, concurrent producer/consumer scenarios, subscriber unsubscribe lifecycle, back-pressure policy combinations, and graceful shutdown propagation
- [ ] **EventBus: Subscriber Documentation** — Document what happens with slow/disconnected subscribers (lagging, recovery, buffer overflow); add architecture decision record for why persistence is Phase 3; add examples for common patterns
- [ ] **Telemetry: Trace Module Tests** — Add unit tests for `Recorder`, `ResourceUsageRecord`, `CallRecord`, `CallBody`, `CallPayload`, `CallStatus`; cover edge cases (empty payloads, redacted bodies, error states, Duration::ZERO)
- [ ] **Telemetry: Concurrent Stress Tests** — Add high-throughput concurrent subscriber tests; verify no data races with 100+ concurrent emitters and 50+ subscribers; verify histogram thread safety under contention
- [ ] **Metrics: Resource Domain Tests** — Add tests verifying all 13 resource metric constants are accessible through `TelemetryAdapter`; test `record_eventbus_stats()` edge cases (NaN, u64::MAX overflow, zero values)
- [ ] **System: Platform Documentation** — Document per-module platform support matrix (Linux/macOS/Windows); document that `ProcessInfo.cmd` is intentionally empty, `environ` is skipped for perf, `network::connections()` returns empty; add workarounds
- [ ] **System: Integration Tests** — Add platform-gated integration tests for memory pressure detection, CPU info retrieval, disk stats; use `#[cfg(target_os)]` to skip OS-specific tests on other platforms
- [ ] **Config: Loader Edge Case Tests** — Add tests for deeply nested TOML, Unicode keys, very large config files (>1MB), circular includes (if supported), symlink following in file watcher
- [ ] **Log: Telemetry Integration Examples** — Add documented examples showing `nebula-log` + `nebula-telemetry` + OpenTelemetry OTLP setup; include Sentry configuration example; document when to enable each feature flag

### Tier 4 — Production Hardening (Advanced Features)

- [ ] **EventBus: Graceful Shutdown** — Propagate shutdown signal to all active subscribers; allow final drain of in-flight events before closing channels; integrate with `tokio_util::sync::CancellationToken`
- [ ] **EventBus: Dead Letter Queue** — Track consistently dropped events with metadata (timestamp, drop reason, subscriber info); expose DLQ as queryable buffer for debugging; integrate with metrics (counter for DLQ entries)
- [ ] **Telemetry: OTLP Exporter** — Implement OpenTelemetry Protocol exporter over gRPC (`opentelemetry-otlp`); convert `CallRecord` → OTEL Span, `Counter/Gauge/Histogram` → OTEL Metrics; support configurable collector endpoint
- [ ] **Telemetry: Sampling Policy** — Add configurable trace sampling (always-on, probability-based, rate-limited, parent-based); allow per-workflow sampling overrides; integrate sampling decision into `ExecutionEvent`
- [ ] **Metrics: OTLP Metrics Export** — Add OpenTelemetry metrics push exporter alongside Prometheus pull; support periodic export interval; convert internal `Counter/Gauge/Histogram` to OTEL metric types
- [ ] **Metrics: Cardinality Guardrails** — Warn on high-cardinality label combinations (>1000 unique series); optionally drop metrics exceeding cardinality threshold; log warnings to `nebula-log`
- [ ] **Log: JSON Lines Format** — Add `logfmt` output mode for streaming JSON Lines (one JSON object per line); suitable for log aggregation pipelines (Fluentd, Vector, Loki)
- [ ] **Log: Rotation Lifecycle Hooks** — Add callback hooks for log file rotation events (before-delete, after-rotate); enable archive/S3 upload of rotated files before cleanup
- [ ] **Config: Remote Config Source** — Add HTTP/HTTPS config loader behind `remote` feature flag; support polling interval, authentication headers, ETag-based caching; integrate with hot-reload watcher
- [ ] **System: Network Connections** — Implement `connections()` using `netstat2` or equivalent crate; populate IP addresses and connection states; gate behind `network` feature flag

### Tier 5 — Cross-Crate Integration & Polish

- [ ] **Unified Observability Pipeline** — End-to-end integration test: config loads → log initializes → telemetry starts → metrics record → eventbus emits → Prometheus/OTLP exports; verify full data flow across all 7 crates
- [ ] **Config-Driven Telemetry Bootstrap** — `nebula-config` settings drive telemetry backend selection (Noop vs Production vs OTLP), log level, metrics export interval, eventbus capacity; single config file controls all cross-cutting behavior
- [ ] **EventBus Metrics Integration** — Automatic `TelemetryAdapter::record_eventbus_stats()` on configurable interval; wire eventbus statistics into Prometheus/OTLP export pipeline without manual polling
- [ ] **All Crates: cargo doc Clean Build** — Verify `cargo doc --no-deps` produces zero warnings across all 7 crates; fix any missing doc links, broken intra-doc references, or undocumented public items
- [ ] **All Crates: Benchmark Suite** — Add Criterion benchmarks for hot paths: config lookup, log formatting, eventbus emit, metrics increment, histogram observe; establish baseline for regression detection in CI

---

## Completed

| Milestone | Date |
|-----------|------|
| Resilience: Circuit Breaker, Retry, Bulkhead, Rate Limiter, Hedge, Fallback, Timeout — 9 patterns with 100+ tests and 12 benchmarks | 2026-03-11 |
| Config: Multi-loader (File/Env/Composite), hot-reload watchers, validation, builder API, 26+ tests | 2026-03-11 |
| Log: tracing-based structured logging, 4 formats (pretty/compact/json/logfmt), rolling file writer, presets, 20+ tests | 2026-03-11 |
| System: Memory pressure detection, CPU/disk/network modules, cross-platform abstractions | 2026-03-11 |
| EventBus: Broadcast transport, scoped subscriptions, filtered subscribers, back-pressure policies, 19 tests, 2 benchmarks | 2026-03-11 |
| Metrics: 48 naming constants, TelemetryAdapter, PrometheusExporter (snapshot), EventBusStats recording | 2026-03-11 |
| Telemetry: ExecutionEvent (8 variants), EventBus wrapper, MetricsRegistry (Counter/Gauge/Histogram), NoopTelemetry, Recorder trait, 17 tests | 2026-03-11 |
| EventBus: Lock poisoning recovery — 8 `.expect()` calls replaced with `unwrap_or_else` + `tracing::warn` | 2026-03-12 |
| Telemetry: Histogram bounded bucket storage — Vec replaced with Prometheus-style atomic buckets, `percentile()` added | 2026-03-12 |
| EventBus, Telemetry, Metrics: Prelude modules added | 2026-03-12 |
| Config: YAML loader (`serde_yaml` behind `yaml` feature), env var interpolation `${VAR}`/`${VAR:-default}`, integration tests | 2026-03-12 |
| Telemetry: `ProductionTelemetry` service + `BufferedRecorder` (mpsc-backed), W3C `TraceContext` propagation in all `ExecutionEvent` variants | 2026-03-12 |
| Metrics: Full Prometheus text-format export — `# HELP`/`# TYPE` metadata, per-bucket `_bucket{le=...}` lines, `_count`/`_sum`; removed unused dev-dependencies | 2026-03-12 |
| Log: `ReloadHandle` exposed on `LoggerGuard`, `WatcherGuard`/`watch_config()` for polling-based config file watching (behind `async` feature), `WriterConfig::non_blocking` documented | 2026-03-12 |

---

## See Also

- `/aif-plan <milestone>` — create branch + detailed task plan for a milestone
- `/aif-implement` — execute the plan
- `docs/ROADMAP.md` — project-wide roadmap (all layers, not just cross-cutting)
