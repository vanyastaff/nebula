# Roadmap

## Alignment with Other Roadmaps

- **Telemetry:** [docs/crates/telemetry/ROADMAP.md](../telemetry/ROADMAP.md) — Phase 2 adds bounded Histogram and `nebula_*` naming; Phase 3 adds Prometheus/OTLP. This metrics ROADMAP defers export design to that decision (extend telemetry vs new crate).
- **Eventbus:** [docs/crates/eventbus/ROADMAP.md](../eventbus/ROADMAP.md) — Phase 3 integrates EventBusStats with metrics. Unified export (Phase 3/4 here) will consume eventbus stats when available.

---

## Phase 1: Documentation and Alignment (Current)

- **Deliverables:**
  - Metrics documentation complete (this doc set)
  - Document current state: telemetry, log, domain crates
  - Define target: unified export, naming convention
- **Risks:** None
- **Exit criteria:** Docs complete; telemetry ROADMAP aligned

**Phase 1 checklist:**
- [x] Metrics doc set complete (README, ARCHITECTURE, API, INTERACTIONS, DECISIONS, PROPOSALS, ROADMAP, SECURITY, RELIABILITY, TEST_STRATEGY, MIGRATION)
- [x] [CURRENT_STATE.md](./CURRENT_STATE.md) — current state of telemetry, log, domain crates
- [x] [TARGET.md](./TARGET.md) — unified export and naming convention (`nebula_*`)
- [x] ROADMAP aligned with telemetry and eventbus ROADMAPs (cross-links above)

## Phase 2: Naming and Adapters

- **Deliverables:**
  - Standard metric naming convention (`nebula_*` prefix)
  - Document metric names for workflow, action, node, credential, resource
  - Adapter design for telemetry → export
- **Risks:** Naming may conflict with existing usage
- **Exit criteria:** Naming doc; adapter design; no breaking changes to telemetry

**Phase 2 status:** Done. `nebula-metrics` provides naming constants and `TelemetryAdapter`; engine and runtime record under `nebula_*` names (workflow, action); TARGET.md documents names; telemetry unchanged.

## Phase 3: Export Implementation

- **Deliverables:**
  - Prometheus exporter (in telemetry or new metrics crate)
  - `/metrics` endpoint (via api or standalone)
  - Bounded Histogram (telemetry)
- **Risks:** Crate extraction vs telemetry extension decision
- **Exit criteria:** Prometheus scrape working; Grafana dashboard

**Phase 3 status:** Prometheus text export implemented in `nebula-metrics`: `snapshot(registry)`, `PrometheusExporter`, `content_type()`. Engine/runtime use `nebula_*` names; scraping the snapshot yields valid Prometheus format. `/metrics` HTTP endpoint is for api/desktop to wire (e.g. GET handler that returns `exporter.snapshot()`). Bounded Histogram remains in telemetry ROADMAP.

## Phase 4: OTLP and Unification

- **Deliverables:**
  - OTLP push (optional feature)
  - Unified registry or adapter for domain crates
  - Standard dashboards and alert rules
- **Risks:** Domain crate integration complexity
- **Exit criteria:** OTLP push; domain metrics exported; runbook

## Metrics of Readiness

| Metric | Target |
|--------|--------|
| Correctness | Prometheus format valid; metric values accurate |
| Latency | Scrape < 100ms; no impact on recording |
| Throughput | 10k metrics/sec recording; scrape handles all |
| Stability | Export failures do not affect execution |
| Operability | Dashboards; alert rules; runbook |
