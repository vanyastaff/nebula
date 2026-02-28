# Test Strategy

## Test Pyramid

- **unit:** Error types; `CredentialId` validation; `SecretString`; crypto encrypt/decrypt; `CredentialContext` scope
- **integration:** Manager CRUD with `MockStorageProvider`; cache hit/miss; scope enforcement; schema validation
- **contract:** `StorageProvider` trait compliance; `CredentialProvider` mock for action tests
- **end-to-end:** Examples as smoke tests; integration tests with testcontainers (LocalStack, Vault, K3s)

## Critical Invariants

- Scope violation always fails; never return credential from wrong scope
- Decryption failure never returns partial/corrupted data
- `SecretString` redacts in Debug/Display
- Cache does not serve credentials across scope boundaries
- Rotation rollback restores previous credential on failure

## Scenario Matrix

- **happy path:** store → retrieve → validate; cache hit; rotation success
- **retry path:** Storage timeout; caller retries (credential does not retry)
- **cancellation path:** Rotation transaction cancelled; cleanup state
- **timeout path:** Storage timeout; return `StorageError::Timeout`
- **upgrade/migration path:** Rotation policy versioning; provider capability negotiation (P-001)

## Tooling

- **property testing:** proptest for `CredentialId`; encrypted payload round-trip
- **fuzzing:** CredentialId validation; decryption with adversarial input
- **benchmarks:** criterion for encrypt/decrypt; cache hit/miss; manager operations
- **CI quality gates:** `cargo test -p nebula-credential`; `cargo test --workspace`; `cargo audit`

## Exit Criteria

- **coverage goals:** Core paths; scope enforcement; error propagation; rotation state machine
- **flaky test budget:** Zero; use deterministic mocks; testcontainers with fixed versions
- **performance regression thresholds:** Criterion benchmarks; cache hit latency < 10ms p99
