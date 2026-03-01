# nebula-idempotency Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Workflow runs and API requests can be retried or duplicated (e.g. at-least-once queue delivery, client retry). Idempotency ensures that repeating the same logical operation (same key) does not execute twice: first request runs, subsequent requests with same key return cached result or "already done". Prevents duplicate side effects and double charges.

**nebula-idempotency is the idempotency system for Nebula: deduplication, retry safety, and exactly-once execution.**

It answers: *How is idempotency key defined (action, workflow, request), where is it stored, and how do runtime and API enforce at-most-once or exactly-once semantics?*

```
Request or execution carries idempotency key (e.g. execution_id + node_id + attempt)
    ↓
IdempotencyManager: lookup key → already completed? return cached result : run and store result
    ↓
Storage: in-memory (MVP) or PostgreSQL/Redis for persistence
```

Contract: key format stable; duplicate key returns cached result or DuplicateIdempotencyKey; no double execution for same key. Partially implemented (execution crate); standalone crate and persistence planned.

---

## User Stories

### Story 1 — Worker Retry Does Not Double-Run (P1)

Worker runs task; succeeds but ack fails. Queue redelivers task. Worker presents same idempotency key; idempotency layer returns cached success. Task is not run again.

**Acceptance**: Key = f(execution_id, node_id, attempt); lookup before run; store result on success; return stored result on duplicate key.

### Story 2 — API Idempotency-Key Header (P2)

Client sends POST with Idempotency-Key header. First request runs; subsequent requests with same key within window return same response. Storage persists key and response.

**Acceptance**: HTTP layer extracts key; idempotency layer checks before handler; store response; return 200 + same body for duplicate.

### Story 3 — Workflow Checkpoint and Resume (P2)

Long workflow can checkpoint. Resume with same execution_id uses checkpoint; no replay from start. Idempotency key may span workflow or per-node.

**Acceptance**: Checkpoint stored with key; resume returns checkpoint state; document key scope (workflow vs node).

---

## Core Principles

### I. Key Format Is Stable

**Idempotency key (e.g. execution_id + node_id + attempt) is deterministic and documented. Changing key format breaks deduplication.**

**Rationale**: Storage and retry depend on key. Stability is critical.

### II. Duplicate Key Returns Cached or Error, Never Re-Run

**Same key twice: first run executes and stores result; second returns stored result or explicit DuplicateIdempotencyKey. No second execution.**

**Rationale**: Prevents double side effects. Cached result or error is the contract.

### III. Storage Is Pluggable

**In-memory for tests; PostgreSQL/Redis for production. IdempotencyManager depends on storage trait.**

**Rationale**: Production needs persistence across restarts. Tests need speed and simplicity.

### IV. No Business Logic in Idempotency Crate

**Idempotency checks and stores keys and results. It does not run actions or workflows.**

**Rationale**: Runtime runs; idempotency wraps. Clear boundary.

---

## Production Vision

Multi-level idempotency (action, workflow, request); persistent storage (PostgreSQL, Redis); HTTP Idempotency-Key support; workflow checkpointing. Key format stable; ExecutionError::DuplicateIdempotencyKey preserved. From archives: idempotency keys table, NodeAttempt, execution_id + node_id + attempt. Gaps: persistent storage; HTTP layer; action-level trait; workflow checkpointing.

### Key gaps

| Gap | Priority |
|-----|----------|
| Persistent storage (PostgreSQL/Redis) | Critical |
| HTTP IdempotencyLayer and header handling | High |
| Action-level IdempotentAction trait | Medium |
| Workflow checkpointing and resume | Medium |
| Key TTL and cleanup | Low |

---

## Key Decisions

### D-001: Key = f(execution_id, node_id, attempt)

**Decision**: Node-level idempotency key is derived from execution, node, and attempt so that retries of same node use same key.

**Rationale**: Worker and queue retry same task; key must match. Attempt distinguishes first try from retry when desired.

**Rejected**: Random key per request — would not dedupe retries.

### D-002: Store Result on Success

**Decision**: On successful execution, store (key, result) so that duplicate key can return same result.

**Rationale**: Client and worker expect same response for duplicate, not just "already done" with no body.

**Rejected**: Store only "done" flag — would not support API idempotency response body.

### D-003: DuplicateIdempotencyKey in ExecutionError

**Decision**: Execution or idempotency layer returns explicit DuplicateIdempotencyKey variant so that engine and API can distinguish from other errors.

**Rationale**: API returns 200 + cached body or 409; engine does not retry. Explicit variant enables this.

**Rejected**: Generic "already exists" — would lose semantics.

---

## Non-Negotiables

1. **Key format stable** — execution_id, node_id, attempt (or documented equivalent); no break in minor.
2. **Duplicate key never re-runs** — return cached result or DuplicateIdempotencyKey.
3. **Storage pluggable** — in-memory for tests; persistent for production.
4. **No business logic in idempotency** — check and store only.
5. **Breaking key or result contract = major + MIGRATION.md**.

---

## Governance

- **PATCH**: Bug fixes, docs. No key or result change.
- **MINOR**: Additive (new key scope, new storage backend). No key format break.
- **MAJOR**: Key format or duplicate semantics change; MIGRATION.md required.
