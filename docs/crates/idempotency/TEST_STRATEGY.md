# Test Strategy

## Test Pyramid

- **Unit:** IdempotencyKey::generate determinism; IdempotencyManager check_and_mark; key format; NodeAttempt with key.
- **Integration:** Execution with idempotency; storage backend (Postgres); HTTP layer with Idempotency-Key.
- **Contract:** Key format stability; DuplicateIdempotencyKey semantics.
- **End-to-end:** Duplicate request returns cached response; no double execution.

## Critical Invariants

- Same (execution_id, node_id, attempt) → same IdempotencyKey.
- check_and_mark returns true exactly once per key.
- Duplicate key → DuplicateIdempotencyKey or cached result.
- NodeAttempt preserves idempotency_key on serialization.

## Scenario Matrix

| Scenario | Coverage |
|----------|----------|
| Happy path | First key → execute; second key → skip/cached |
| Retry path | Same attempt number → same key → dedup |
| Different attempts | Different attempt numbers → different keys |
| Storage failure | Fail open or fail closed; no double execute |
| Upgrade/migration | Key format; storage schema |

## Tooling

- **Property testing:** proptest for key generation (exec_id, node_id, attempt).
- **Fuzzing:** Optional; key string fuzz.
- **Benchmarks:** check_and_mark latency; storage get/set.
- **CI quality gates:** `cargo test -p nebula-execution`; idempotency tests.

## Exit Criteria

- **Coverage goals:** Key generation; check_and_mark; NodeAttempt; error.
- **Flaky test budget:** Zero.
- **Performance regression:** check_and_mark < 1µs (in-memory).
