# nebula-idempotency (Planned)

Idempotency system for Nebula: deduplication, retry safety, and exactly-once execution.

## Scope

- **In scope (target):**
  - Multi-level idempotency: Action, Workflow, Request, Transaction
  - Storage backends: in-memory, PostgreSQL, Redis
  - HTTP Idempotency-Key header support
  - Workflow checkpointing and resume

- **Out of scope:**
  - General retry logic (see `nebula-resilience`)
  - Credential storage (see `nebula-credential`)

## Current State

- **Maturity:** Partial — core types in `nebula-execution`; no standalone crate
- **Key strengths:** `IdempotencyKey` and `IdempotencyManager` in execution; deterministic key from execution_id, node_id, attempt; `NodeAttempt` carries idempotency_key; DB schema has `idempotency_keys` table and `executions.idempotency_key`
- **Key risks:** In-memory only (IdempotencyManager uses HashSet); no persistent storage; no HTTP layer; no action-level idempotency

## Target State

- **Production criteria:** Persistent storage (PostgreSQL); HTTP IdempotencyLayer; action-level IdempotentAction trait; workflow checkpointing
- **Compatibility guarantees:** Key format stable; ExecutionError::DuplicateIdempotencyKey preserved

## Document Map

- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [INTERACTIONS.md](./INTERACTIONS.md)
- [DECISIONS.md](./DECISIONS.md)
- [ROADMAP.md](./ROADMAP.md)
- [PROPOSALS.md](./PROPOSALS.md)
- [SECURITY.md](./SECURITY.md)
- [RELIABILITY.md](./RELIABILITY.md)
- [TEST_STRATEGY.md](./TEST_STRATEGY.md)
- [MIGRATION.md](./MIGRATION.md)

## Archive

Legacy material:
- [`_archive/`](./_archive/)
