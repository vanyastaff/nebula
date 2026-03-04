# Tasks: nebula-tenant

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix `TNT`

---

## Phase 1: Contract and Safety Baseline ⬜

**Goal**: Create crate; tenant context model; identity validation; fail-closed policy.

- [ ] TNT-T001 Create `crates/tenant` crate with `Cargo.toml` and workspace registration
- [ ] TNT-T002 Define `TenantContext` struct — tenant ID, plan, quota snapshot in `src/context.rs`
- [ ] TNT-T003 [P] Implement identity validation in `src/identity.rs` — verify tenant ID format and existence
- [ ] TNT-T004 [P] Define baseline policy API in `src/policy.rs` — `check_ingress(context) -> Result<(), TenantError>`
- [ ] TNT-T005 Implement fail-closed ingress checks — deny by default; require explicit allow
- [ ] TNT-T006 Write cross-crate contract tests for tenant context propagation in `tests/propagation.rs`
- [ ] TNT-T007 Document cross-crate boundaries: how API layer extracts tenant, how engine propagates it

**Checkpoint**: Tenant context propagates end-to-end through critical workflow paths; fail-closed policy enforced.

---

## Phase 2: Runtime Hardening ⬜

**Goal**: Quota accounting; admission checkpoints in runtime/resource operations; audit trail.

- [ ] TNT-T008 Implement concurrency-safe quota accounting in `src/quota.rs` (atomic or sharded)
- [ ] TNT-T009 Add admission checkpoints in engine — reject workflow start if tenant over quota
- [ ] TNT-T010 [P] Add admission checkpoints in resource acquire path
- [ ] TNT-T011 [P] Emit structured audit events for tenant policy decisions
- [ ] TNT-T012 Write stress test for quota under concurrent updates — verify no race conditions
- [ ] TNT-T013 Verify deterministic quota behavior under retry scenarios

**Checkpoint**: Quota deterministic under stress; audit trail emitted for all policy decisions.

---

## Phase 3: Scale and Performance ⬜

**Goal**: High-cardinality optimization; policy cache with bounded staleness.

- [ ] TNT-T014 Implement policy cache in `src/cache.rs` — cache hot policy decisions with TTL
- [ ] TNT-T015 [P] Optimize high-cardinality tenant workloads — avoid per-request allocations
- [ ] TNT-T016 [P] Tune telemetry cardinality — cap label cardinality for multi-tenant metrics
- [ ] TNT-T017 Benchmark policy check latency under representative tenant distribution

**Checkpoint**: Stable latency across representative distribution; cache reduces policy check overhead.

---

## Phase 4: Ecosystem and DX ⬜

**Goal**: Partition strategy tooling; policy templates; operator runbooks.

- [ ] TNT-T018 Write partition strategy tooling and migration assistants
- [ ] TNT-T019 [P] Write tenant policy templates for common tiers: free, pro, enterprise
- [ ] TNT-T020 [P] Write operator runbook in `docs/crates/tenant/RUNBOOK.md`
- [ ] TNT-T021 Validate safe rollout playbook in staging environment simulation

**Checkpoint**: Runbook available; policy templates cover common tiers; migration assistants tested.

---

## Dependencies & Execution Order

- Phase 1 → Phase 2 → Phase 3 → Phase 4 (sequential)
- Phase 1 requires API Phase 1 to be clear on request tenant extraction
- [P] tasks within phases can run in parallel

## Verification (after all phases)

- [ ] `cargo check -p nebula-tenant`
- [ ] `cargo test -p nebula-tenant`
- [ ] `cargo clippy -p nebula-tenant -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-tenant`
- [ ] No cross-tenant policy bypass in contract tests
