# Tasks: nebula-worker

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix `WRK`

---

## Phase 1: Contract and Safety Baseline ⬜

**Goal**: Worker config/state model; queue lease contract; sandbox integration; metrics skeleton.

- [ ] WRK-T001 Define worker config model in `src/config.rs` — concurrency, timeouts, queue endpoint
- [ ] WRK-T002 Implement queue lease contract in `src/lease.rs`: `claim`, `heartbeat`, `ack`, `nack`
- [ ] WRK-T003 [P] Implement lease idempotency — ensure finalization (ack/nack) is idempotent
- [ ] WRK-T004 Implement sandbox integration: route claimed task to sandbox/runtime for execution
- [ ] WRK-T005 [P] Implement timeout/cancel flow — cancel task execution on lease timeout
- [ ] WRK-T006 Wire core metrics/logging: task claimed, started, completed, failed, nacked
- [ ] WRK-T007 Write contract tests for lease lifecycle in `tests/lease_lifecycle.rs`
- [ ] WRK-T008 [P] Write drain behavior integration test — worker drains queue before shutdown

**Checkpoint**: Lease contract tests green; drain validated; finalization idempotency confirmed.

---

## Phase 2: Runtime Hardening ⬜

**Goal**: Retry/backoff, failure taxonomy, dead-letter strategy, health/readiness.

- [ ] WRK-T009 Integrate retry/backoff using `nebula-resilience` policies in `src/retry.rs`
- [ ] WRK-T010 [P] Define structured failure taxonomy in `src/error.rs`: transient, fatal, timeout, capability-denied
- [ ] WRK-T011 Implement dead-letter strategy — move failed tasks to dead-letter queue after max retries
- [ ] WRK-T012 [P] Implement health endpoint — healthy/draining/degraded states
- [ ] WRK-T013 Implement graceful rolling restart — finish in-flight tasks, stop accepting new ones
- [ ] WRK-T014 Write chaos tests in `tests/chaos.rs` — inject queue failures, runtime failures, verify no task loss
- [ ] WRK-T015 Write restart simulation test — no task loss on rolling restart

**Checkpoint**: Chaos tests pass; no task loss in restart simulations; retry storms prevented by circuit breaker.

---

## Phase 3: Scale and Performance ⬜

**Goal**: Adaptive concurrency, autoscaling signals, hot-path optimization.

- [ ] WRK-T016 Implement adaptive concurrency: scale worker count based on queue depth and latency
- [ ] WRK-T017 [P] Emit autoscaling signals: queue saturation, lease lag, completion latency
- [ ] WRK-T018 [P] Benchmark hot-path execution overhead in `benches/throughput.rs`
- [ ] WRK-T019 Optimize task claim and dispatch cycle to minimize latency
- [ ] WRK-T020 Write load test: verify target throughput and p95 latency under stress

**Checkpoint**: Target throughput met; p95 latency within SLO; autoscaling signals emitted.

---

## Phase 4: Ecosystem and DX ⬜

**Goal**: Operator handbook, compatibility matrix, telemetry dashboards, SLO alerts.

- [ ] WRK-T021 Write worker operator handbook in `docs/crates/worker/RUNBOOK.md` — start, stop, drain, scale
- [ ] WRK-T022 [P] Write plugin/action compatibility matrix — which action types work with which configurations
- [ ] WRK-T023 [P] Define telemetry dashboard queries and SLO alert rules
- [ ] WRK-T024 Exercise contract versioning — test one breaking change with migration path

**Checkpoint**: Runbook drill completed; dashboards defined; migration policy tested.

---

## Dependencies & Execution Order

- Phase 1 → Phase 2 → Phase 3 → Phase 4 (sequential)
- Phase 1 depends on `nebula-execution` (task state) and `nebula-sandbox`/`nebula-runtime` (execution)
- [P] tasks within phases can run in parallel

## Verification (after all phases)

- [ ] `cargo check -p nebula-worker`
- [ ] `cargo test -p nebula-worker`
- [ ] `cargo clippy -p nebula-worker -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-worker`
