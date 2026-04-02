# Roadmap

## Phase 1: Contract and Safety Baseline (Current)

- **Deliverables:**
  - EventBus, ExecutionEvent, MetricsRegistry stable
  - Engine and runtime integration complete
  - NoopTelemetry for testing/MVP
  - Unit and integration tests
- **Risks:** Histogram memory growth under load
- **Exit criteria:** All tests pass; no panics in hot path; docs complete

## Phase 2: Runtime Hardening

- **Deliverables:**
  - Bounded/bucketed Histogram (or replacement)
  - Document metric naming convention (`nebula_*` prefix)
  - Event schema versioning (if needed)
- **Risks:** Histogram API change may affect consumers
- **Exit criteria:** Histogram bounded; no unbounded memory growth; backward compat or migration path

## Phase 3: Export and Production Readiness

- **Deliverables:**
  - Prometheus exporter (optional feature)
  - OTLP metrics export (optional feature)
  - `PrometheusTelemetry` or similar implementing `TelemetryService`
- **Risks:** Export adds dependencies; cardinality management
- **Exit criteria:** Metrics scrapeable by Prometheus; OTLP push working; feature-gated

## Phase 4: Ecosystem and DX

- **Deliverables:**
  - Standard dashboard templates (Grafana)
  - Alert rule examples
  - Correlation with nebula-log traces (trace_id in events?)
- **Risks:** Dashboard maintenance; version skew
- **Exit criteria:** Docs and examples for production deployment

## Metrics of Readiness

| Metric | Target |
|--------|--------|
| Correctness | All events emitted in correct order; metrics accurate |
| Latency | Emit < 1µs p99; metric record < 1µs p99 |
| Throughput | 10k events/sec without subscriber backpressure |
| Stability | No panics; no memory leak in Histogram (Phase 2) |
| Operability | Exporters documented; dashboards available (Phase 4) |
