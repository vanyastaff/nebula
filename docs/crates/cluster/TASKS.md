# Tasks: nebula-cluster

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix `CLR`

---

## Phase 1: Contract and Safety Baseline ⬜

**Goal**: Create crate; membership model; placement API with deterministic behavior.

- [ ] CLR-T001 Create `crates/cluster` crate with `Cargo.toml` and workspace registration
- [ ] CLR-T002 Define `ClusterMember` struct — node ID, address, capabilities, state in `src/member.rs`
- [ ] CLR-T003 [P] Implement safe `join(node_info) -> Result<MemberId>` API in `src/membership.rs`
- [ ] CLR-T004 [P] Implement safe `leave(member_id)` — drains workflows before removal
- [ ] CLR-T005 Define basic placement API in `src/placement.rs` — `place_workflow(workflow_id) -> MemberId`
- [ ] CLR-T006 Ensure placement is deterministic — same inputs yield same placement decision
- [ ] CLR-T007 Write contract tests: join/leave lifecycle + placement in `tests/membership.rs`
- [ ] CLR-T008 [P] Verify contract compatibility with `nebula-runtime`

**Checkpoint**: Compile-time and integration contract checks pass with runtime; placement deterministic.

---

## Phase 2: Runtime Hardening ⬜

**Goal**: Failover detection; idempotent rescheduling; durable control-plane state; observability.

- [ ] CLR-T009 Implement failover detection — detect unreachable members via heartbeat timeout
- [ ] CLR-T010 [P] Implement idempotent rescheduling — reassign workflows from failed nodes
- [ ] CLR-T011 Integrate control-plane state with `nebula-storage` — persist membership and placement
- [ ] CLR-T012 [P] Add observability hooks: emit cluster lifecycle events (join, leave, failover)
- [ ] CLR-T013 Write failure-injection tests: kill node → verify workflows reassigned correctly

**Checkpoint**: Failure-injection tests show deterministic recovery; control-plane state persisted.

---

## Phase 3: Scale and Performance ⬜

**Goal**: Scheduler optimization; fairness; benchmark placement latency.

- [ ] CLR-T014 [P] Optimize scheduler for high-cardinality workflow distributions (1000+ workflows)
- [ ] CLR-T015 Implement fairness guarantees — prevent hot-node assignment under burst load
- [ ] CLR-T016 Add criterion benchmarks for cluster decision latency in `benches/placement.rs`
- [ ] CLR-T017 Verify placement latency within SLO budget under load

**Checkpoint**: Placement latency within budget; fairness maintained under burst.

---

## Phase 4: Ecosystem and DX ⬜

**Goal**: Operator APIs/CLI; autoscaling policy framework; incident runbooks.

- [ ] CLR-T018 [P] Implement operator API: `rebalance()` — redistribute workflows across nodes
- [ ] CLR-T019 [P] Implement maintenance mode: drain node without downtime
- [ ] CLR-T020 Define autoscaling policy framework — scale-out trigger conditions and cooldown
- [ ] CLR-T021 [P] Write staged rollout policy: add nodes gradually with verification
- [ ] CLR-T022 [P] Write incident runbook in `docs/crates/cluster/RUNBOOK.md`
- [ ] CLR-T023 Validate production readiness checklist in staging simulation

**Checkpoint**: Operator APIs functional; autoscaling policy defined; runbook published.

---

## Dependencies & Execution Order

- Phase 1 → Phase 2 → Phase 3 → Phase 4 (sequential)
- Phase 1 requires `nebula-runtime` Phase 3 (multi-runtime) to be designed first
- Phase 2 requires `nebula-storage` Phase 1 (Postgres) for durable state
- [P] tasks within phases can run in parallel

## Verification (after all phases)

- [ ] `cargo check -p nebula-cluster`
- [ ] `cargo test -p nebula-cluster`
- [ ] `cargo clippy -p nebula-cluster -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-cluster`
- [ ] No split-brain placement ownership in test scenarios
