# Tasks: nebula-idempotency

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix `IDP`

---

## Phase 1: Documentation and Alignment 🔄

**Goal**: Complete doc set; document current execution module state; define target architecture.

- [ ] IDP-T001 [P] Write complete idempotency doc set (README, ARCHITECTURE, API, DECISIONS, ROADMAP)
- [ ] IDP-T002 [P] Document current state: execution module idempotency, existing DB schema
- [ ] IDP-T003 Define target architecture: multi-level (in-memory, storage, HTTP, action, workflow) in ARCHITECTURE.md
- [ ] IDP-T004 Clarify execution integration: where deduplication hooks into engine/runtime

**Checkpoint**: Docs complete; execution integration clearly specified.

---

## Phase 2: Persistent Storage ⬜

**Goal**: `IdempotencyStorage` trait; Postgres backend with `idempotency_keys` table; TTL cleanup.

**Prerequisite**: `nebula-storage` Phase 1 (STG Phase 1 tasks must be done).

- [ ] IDP-T005 Define `IdempotencyStorage` trait in `src/storage.rs` — `check_and_mark`, `mark_complete`, `get_cached_response`
- [ ] IDP-T006 Create `idempotency_keys` table migration in `migrations/` — key, status, response, expires_at
- [ ] IDP-T007 Implement `PostgresIdempotencyStorage` in `src/postgres.rs` using `sqlx`
- [ ] IDP-T008 [P] Implement TTL-based key expiry and cleanup job
- [ ] IDP-T009 [P] Implement `IdempotencyManager` wrapping storage for high-level API
- [ ] IDP-T010 Write tests: key persists across restart; duplicate returns cached result

**Checkpoint**: Persistent dedup: key survives restart; `check_and_mark < 5ms` (storage target).

---

## Phase 3: HTTP and Request-Level ⬜

**Goal**: axum middleware; response caching; conflict handling (wait vs reject).

- [ ] IDP-T011 Implement `IdempotencyLayer` for axum in `src/http/layer.rs` — extract `Idempotency-Key` header
- [ ] IDP-T012 [P] Implement response caching — serialize and cache HTTP response body + status
- [ ] IDP-T013 [P] Implement conflict handling: `wait` strategy (poll until first request completes)
- [ ] IDP-T014 [P] Implement conflict handling: `reject` strategy (return 409 Conflict immediately)
- [ ] IDP-T015 Write tests: Stripe-compatible key semantics; duplicate POST returns cached response

**Checkpoint**: API dedup working; Stripe-compatible; both conflict strategies tested.

---

## Phase 4: Action and Workflow Levels ⬜

**Goal**: `IdempotentAction` trait; key strategies; workflow checkpointing for resume.

- [ ] IDP-T016 Define `IdempotentAction` trait in `src/action.rs` — `idempotency_key(&self, context) -> String`
- [ ] IDP-T017 [P] Define `IdempotencyConfig` for per-action configuration — TTL, key strategy
- [ ] IDP-T018 Implement key strategies: input-hash, execution-id, custom in `src/key_strategy.rs`
- [ ] IDP-T019 [P] Implement workflow checkpointing — persist completed node outputs for resume
- [ ] IDP-T020 Write test: workflow resume from checkpoint skips already-completed nodes

**Checkpoint**: Action-level idempotency working; workflow resumes correctly from checkpoint.

---

## Dependencies & Execution Order

- Phase 1 → Phase 2 → Phase 3 → Phase 4 (sequential)
- Phase 2 requires `nebula-storage` Phase 1 (Postgres backend)
- Phase 3 requires `nebula-api` Phase 1 (axum setup)
- [P] tasks within phases can run in parallel

## Verification (after all phases)

- [ ] `cargo check -p nebula-idempotency`
- [ ] `cargo test -p nebula-idempotency`
- [ ] `cargo clippy -p nebula-idempotency -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-idempotency`
- [ ] No duplicate execution for same idempotency key in integration tests
