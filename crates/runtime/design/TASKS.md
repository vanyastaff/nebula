# Tasks: nebula-runtime

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix `RTM`

---

## Phase 1: Isolation and Spill 🔄

**Goal**: Route actions by isolation level; implement SpillToBlob; enforce byte budget.

- [ ] RTM-T001 Implement `resolve_isolation_level(metadata: &ActionMetadata) -> IsolationLevel` in `src/isolation.rs`
- [ ] RTM-T002 Route `IsolationLevel::Trusted` actions to direct executor (bypass sandbox)
- [ ] RTM-T003 Route `IsolationLevel::CapabilityGated` / `IsolationLevel::Isolated` through `SandboxRunner`
- [ ] RTM-T004 [P] Define blob storage abstraction in `src/blob.rs` — write blob, return reference
- [ ] RTM-T005 Implement `SpillToBlob` — write `ActionResult` payload to blob when size exceeds threshold
- [ ] RTM-T006 Add `BlobRef` to `ActionResult` for consumer to fetch on demand
- [ ] RTM-T007 Implement `max_total_execution_bytes` enforcement in executor — error if exceeded
- [ ] RTM-T008 [P] Write tests: trusted action runs directly, isolated action goes via sandbox
- [ ] RTM-T009 [P] Write tests: SpillToBlob writes to storage, BlobRef returned to consumer

**Checkpoint**: Trusted actions bypass sandbox; isolated actions go through SandboxRunner; large results spill to blob; byte budget enforced.

---

## Phase 2: Trigger Lifecycle ⬜

**Goal**: Webhook and schedule triggers activate/deactivate workflows via engine integration.

- [ ] RTM-T010 Define trigger lifecycle API: `activate`, `deactivate`, `listen` in `src/trigger.rs`
- [ ] RTM-T011 Implement webhook trigger execution — activates workflow on HTTP POST
- [ ] RTM-T012 Implement schedule trigger — fires at cron times
- [ ] RTM-T013 [P] Integrate trigger activation with engine for event-driven workflow start
- [ ] RTM-T014 Ensure triggers deactivate cleanly on workflow deactivation
- [ ] RTM-T015 Write integration tests: webhook trigger → workflow execution → completion

**Checkpoint**: Webhook trigger activates workflow on POST; schedule fires at cron; deactivation cleans up.

---

## Phase 3: Multi-Runtime Coordination ⬜

**Goal**: Multiple runtime instances with discovery, assignment, failover.

- [ ] RTM-T016 Implement `RuntimeRegistry` — register/discover runtime instances
- [ ] RTM-T017 Implement `WorkflowCoordinator` — assign workflow to runtime instance
- [ ] RTM-T018 [P] Implement load balancing strategy in coordinator
- [ ] RTM-T019 Implement failover — reassign workflow when runtime instance fails
- [ ] RTM-T020 Write integration test: two runtime instances, coordinator assignment, failover

**Checkpoint**: Multiple runtimes; coordinator assigns workflows deterministically; failover tested.

---

## Phase 4: Health and Observability ⬜

**Goal**: HealthMonitor, graceful shutdown, runtime-level metrics.

- [ ] RTM-T021 Implement `HealthMonitor` for runtime components in `src/health.rs`
- [ ] RTM-T022 Implement graceful shutdown — drain in-flight actions before stopping
- [ ] RTM-T023 [P] Add runtime metrics: queue depth, active executions via `nebula-telemetry`
- [ ] RTM-T024 [P] Expose health check endpoint or equivalent for orchestrator readiness probes
- [ ] RTM-T025 Write test: graceful shutdown completes in-flight actions before exit

**Checkpoint**: /health equivalent operational; graceful shutdown drains in-flight actions; metrics emitted.

---

## Dependencies & Execution Order

- Phase 1 is current focus; depends on `nebula-action` Phase 2 (ActionMetadata with isolation)
- Phase 1 → Phase 2 (can start after Phase 1 is complete)
- Phase 3 is optional for MVP; Phase 4 can run in parallel with Phase 3

## Verification (after all phases)

- [ ] `cargo check -p nebula-runtime`
- [ ] `cargo test -p nebula-runtime`
- [ ] `cargo clippy -p nebula-runtime -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-runtime`
- [ ] execute_action overhead measured < 1ms
