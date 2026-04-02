# Current State

This document captures the **current** metrics and observability state across telemetry, log, and domain crates as of the metrics ROADMAP Phase 1. It serves as the baseline for alignment and Phase 2+ work.

## Summary

| Area | Implementation | Export | Naming |
|------|-----------------|--------|--------|
| **nebula-metrics** | Naming constants, `TelemetryAdapter` over telemetry | Stub (Phase 3) | `nebula_*` constants + typed accessors |
| **Telemetry** | In-memory Counter, Gauge, Histogram, MetricsRegistry | None | Mixed (some `*_total`, no `nebula_*` prefix) |
| **Log** | Standard `metrics` crate (feature `observability`) | Optional Prometheus | Caller-defined |
| **Engine/Runtime** | Use telemetry MetricsRegistry | None | See table below |
| **Resource** | Standard `metrics` crate (feature `metrics`) + `nebula-metrics` naming | Via metrics crate | `nebula_resource_*` (from nebula-metrics) |
| **Credential** | Domain structs (RotationMetrics, etc.) | None | Internal |
| **Resilience** | Observability hooks, typed metrics | Hook-based | Custom |
| **Memory** | Domain structs (MemoryMetrics, cache stats) | None | Internal |

---

## 0. nebula-metrics

**Location:** `crates/metrics/`

**APIs:**
- `naming` — constants: `NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL`, `NEBULA_ACTION_DURATION_SECONDS`, etc.
- `TelemetryAdapter` — wraps `Arc<MetricsRegistry>`, exposes typed accessors (e.g. `workflow_executions_started_total()`, `action_duration_seconds()`) that use standard names
- Re-exports: `Counter`, `Gauge`, `Histogram`, `MetricsRegistry` from telemetry
- `export` — stub; Prometheus exporter behind `prometheus` feature (Phase 3)

**Export:** None yet; `prometheus` feature adds optional deps and empty stub.

**Consumers:** Engine/runtime can switch to `TelemetryAdapter` to record under `nebula_*` names without changing telemetry.

---

## 1. nebula-telemetry

**Location:** `crates/telemetry/src/metrics.rs`, `lib.rs`

**APIs:**
- `MetricsRegistry` — create/get counters, gauges, histograms by name
- `Counter`, `Gauge`, `Histogram` — in-memory, atomic (Counter/Gauge) or `Vec<f64>` (Histogram)
- `NoopMetricsRegistry` — no-op for tests

**Export:** None. Metrics stay in process; no Prometheus or OTLP.

**Consumers:** `nebula-engine`, `nebula-runtime` (both take `Arc<MetricsRegistry>`).

**Metric names in use (engine + runtime):**

| Name | Type | Crate | Notes |
|------|------|-------|--------|
| `executions_started_total` | Counter | engine | |
| `executions_completed_total` | Counter | engine | |
| `executions_failed_total` | Counter | engine | |
| `execution_duration_seconds` | Histogram | engine | |
| `actions_executed_total` | Counter | runtime | |
| `actions_failed_total` | Counter | runtime | |
| `action_duration_seconds` | Histogram | runtime | |

**Risks:** Histogram stores all observations in memory (unbounded). No `nebula_` prefix yet.

**Docs:** Crate lib.rs recommends `nebula_` prefix for future export; see [telemetry ROADMAP](../telemetry/ROADMAP.md).

---

## 2. nebula-log

**Location:** `crates/log/src/metrics/mod.rs`, `helpers.rs`

**APIs (feature `observability`):**
- Re-exports: `metrics::counter!`, `gauge!`, `histogram!`, `describe_*`, `Recorder`, etc.
- `timed_block(name, F)`, `timed_block_async(name, F)` — record duration as histogram
- `TimingGuard` — RAII timer recording to histogram on drop

**Export:** Optional Prometheus exporter (if present in dependency tree / feature).

**Naming:** Metric names are caller-defined; no enforced convention.

**Use case:** General observability and timing; not workflow-specific.

---

## 3. nebula-engine and nebula-runtime

**Role:** Primary consumers of `nebula-telemetry::MetricsRegistry`.

**Engine** (`crates/engine/src/engine.rs`):
- `executions_started_total` — inc when execution starts
- `executions_completed_total` — inc on success
- `executions_failed_total` — inc on failure
- `execution_duration_seconds` — observe(elapsed)

