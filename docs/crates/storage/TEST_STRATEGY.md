# Test Strategy

## Test Pyramid

- **Unit:** MemoryStorage get/set/delete/exists; MemoryStorageTyped roundtrip; StorageError display; serialization error propagation.
- **Integration:** (Future) PostgresStorage, RedisStorage, S3Storage with real backends (testcontainers, localstack, embedded Redis).
- **Contract:** Storage trait semantics; get returns Option; set overwrites; delete idempotent.
- **End-to-end:** Consumer (e.g. workflow repo) uses storage; full flow.

## Critical Invariants

- get returns Ok(None) when key absent; Ok(Some(v)) when present.
- set overwrites existing value.
- delete removes key; exists returns false after delete.
- exists returns true iff get would return Some.
- MemoryStorageTyped serializes/deserializes via serde_json; roundtrip preserves data.
- StorageError::Serialization on invalid JSON in MemoryStorageTyped::get.

## Scenario Matrix

| Scenario | Coverage |
|----------|----------|
| Happy path | set → get → Some; delete → get → None |
| Missing key | get → None; exists → false |
| Overwrite | set(a) → set(b) → get → Some(b) |
| Delete absent | delete → Ok (idempotent) |
| Serialization | MemoryStorageTyped with invalid JSON in raw bytes → Serialization error |
| Typed roundtrip | Struct with nested values; set → get → equal |

## Tooling

- **Property testing:** proptest for key/value roundtrip (MemoryStorageTyped).
- **Fuzzing:** Optional; key string fuzz; value bytes fuzz.
- **Benchmarks:** get/set latency; throughput.
- **CI quality gates:** `cargo test -p nebula-storage`.

## Exit Criteria

- **Coverage goals:** MemoryStorage, MemoryStorageTyped, StorageError; (future) each backend.
- **Flaky test budget:** Zero.
- **Performance regression:** get/set < 1ms (MemoryStorage).
