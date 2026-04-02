# Implementation Plan: nebula-metrics

**Crate**: `nebula-metrics` | **Path**: `crates/metrics` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

The metrics crate provides standard metric naming constants (`nebula_*` prefix), a `TelemetryAdapter` for bridging domain crates to telemetry, and a Prometheus exporter. It complements `nebula-telemetry` — telemetry owns the registry and collection; metrics owns naming conventions and export format. Phases 1–3 are largely complete; current focus is Phase 4: OTLP push and unified registry across all domain crates.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio (for export)
**Key Dependencies**: `nebula-telemetry`, `prometheus` (Prometheus text format)
**Testing**: `cargo test -p nebula-metrics`

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 1: Documentation and Alignment | ✅ Done | Full doc set complete, aligned with telemetry ROADMAP |
| Phase 2: Naming and Adapters | ✅ Done | `nebula_*` naming constants, `TelemetryAdapter`, TARGET.md |
| Phase 3: Export Implementation | ✅ Done | Prometheus exporter (`PrometheusExporter`), `snapshot()`, content_type |
| Phase 4: OTLP and Unification | ⬜ Planned | OTLP push, unified registry, standard dashboards |

## Phase Details

### Phase 1: Documentation and Alignment ✅

**Deliverables** (complete):
- Complete metrics doc set (README, ARCHITECTURE, API, INTERACTIONS, DECISIONS, PROPOSALS, ROADMAP, SECURITY, RELIABILITY, TEST_STRATEGY, MIGRATION)
- CURRENT_STATE.md documenting current telemetry/log/domain state
- TARGET.md defining unified export and `nebula_*` naming convention

### Phase 2: Naming and Adapters ✅

**Deliverables** (complete):
- Standard metric naming convention (`nebula_*` prefix)
- `TelemetryAdapter` for telemetry → export bridge
- Engine and runtime record metrics under `nebula_*` names

### Phase 3: Export Implementation ✅

**Deliverables** (complete):
- Prometheus text export: `snapshot(registry)`, `PrometheusExporter`, `content_type()`
- Engine/runtime use `nebula_*` names; Prometheus format valid

**Remaining**:
- `/metrics` HTTP endpoint needs wiring in `nebula-api` or desktop

### Phase 4: OTLP and Unification

**Goal**: OTLP push feature; unified registry for domain crates; standard dashboards.

**Deliverables**:
- OTLP push (optional feature behind `otlp` flag)
- Unified registry or adapter for domain crates (credential, resource, worker)
- Standard dashboards and alert rules (Grafana JSON)

**Exit Criteria**:
- OTLP push sends metrics to collector; domain metrics exported; runbook documents configuration

**Risks**:
- Domain crate integration complexity; OTLP dependency size

## Inter-Crate Dependencies

- **Depends on**: `nebula-telemetry`
- **Depended by**: `nebula-api` (`/metrics` endpoint), `nebula-engine`, `nebula-runtime`, `nebula-worker`

## Verification

- [ ] `cargo check -p nebula-metrics --all-features`
- [ ] `cargo test -p nebula-metrics`
- [ ] `cargo clippy -p nebula-metrics -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-metrics`
