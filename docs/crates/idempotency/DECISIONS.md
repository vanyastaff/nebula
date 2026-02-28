# Decisions

## D001: Idempotency in Execution Crate (Current)

**Status:** Adopt

**Context:** Node-level idempotency needed for retry safety; execution owns attempt lifecycle.

**Decision:** `IdempotencyKey` and `IdempotencyManager` live in `nebula-execution`. Key format: `{execution_id}:{node_id}:{attempt}`. In-memory HashSet for dedup.

**Alternatives considered:** Separate nebula-idempotency from start; storage in engine.

**Trade-offs:** No persistent storage; single-process; acceptable for MVP.

**Consequences:** Future extraction to idempotency crate; migration path needed.

**Migration impact:** None until extraction.

**Validation plan:** Unit tests in execution; key determinism.

---

## D002: Deterministic Key from Context

**Status:** Adopt

**Context:** Node retries must use same key for same logical attempt.

**Decision:** Key = `execution_id:node_id:attempt`. Deterministic; no user input in node-level key.

**Alternatives considered:** UUID per attempt; content hash.

**Trade-offs:** Simple; no collision across executions; attempt number distinguishes retries.

**Consequences:** Key format is contract; changing requires migration.

**Migration impact:** None.

**Validation plan:** Unit test: same inputs → same key.

---

## D003: In-Memory Manager (Current)

**Status:** Adopt (with known limitation)

**Context:** MVP needs zero external deps; single-process execution.

**Decision:** `IdempotencyManager` uses `HashSet<String>`; check_and_mark is insert (returns true if new).

**Alternatives considered:** PostgreSQL from start; Redis.

**Trade-offs:** Lost on restart; no cross-process; simple.

**Consequences:** Production needs persistent backend (Phase 2).

**Migration impact:** Add storage backend; manager becomes facade.

**Validation plan:** Unit tests; document limitation.

---

## D004: DB Schema for Future Use

**Status:** Adopt

**Context:** Migrations define `idempotency_keys` table and `executions.idempotency_key`; ready for persistent idempotency.

**Decision:** Schema exists; code does not yet use it. Execution-level key for workflow dedup; table for request/action-level.

**Alternatives considered:** No schema until needed; different table design.

**Trade-offs:** Schema ready; no code coupling yet.

**Consequences:** Future idempotency crate implements storage against this schema.

**Migration impact:** None.

**Validation plan:** Migration applies; table exists.

---

## D005: DuplicateIdempotencyKey Error

**Status:** Adopt

**Context:** Caller must distinguish duplicate from first execution.

**Decision:** `ExecutionError::DuplicateIdempotencyKey(String)` when key already seen. Caller returns cached or rejects.

**Alternatives considered:** Return cached result implicitly; panic.

**Trade-offs:** Explicit error; caller controls behavior.

**Consequences:** API contract; preserved in idempotency crate.

**Migration impact:** None.

**Validation plan:** Error display; caller handling.
