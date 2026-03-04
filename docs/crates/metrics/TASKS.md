# Tasks: nebula-metrics

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix `MET`

---

## Phase 1: Documentation and Alignment ✅

**Goal**: Full doc set; document current state; define target naming convention.

- [x] MET-T001 Write complete metrics doc set (README, ARCHITECTURE, API, INTERACTIONS, DECISIONS, PROPOSALS, ROADMAP, SECURITY, RELIABILITY, TEST_STRATEGY, MIGRATION)
- [x] MET-T002 Write CURRENT_STATE.md — document telemetry, log, and domain crate current metric state
- [x] MET-T003 Write TARGET.md — define unified export and `nebula_*` naming convention
- [x] MET-T004 Align ROADMAP with telemetry and eventbus cross-links

**Checkpoint**: ✅ Done — full doc set committed.

---

## Phase 2: Naming and Adapters ✅

**Goal**: Standard `nebula_*` metric naming; TelemetryAdapter; document metric names for all crates.

- [x] MET-T005 Define metric naming constants for workflow, action, node, credential, resource
- [x] MET-T006 Implement `TelemetryAdapter` bridging domain crates to telemetry export
- [x] MET-T007 Update engine and runtime to record metrics under `nebula_*` names
- [x] MET-T008 Document all metric names in TARGET.md

**Checkpoint**: ✅ Done — `TelemetryAdapter` in place; engine/runtime use `nebula_*` names.

---

## Phase 3: Export Implementation ✅

**Goal**: Prometheus exporter; `/metrics` HTTP endpoint wiring.

- [x] MET-T009 Implement `snapshot(registry) -> String` for Prometheus text format
- [x] MET-T010 Implement `PrometheusExporter` struct with `content_type()` method
- [x] MET-T011 Verify Prometheus format valid (scrape-compatible output)
- [ ] MET-T012 Wire `/metrics` GET handler in `nebula-api` returning `exporter.snapshot()` (remaining)

**Checkpoint**: Prometheus scrape works; `/metrics` endpoint wired in nebula-api.

---

## Phase 4: OTLP and Unification ⬜

**Goal**: OTLP push; unified registry for domain crates; standard dashboards.

- [ ] MET-T013 Implement OTLP push exporter behind `otlp` feature flag in `src/export/otlp.rs`
- [ ] MET-T014 [P] Implement unified registry adapter for domain crates (credential, resource, worker)
- [ ] MET-T015 [P] Add credential metrics under `nebula_credential_*` naming convention
- [ ] MET-T016 [P] Add resource metrics under `nebula_resource_*` naming convention
- [ ] MET-T017 [P] Add worker metrics under `nebula_worker_*` naming convention
- [ ] MET-T018 Create standard Grafana dashboard JSON in `docs/dashboards/`
- [ ] MET-T019 Write runbook for configuring OTLP collector endpoint

**Checkpoint**: OTLP push operational; all domain crates' metrics exported; Grafana dashboard committed.

---

## Dependencies & Execution Order

- Phases 1–3 complete; Phase 4 is next
- MET-T012 (wire /metrics in api) depends on `nebula-api` Phase 1 being in progress
- Phase 4 [P] tasks can all run in parallel

## Verification (after all phases)

- [ ] `cargo check -p nebula-metrics --all-features`
- [ ] `cargo test -p nebula-metrics`
- [ ] `cargo clippy -p nebula-metrics -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-metrics`
- [ ] Prometheus scrape returns valid format
