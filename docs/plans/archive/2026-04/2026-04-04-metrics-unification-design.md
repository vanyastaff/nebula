# Metrics Unification Design

**Date:** 2026-04-04
**Status:** Approved
**Scope:** nebula-metrics, nebula-telemetry, nebula-log, domain crates (resource, credential, memory)

---

## Problem

Nebula has three independent metrics systems that don't talk to each other:

1. **nebula-telemetry -> nebula-metrics** — lock-free registry + Prometheus export (used by engine, runtime)
2. **Custom atomic structs** — `ResourceMetrics` in resource, `RotationMetrics` in credential, `AtomicCacheStats` in memory — completely isolated, never exported
3. **Standard `metrics` crate** — in nebula-log behind `observability` feature — parallel universe

This creates cognitive overhead, duplicated logic (e.g., manual O(n log n) percentile in credential vs O(1) bounded Histogram in telemetry), and no unified export path for operational monitoring.

---

## Decisions

| # | Decision | Choice | Rationale |
|---|----------|--------|-----------|
| 1 | End goal | Unified path: telemetry registry -> metrics export | Eliminates duplication, single export endpoint |
| 2 | Macros for naming.rs | No | YAGNI — 3 match blocks of ~15 arms each, not 300-line monsters |
| 3 | How domain crates record | `Arc<MetricsRegistry>` via DI, constants from `nebula_metrics::naming` | Minimal deps, metrics stays export layer |
| 4 | Optional metrics | `Option<Arc<MetricsRegistry>>` in domain structs | Avoids feature flag combinatorial explosion in CI |
| 5 | Separate naming crate | No | nebula-metrics is cross-cutting, incremental compile cost ~0 |
| 6 | TTL / lifecycle | Centralized `retain_recent()` only | Domain crates don't manage metric lifecycle |
| 7 | nebula-log metrics | Remove entirely (feature `observability` + `metrics` crate dep) | SRP — logger logs, metrics crate measures |

---

## Target Architecture

```
                    ┌─────────────────────────┐
                    │     Prometheus / OTLP    │
                    └────────────▲────────────┘
                                 │ snapshot() / push
                    ┌────────────┴────────────┐
                    │     nebula-metrics       │
                    │  naming.rs  (constants)  │
                    │  filter.rs  (allowlist)  │
                    │  export/    (prometheus) │
                    │  adapter.rs (typed DX)   │
                    └────────────▲────────────┘
                                 │ depends on
                    ┌────────────┴────────────┐
                    │    nebula-telemetry      │
                    │  MetricsRegistry         │
                    │  Counter/Gauge/Histogram │
                    │  LabelInterner/LabelSet  │
                    │  retain_recent (TTL)     │
                    └────────────▲────────────┘
                                 │ Arc<MetricsRegistry> via DI
          ┌──────────┬───────────┼───────────┬──────────┐
          │          │           │           │          │
     ┌────┴───┐ ┌───┴────┐ ┌───┴────┐ ┌───┴─────┐ ┌──┴───┐
     │ engine │ │runtime │ │resource│ │credential│ │memory│
     └────────┘ └────────┘ └────────┘ └──────────┘ └──────┘
```

### Data flow

1. Domain crate receives `Option<Arc<MetricsRegistry>>` via constructor
2. Builds internal `XxxMetrics` struct (or `None` if registry absent)
3. Records via `registry.counter(NEBULA_...).inc()` on hot path (lock-free atomic)
4. nebula-metrics `PrometheusExporter` iterates registry via `snapshot_*` APIs
5. API layer serves `GET /metrics` with Prometheus text format
6. `retain_recent()` called periodically from engine/api (e.g., every 5 min)

### Cardinality protection (two lines of defense)

- **Static (write-time):** `LabelAllowlist` strips high-cardinality keys before recording
- **Dynamic (maintenance):** `retain_recent(Duration)` evicts stale labeled series by TTL

---

## Domain Crate Pattern

Each domain crate that records metrics follows this pattern:

```rust
// crates/resource/src/metrics.rs (internal, pub(crate))
use nebula_metrics::naming::*;
use nebula_telemetry::metrics::{Counter, Gauge, Histogram, MetricsRegistry};

pub(crate) struct ResourcePoolMetrics {
    create_total: Counter,
    acquire_total: Counter,
    acquire_wait: Histogram,
    release_total: Counter,
    error_total: Counter,
    health_state: Gauge,
}

impl ResourcePoolMetrics {
    pub fn new(registry: &MetricsRegistry) -> Self {
        Self {
            create_total: registry.counter(NEBULA_RESOURCE_CREATE_TOTAL),
            acquire_total: registry.counter(NEBULA_RESOURCE_ACQUIRE_TOTAL),
            acquire_wait: registry.histogram(NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS),
            release_total: registry.counter(NEBULA_RESOURCE_RELEASE_TOTAL),
            error_total: registry.counter(NEBULA_RESOURCE_ERROR_TOTAL),
            health_state: registry.gauge(NEBULA_RESOURCE_HEALTH_STATE),
        }
    }

    #[inline]
    pub fn inc_create(&self) {
        self.create_total.inc();
    }

    #[inline]
    pub fn record_acquire_wait(&self, seconds: f64) {
        self.acquire_wait.observe(seconds);
    }
    // ... etc
}
```

