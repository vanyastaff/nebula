# Implementation Plan: nebula-telemetry

**Crate**: `nebula-telemetry` | **Path**: `crates/telemetry` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

Event bus, metrics registry, and telemetry service for the Nebula workflow engine. Provides ExecutionEvent emission, MetricsRegistry for counters/gauges/histograms, and a TelemetryService trait. Current focus is Phase 1 (Contract and Safety Baseline) -- stabilizing the core types and engine integration.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio
**Key Dependencies**: nebula-core, nebula-eventbus, async-trait, chrono, serde/serde_json, tracing
**Testing**: `cargo test -p nebula-telemetry`

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 1: Contract and Safety Baseline | In Progress | Stabilize EventBus, ExecutionEvent, MetricsRegistry, NoopTelemetry |
| Phase 2: Runtime Hardening | Planned | Bounded Histogram, naming convention, event versioning |
| Phase 3: Export and Production Readiness | Planned | Prometheus exporter, OTLP export, feature-gated |
| Phase 4: Ecosystem and DX | Planned | Grafana dashboards, alert rules, trace correlation |

## Phase Details

### Phase 1: Contract and Safety Baseline

**Goal**: Stable telemetry contracts with engine/runtime integration and no panics in hot paths.

**Deliverables**:
- EventBus, ExecutionEvent, MetricsRegistry stable APIs
- Engine and runtime integration complete
- NoopTelemetry for testing/MVP
- Unit and integration tests

**Exit Criteria**:
- All tests pass; no panics in hot path; docs complete

**Risks**:
- Histogram memory growth under load

**Dependencies**: nebula-core, nebula-eventbus

### Phase 2: Runtime Hardening

**Goal**: Bounded memory usage and standardized naming across all metrics.

**Deliverables**:
- Bounded/bucketed Histogram (or replacement)
- Document metric naming convention (`nebula_*` prefix)
- Event schema versioning (if needed)

**Exit Criteria**:
- Histogram bounded; no unbounded memory growth; backward compat or migration path

**Risks**:
- Histogram API change may affect consumers

**Dependencies**: None beyond Phase 1

### Phase 3: Export and Production Readiness

**Goal**: Metrics scrapeable by Prometheus and pushable via OTLP.

**Deliverables**:
- Prometheus exporter (optional feature)
- OTLP metrics export (optional feature)
- `PrometheusTelemetry` or similar implementing `TelemetryService`

**Exit Criteria**:
- Metrics scrapeable by Prometheus; OTLP push working; feature-gated

**Risks**:
- Export adds dependencies; cardinality management

**Dependencies**: nebula-metrics (coordinates export format)

### Phase 4: Ecosystem and DX

**Goal**: Production deployment support with dashboards, alerts, and trace correlation.

**Deliverables**:
- Standard dashboard templates (Grafana)
- Alert rule examples
- Correlation with nebula-log traces (trace_id in events)

**Exit Criteria**:
- Docs and examples for production deployment

**Risks**:
- Dashboard maintenance; version skew

**Dependencies**: nebula-log (trace correlation)

## Inter-Crate Dependencies

- **Depends on**: nebula-core, nebula-eventbus
- **Depended by**: nebula-metrics (adapters/export), nebula-engine (execution events), nebula-runtime (metrics recording)

## Verification

- [ ] `cargo check -p nebula-telemetry`
- [ ] `cargo test -p nebula-telemetry`
- [ ] `cargo clippy -p nebula-telemetry -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-telemetry`
