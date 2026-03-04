# Implementation Plan: nebula-idempotency

**Crate**: `nebula-idempotency` | **Path**: `crates/idempotency` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

The idempotency crate provides deduplication guarantees at multiple levels — execution (in-memory), HTTP requests (Idempotency-Key header), action, and workflow checkpoint/resume. It ensures that retried operations produce the same result without side effects. Current focus is Phase 1: documentation and alignment with current execution module state.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio
**Key Dependencies**: `nebula-core`, `nebula-storage` (for persistent storage backend)
**Testing**: `cargo test -p nebula-idempotency`

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 1: Documentation and Alignment | 🔄 In Progress | Doc set complete; execution integration clear |
| Phase 2: Persistent Storage | ⬜ Planned | IdempotencyStorage trait, Postgres backend, TTL |
| Phase 3: HTTP and Request-Level | ⬜ Planned | axum middleware, response caching, conflict handling |
| Phase 4: Action and Workflow Levels | ⬜ Planned | IdempotentAction trait, workflow checkpointing |

## Phase Details

### Phase 1: Documentation and Alignment 🔄

**Goal**: Complete idempotency documentation; document current state; define target.

**Deliverables**:
- Complete idempotency doc set
- Document current state: execution module, DB schema
- Define target: multi-level, storage, HTTP

**Exit Criteria**:
- Docs complete; execution integration clear

### Phase 2: Persistent Storage

**Goal**: `IdempotencyStorage` trait; Postgres backend; TTL and cleanup.

**Deliverables**:
- `IdempotencyStorage` trait
- `PostgresStorage` using `idempotency_keys` table
- `IdempotencyManager` backed by storage (optional)
- TTL and cleanup job

**Exit Criteria**:
- Persistent dedup: key survives restart

**Risks**:
- Schema migration if table design changes

**Dependencies**: `nebula-storage` Phase 1 (Postgres)

### Phase 3: HTTP and Request-Level

**Goal**: axum `IdempotencyLayer`; response caching; conflict handling.

**Deliverables**:
- `IdempotencyLayer` for axum — handles `Idempotency-Key` header
- Response caching — return cached response for duplicate requests
- Conflict handling: wait or reject strategies

**Exit Criteria**:
- API dedup working; Stripe-compatible key semantics

**Risks**:
- Header semantics; cache size management

### Phase 4: Action and Workflow Levels

**Goal**: `IdempotentAction` trait; workflow checkpointing; optional crate extraction.

**Deliverables**:
- `IdempotentAction` trait with key strategies
- `IdempotencyConfig` per-action configuration
- Workflow checkpointing for resume after failure
- Extract `nebula-idempotency` as standalone crate (if not already)

**Exit Criteria**:
- Action-level idempotency working; workflow resume from checkpoint

## Inter-Crate Dependencies

- **Depends on**: `nebula-core`, `nebula-storage` (Phase 2+)
- **Depended by**: `nebula-engine` (execution deduplication), `nebula-api` (HTTP middleware)

## Verification

- [ ] `cargo check -p nebula-idempotency`
- [ ] `cargo test -p nebula-idempotency`
- [ ] `cargo clippy -p nebula-idempotency -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-idempotency`
