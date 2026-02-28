# Roadmap

## Phase 1: Documentation and Alignment (Current)

- **Deliverables:**
  - Idempotency documentation complete (this doc set)
  - Document current state: execution module, DB schema
  - Define target: multi-level, storage, HTTP
- **Risks:** None
- **Exit criteria:** Docs complete; execution integration clear

## Phase 2: Persistent Storage

- **Deliverables:**
  - IdempotencyStorage trait
  - PostgresStorage using idempotency_keys table
  - IdempotencyManager backed by storage (optional)
  - TTL and cleanup
- **Risks:** Schema migration if table design changes
- **Exit criteria:** Persistent dedup; key survives restart

## Phase 3: HTTP and Request-Level

- **Deliverables:**
  - IdempotencyLayer for axum (Idempotency-Key header)
  - Response caching
  - Conflict handling (wait, reject)
- **Risks:** Header semantics; cache size
- **Exit criteria:** API dedup working; Stripe-compatible

## Phase 4: Action and Workflow Levels

- **Deliverables:**
  - IdempotentAction trait
  - IdempotencyConfig, key strategies
  - Workflow checkpointing
  - Extract nebula-idempotency crate (optional)
- **Risks:** Integration complexity; action composition
- **Exit criteria:** Action-level idempotency; checkpoint resume

## Metrics of Readiness

| Metric | Target |
|--------|--------|
| Correctness | No duplicate execution for same key |
| Latency | check_and_mark < 1ms (in-memory); < 5ms (storage) |
| Durability | Keys persisted; TTL enforced |
| Compatibility | Key format stable; DuplicateIdempotencyKey preserved |
