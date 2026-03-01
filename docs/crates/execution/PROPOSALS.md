# Proposals

## P-001: Resume Token Type

**Type:** Non-breaking (additive)

**Motivation:** Suspend/resume flows (e.g. execution Paused waiting for webhook) need a stable token or handle that API and engine can pass to “resume” without redefining state. Currently no resume handle in execution crate.

**Proposal:** Add optional `resume_token: Option<ResumeToken>` to ExecutionState or a new variant in ExecutionStatus (e.g. Suspended { resume_token }). `ResumeToken` is an opaque type (e.g. newtype around String or UUID) serializable for API. Engine sets it when transitioning to Paused or a “waiting” state; resume endpoint accepts token and transitions back to Running.

**Expected benefits:** Clear contract for resume; API and engine share one type; no ad-hoc tokens in engine.

**Costs:** New type and possibly new status variant; engine and API must implement resume flow.

**Risks:** Token lifecycle (expiry, one-time use) is policy; execution crate only holds the value.

**Compatibility impact:** Additive; existing state without resume_token remains valid. Serialized form gains optional field.

**Status:** Draft

---

## P-002: ExecutionResult Summary Type for API

**Type:** Non-breaking (additive)

**Motivation:** GET /api/v1/executions/:id may return a summary (status, result summary, error message) rather than full ExecutionState. A dedicated `ExecutionResult` or `ExecutionSummary` type in execution crate would standardize API response shape and keep engine/API aligned.

**Proposal:** Add `ExecutionResult` or `ExecutionSummary` struct: execution_id, workflow_id, status, completed_at, optional result_summary (e.g. output count, total bytes), optional error_message. Engine builds it from ExecutionState; API returns it. Full ExecutionState remains for internal or admin use.

**Expected benefits:** Stable API response type; single place for “what the API returns” shape.

**Costs:** One more type to maintain; possible duplication of fields from ExecutionState.

**Risks:** Drift between ExecutionState and summary if not built from state in code.

**Compatibility impact:** Additive. New type only.

**Status:** Draft

---

## P-003: IdempotencyManager Trait for Persistent Backend

**Type:** Breaking (if we replace current IdempotencyManager)

**Motivation:** Current IdempotencyManager is in-memory. Production needs persistent store (e.g. PostgreSQL). nebula-idempotency may provide storage-backed implementation; execution crate could define a minimal trait (e.g. `check_and_mark(&self, key) -> Result<bool, E>`) so that engine can be generic over in-memory vs persistent.

**Proposal:** Introduce `IdempotencyStore` trait in execution crate (or in nebula-idempotency) with `check_and_mark`. `IdempotencyManager` implements it for in-memory; idempotency crate implements it for PostgreSQL/Redis. Engine takes `Arc<dyn IdempotencyStore>`. Keep `IdempotencyKey` and key format in execution crate.

**Expected benefits:** Pluggable idempotency backend; engine unchanged when switching to persistent.

**Costs:** Trait design and possibly async; coordination with nebula-idempotency.

**Risks:** Trait in execution crate might pull in async or error type from idempotency crate; dependency direction to consider.

**Compatibility impact:** Breaking if we remove or change IdempotencyManager API. Prefer additive: add trait, keep existing IdempotencyManager as default impl.

**Status:** Draft
