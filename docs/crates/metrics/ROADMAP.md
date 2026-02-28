# Roadmap

## Phase 1: Documentation and Alignment (Current)

- **Deliverables:**
  - Metrics documentation complete (this doc set)
  - Document current state: telemetry, log, domain crates
  - Define target: unified export, naming convention
- **Risks:** None
- **Exit criteria:** Docs complete; telemetry ROADMAP aligned

## Phase 2: Naming and Adapters

- **Deliverables:**
  - Standard metric naming convention (`nebula_*` prefix)
  - Document metric names for workflow, action, node, credential, resource
  - Adapter design for telemetry → export
- **Risks:** Naming may conflict with existing usage
- **Exit criteria:** Naming doc; adapter design; no breaking changes to telemetry

## Phase 3: Export Implementation

- **Deliverables:**
  - Prometheus exporter (in telemetry or new metrics crate)
  - `/metrics` endpoint (via api or standalone)
  - Bounded Histogram (telemetry)
- **Risks:** Crate extraction vs telemetry extension decision
- **Exit criteria:** Prometheus scrape working; Grafana dashboard

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
