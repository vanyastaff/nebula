# Interactions

## Ecosystem Map (Current + Planned)

### Existing Crates

| Crate | Relationship | Description |
|-------|-------------|-------------|
| `nebula-telemetry` | Upstream | In-memory Counter, Gauge, Histogram, MetricsRegistry; engine/runtime consumers |
| `nebula-log` | Upstream | Standard `metrics` crate; observability feature; Prometheus exporter (optional) |
| `nebula-engine` | Indirect | Uses telemetry MetricsRegistry |
| `nebula-runtime` | Indirect | Uses telemetry MetricsRegistry |
| `nebula-memory` | Sibling | Domain metrics (MemoryMetric, extensions) |
| `nebula-credential` | Sibling | StorageMetrics |
| `nebula-resource` | Sibling | Resource metrics |
| `nebula-resilience` | Sibling | Circuit breaker, retry metrics |

### Planned Crates

- **nebula-metrics:** Standalone metrics crate or telemetry Phase 3 export module
  - Centralizes Prometheus/OTLP export
  - Standard naming convention
  - Adapter for telemetry, log, domain crates

## Downstream Consumers (Target)

- **nebula-api:** Expose `/metrics` for Prometheus scrape
- **nebula-app:** Dashboard metrics display
- **Grafana/Prometheus:** External monitoring stack

## Upstream Dependencies

| Crate | Why needed | Hard contract | Fallback |
|-------|------------|---------------|----------|
| `nebula-telemetry` | In-memory primitives | MetricsRegistry, Counter, Gauge, Histogram | Adapter layer |
| `nebula-log` | Standard metrics | `metrics` crate macros | Optional |
| `prometheus` / `metrics-exporter-prometheus` | Export | Prometheus format | Feature-gated |
| `opentelemetry` | OTLP | OTLP spec | Feature-gated |

## Interaction Matrix

| This crate <-> Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|---------------------|-----------|----------|------------|------------------|-------|
| metrics -> telemetry | out | Adapter over MetricsRegistry | sync | N/A | Read telemetry metrics for export |
| metrics -> log | out | Optional metrics crate integration | sync | N/A | Re-export or bridge |
| metrics -> api | out | `/metrics` endpoint | async | Non-blocking | Scrape endpoint |
| api -> metrics | in | HTTP GET /metrics | async | Return 500 on export failure | Prometheus scrape |

## Runtime Sequence (Target)

1. Application starts; metrics registry initialized (or from telemetry).
2. Engine, runtime, domain crates record metrics.
3. Prometheus scrapes `/metrics` periodically; or OTLP push to collector.
4. Grafana/alerting consumes metrics.

## Cross-Crate Ownership

| Responsibility | Owner |
|----------------|-------|
| In-memory primitives | `nebula-telemetry` |
| Export (Prometheus, OTLP) | `nebula-metrics` (planned) or telemetry |
| Metric naming convention | `nebula-metrics` (planned) |
| Domain-specific metrics | Each crate (memory, credential, resource, resilience) |
| Scrape endpoint | `nebula-api` (when metrics crate exists) |

## Failure Propagation

- **How failures bubble up:** Export failures (e.g. OTLP push) should not affect recording; best-effort export.
- **Where retries apply:** OTLP push; configurable backoff.
- **Where retries forbidden:** Recording must never block hot path.

## Versioning and Compatibility

- **Compatibility promise:** Metric names stable; Prometheus format compatible.
- **Breaking-change protocol:** Major version bump; migration guide.
- **Deprecation window:** 2 minor releases for metric name changes.

## Contract Tests Needed

- [ ] Prometheus scrape returns valid format
- [ ] Metric names follow `nebula_*` convention
- [ ] Export does not block recording
- [ ] Telemetry adapter correctly reads metrics