Owner struct holds `Option<ResourcePoolMetrics>`:

```rust
pub struct ResourcePool {
    metrics: Option<ResourcePoolMetrics>,
    // ...
}

impl ResourcePool {
    pub fn new(/* ... */, registry: Option<&MetricsRegistry>) -> Self {
        Self {
            metrics: registry.map(ResourcePoolMetrics::new),
            // ...
        }
    }

    fn on_acquire(&self) {
        if let Some(m) = &self.metrics {
            m.inc_acquire();
        }
    }
}
```

---

## What Gets Removed

### From nebula-resource
- `ResourceMetrics` struct (custom atomic counters) — replaced by `ResourcePoolMetrics` using telemetry registry
- `MetricsSnapshot` — no longer needed, registry snapshots handle export

### From nebula-credential
- `RotationMetrics` struct (RwLock + manual percentile) — replaced by telemetry `Histogram` with O(1) bounded buckets
- `CredentialMetrics` per-credential tracking — replaced by labeled metrics in registry
- 90-day auto-cleanup — replaced by centralized `retain_recent()`

### From the archived memory crate
- `AtomicCacheStats` — replaced by counters/gauges in registry (hit/miss/eviction)
- `MemoryMetrics` / `BudgetMetrics` — evaluate case-by-case; some may stay as internal snapshots if they serve a purpose beyond monitoring

### From nebula-log
- Feature `observability` — removed entirely
- Dependency on `metrics` crate (ecosystem) — removed
- `timed_block()` / `timed_block_async()` — migrate to nebula-telemetry or nebula-metrics as utility
- `TimingGuard` RAII — migrate with `timed_block`
- `MetricsHook` — removed (no longer a separate recording path)

### From nebula-metrics
- `record_eventbus_stats()` bridge method — eventually replaced by EventBus recording directly to registry (when eventbus depends on telemetry)
- Legacy naming constants — removed after migration window

---

## New Naming Constants Needed

For credential domain (add to `naming.rs`):

```
NEBULA_CREDENTIAL_ROTATIONS_TOTAL           (counter)
NEBULA_CREDENTIAL_ROTATION_FAILURES_TOTAL   (counter)
NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS (histogram)
NEBULA_CREDENTIAL_ACTIVE_TOTAL              (gauge)
NEBULA_CREDENTIAL_EXPIRED_TOTAL             (counter)
```

For memory/cache domain:

```
NEBULA_CACHE_HITS_TOTAL                     (counter)
NEBULA_CACHE_MISSES_TOTAL                   (counter)
NEBULA_CACHE_EVICTIONS_TOTAL                (counter)
NEBULA_CACHE_SIZE                           (gauge)
```

Exact names to be finalized during implementation based on actual domain needs.

---

## TelemetryAdapter Role (Post-Unification)

`TelemetryAdapter` remains as a convenience wrapper for engine/runtime where the number of standard metrics is large and typed accessors improve DX. It is NOT required — domain crates use `MetricsRegistry` directly.

Post-unification `TelemetryAdapter` provides:
- Typed workflow/action accessors (existing)
- `LabelAllowlist` integration (existing)
- Generic `.counter(name)` / `.gauge(name)` / `.histogram(name)` fallback (existing)

It does NOT grow to include every domain's metrics — that would make it a God Object.

---

## Migration Order

### Phase 1: Foundation
1. Add credential + cache naming constants to `naming.rs`
2. Add HELP text to prometheus.rs match blocks
3. Migrate `timed_block` utilities from nebula-log to nebula-telemetry

### Phase 2: Domain Migration
4. nebula-resource: replace `ResourceMetrics` with registry-based `ResourcePoolMetrics`
5. nebula-credential: replace `RotationMetrics` with registry-based recording
6. archived memory crate: replace `AtomicCacheStats` with registry counters

### Phase 3: Cleanup
7. nebula-log: remove `observability` feature, `metrics` crate dependency
8. nebula-metrics: remove legacy naming constants (after migration window)
9. nebula-metrics: remove `record_eventbus_stats()` if eventbus records directly

### Phase 4: Wiring
10. Wire `GET /metrics` endpoint in nebula-api
11. Periodic `retain_recent()` call from engine/api layer
12. Grafana dashboard templates

---

## Risks

| Risk | Mitigation |
|------|------------|
| Domain crates gain telemetry dependency | telemetry is cross-cutting, minimal footprint (atomics + dashmap) |
| Option check on every metric write | Single branch predict; counters are fire-and-forget atomics |
| Breaking credential/resource internal APIs | Internal structs (`pub(crate)`), no external consumers |
| timed_block migration breaks downstream | Feature-flag transition period if needed |
| Naming constant typos | Constants are `&str` checked at compile time via imports |

---

## Non-Goals

- OTLP push exporter (Phase 4 of metrics roadmap — separate effort)
- Distributed tracing integration (separate concern, handled by telemetry spans)
- Dashboard/alert definitions (operational, not architectural)
- Custom bucket boundaries per domain (default 11 buckets sufficient for now)
