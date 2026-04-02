# Tasks: nebula-telemetry

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix TEL

---

## Phase 1: Contract and Safety Baseline

**Goal**: Stable telemetry contracts with no panics in hot paths.

- [ ] TEL-T001 [P] Stabilize EventBus API surface (send, subscribe, unsubscribe)
- [ ] TEL-T002 [P] Stabilize ExecutionEvent enum (variants for node start, complete, error)
- [ ] TEL-T003 [P] Stabilize MetricsRegistry API (counter, gauge, histogram operations)
- [ ] TEL-T004 Implement NoopTelemetry for testing and MVP usage
- [ ] TEL-T005 Integrate telemetry with engine: emit ExecutionEvents on workflow/node lifecycle
- [ ] TEL-T006 Integrate telemetry with runtime: record metrics on task completion
- [ ] TEL-T007 Write unit tests for EventBus, MetricsRegistry, and NoopTelemetry
- [ ] TEL-T008 Write integration test verifying no panics under concurrent event emission

**Checkpoint**: All tests pass. Engine and runtime emit telemetry. No panics in hot path. API docs complete.

---

## Phase 2: Runtime Hardening

**Goal**: Bounded memory usage and standardized metric naming.

- [ ] TEL-T009 Replace unbounded Histogram with bounded/bucketed implementation
- [ ] TEL-T010 Document and enforce `nebula_*` metric naming convention
- [ ] TEL-T011 Evaluate need for event schema versioning; implement if warranted
- [ ] TEL-T012 Write stress test: verify Histogram memory stays bounded under 100k+ samples
- [ ] TEL-T013 Ensure backward compatibility or provide migration path for Histogram API changes

**Checkpoint**: Histogram bounded. No unbounded memory growth under load. Naming convention documented and enforced.

---

## Phase 3: Export and Production Readiness

**Goal**: Metrics scrapeable by Prometheus and pushable via OTLP.

- [ ] TEL-T014 Implement PrometheusTelemetry (or similar) behind `prometheus` feature flag
- [ ] TEL-T015 Implement OTLP metrics push behind `otlp` feature flag
- [ ] TEL-T016 Coordinate with nebula-metrics on export format and adapter interface
- [ ] TEL-T017 [P] Write tests for Prometheus export: valid format, correct values
- [ ] TEL-T018 [P] Write tests for OTLP push: mock endpoint, verify payload

**Checkpoint**: Prometheus scrape produces valid metrics. OTLP push works. Both are feature-gated and add no dependencies when disabled.

---

## Phase 4: Ecosystem and DX

**Goal**: Production deployment support with dashboards and trace correlation.

- [ ] TEL-T019 [P] Create standard Grafana dashboard templates for workflow metrics
- [ ] TEL-T020 [P] Create alert rule examples (failure rate, latency spikes)
- [ ] TEL-T021 Implement trace_id correlation between telemetry events and nebula-log
- [ ] TEL-T022 Write documentation and examples for production deployment

**Checkpoint**: Grafana dashboards importable. Alert rules tested. trace_id links events to log traces.

---

## Dependencies & Execution Order

Phases are sequential. Each phase builds on the stability guarantees of the previous one.

- **Phase 1** establishes the contract. No export, no optimization, just correctness.
- **Phase 2** hardens runtime behavior (Histogram bounds). Must complete before export makes sense.
- **Phase 3** adds export. Coordinates with nebula-metrics crate (which already has Prometheus text export). Decide whether telemetry exports directly or metrics crate adapts.
- **Phase 4** is documentation and DX. Requires Phase 3 exports to be functional.

Within each phase, tasks marked [P] can be developed in parallel.