**Runtime** (`crates/runtime/src/runtime.rs`):
- `actions_executed_total` — inc per action run
- `actions_failed_total` — inc on action error
- `action_duration_seconds` — observe(duration)

No export; all data lives in the in-memory registry.

---

## 4. nebula-resource

**Location:** `crates/resource/src/metrics.rs` (feature `metrics`)

**Implementation:** Subscribes to `EventBus<ResourceEvent>`; uses **standard `metrics` crate** with **metric names from `nebula-metrics`** (`NEBULA_RESOURCE_*` constants) to record:

| Metric (constant) | Type | Labels |
|-------------------|------|--------|
| `nebula_resource_create_total` | Counter | resource_id |
| `nebula_resource_acquire_total` | Counter | resource_id |
| `nebula_resource_acquire_wait_duration_seconds` | Histogram | resource_id |
| `nebula_resource_release_total` | Counter | resource_id |
| `nebula_resource_usage_duration_seconds` | Histogram | resource_id |
| `nebula_resource_cleanup_total` | Counter | resource_id |
| `nebula_resource_error_total` | Counter | resource_id |
| `nebula_resource_health_state` | Gauge | resource_id |
| `nebula_resource_pool_exhausted_total` | Counter | resource_id |
| `nebula_resource_pool_waiters` | Gauge | resource_id |
| `nebula_resource_quarantine_total` | Counter | resource_id |
| `nebula_resource_quarantine_released_total` | Counter | resource_id |
| `nebula_resource_config_reloaded_total` | Counter | resource_id |

**Export:** When a `metrics` crate recorder is installed (e.g. Prometheus), resource metrics are exported with the unified `nebula_resource_*` names. Feature `metrics` pulls in `nebula-metrics` for naming.

---

## 5. nebula-credential

**Location:** `crates/credential/src/rotation/metrics.rs`, storage/provider metrics

**Implementation:** Domain-specific structs (e.g. `RotationMetrics`, `CredentialMetrics`) with in-memory counters, durations, success/failure counts. Not the standard `metrics` crate or telemetry registry.

**Export:** None. Metrics are for in-process use (e.g. health, success rate).

**Naming:** Internal types; no Prometheus-style names yet.

---

## 6. nebula-resilience

**Location:** `crates/resilience/src/observability/` (hooks, metrics)

**Implementation:** Observability hooks with typed metrics (`Metric<N>`, `metrics::counter`, `service_gauge`, `operation_histogram`). Can emit to a collector; log integration for events.

**Export:** Hook-based; no built-in Prometheus/OTLP. `export_metrics()` on hooks.

**Naming:** Custom (e.g. `request_duration`, `connections` with service/operation labels).

---

## 7. nebula-memory

**Location:** `crates/memory/src/cache/stats.rs`, `memory_stats.rs`, `pool/health.rs`, etc.

**Implementation:** Domain structs: `AtomicCacheStats`, `MemoryMetrics`, `HealthMetrics`, `PoolStats` — counters and gauges for cache hits/misses, allocation, pool utilization.

**Export:** None. Used for in-process monitoring and tests.

---

## Alignment with Other Roadmaps

- **Telemetry ROADMAP:** Phase 2 adds bounded Histogram and documents `nebula_*` naming; Phase 3 adds Prometheus/OTLP export. Metrics ROADMAP Phase 1 exit criteria require telemetry ROADMAP alignment (see [ROADMAP.md](./ROADMAP.md)).
- **Eventbus ROADMAP:** Phase 3 delivers EventBusStats integration with metrics. Resource and telemetry event flows may feed the same export later.

---

## Gaps (for Phase 2+)

1. **Naming:** No single convention; engine/runtime use `*_total`/`*_seconds` but not `nebula_*`.
2. **Export:** Telemetry metrics are not scrapeable; resource uses `metrics` crate but export is optional and separate.
3. **Unification:** Two worlds — telemetry in-memory registry vs standard `metrics` crate (log, resource). Adapter or single backend needed for one scrape endpoint.
4. **Bounded histograms:** Telemetry Histogram is unbounded; Phase 2 (telemetry) / Phase 3 (metrics) will address this.
