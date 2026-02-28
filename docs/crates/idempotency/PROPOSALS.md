# Proposals

## P001: Extract nebula-idempotency Crate

**Type:** Non-breaking (additive)

**Motivation:** Clear ownership; storage backends; HTTP layer; action trait. Execution keeps minimal types or re-exports.

**Proposal:** Create `nebula-idempotency` crate. Move IdempotencyKey, IdempotencyManager from execution (or duplicate and deprecate). Add IdempotencyStorage, PostgresStorage, IdempotencyLayer. Execution depends on idempotency for types.

**Expected benefits:** Single place for idempotency; storage abstraction; HTTP integration.

**Costs:** New crate; migration of execution types; dependency direction.

**Risks:** Execution may need to keep key generation for NodeAttempt; avoid circular deps.

**Compatibility impact:** Execution re-exports or depends; API unchanged.

**Status:** Draft

---

## P002: IdempotencyStorage Trait

**Type:** Non-breaking

**Motivation:** Pluggable backends; in-memory for test; Postgres for production.

**Proposal:** `trait IdempotencyStorage { async fn get(&self, key: &str) -> Result<Option<CachedResult>>; async fn set(&self, key: &str, result: &CachedResult, ttl: Duration) -> Result<()>; }`. Implement MemoryStorage, PostgresStorage. IdempotencyManager uses trait.

**Expected benefits:** Testability; production storage; TTL.

**Costs:** Async; storage dependency.

**Risks:** Storage failure handling; atomicity.

**Compatibility impact:** Additive; current in-memory remains default.

**Status:** Draft

---

## P003: Idempotency-Key HTTP Layer

**Type:** Non-breaking

**Motivation:** Stripe-style API deduplication; Idempotency-Key header; response caching.

**Proposal:** Axum middleware: extract Idempotency-Key header; check storage; if hit return cached response (X-Idempotency-Replay: true); if miss execute handler, cache response, return. TTL 24h default.

**Expected benefits:** API safety; standard pattern.

**Costs:** Middleware complexity; cache size; conflict handling.

**Risks:** Concurrent requests with same key; cache invalidation.

**Compatibility impact:** Additive; opt-in per route.

**Status:** Draft

---

## P004: IdempotentAction Trait

**Type:** Non-breaking

**Motivation:** Action-level idempotency; content-based or user keys; is_safe_to_retry.

**Proposal:** `trait IdempotentAction { fn idempotency_config(&self) -> IdempotencyConfig; async fn is_safe_to_retry(&self, input, prev_result, context) -> Result<bool>; }`. Executor wrapper checks key before execute; caches result.

**Expected benefits:** Per-action config; retry safety.

**Costs:** Trait implementation; executor composition.

**Risks:** Key generation from input; parameter hashing.

**Compatibility impact:** Additive; actions opt-in.

**Status:** Draft
