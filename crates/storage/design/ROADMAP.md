# Roadmap

## Phase 1: Postgres Backend

**Deliverables:**
- PostgresStorage implementing Storage<String, Vec<u8>> or Storage<String, serde_json::Value>
- Key-value table (e.g. storage_kv: key TEXT PRIMARY KEY, value JSONB, updated_at TIMESTAMPTZ)
- Connection pooling via sqlx PgPool
- Feature `postgres` enables implementation

**Risks:**
- Schema design for key namespace (workflow:, execution:, etc.)
- Migration in repo-root migrations/

**Exit criteria:**
- PostgresStorage::new(connection_string) works
- get/set/delete/exists against Postgres
- Tests with testcontainers or embedded Postgres

---

## Phase 2: Redis and S3 Backends

**Deliverables:**
- RedisStorage implementing Storage<String, Vec<u8>>
- S3Storage implementing Storage<String, Vec<u8>> (key = S3 key path)
- Features redis, s3
- Optional: list/prefix scan for Redis (SCAN), S3 (list_objects_v2)

**Risks:**
- Redis connection management; S3 bucket/prefix config
- TTL for Redis — add to trait or backend-specific?

**Exit criteria:**
- RedisStorage, S3Storage work with get/set/delete/exists
- Integration tests

---

## Phase 3: List and Prefix Scan

**Deliverables:**
- `list_prefix(&self, prefix: &str) -> Result<Vec<Key>, StorageError>` or `ListableStorage` trait
- Implementations for Memory, Postgres, Redis, S3
- Pagination if needed (limit/offset or cursor)

**Risks:**
- API design; some backends (S3) have different list semantics
- Performance for large key spaces

**Exit criteria:**
- Consumers can list keys by prefix
- All backends support list

---

## Phase 4: Transactions and Caching

**Deliverables:**
- Optional `TransactionalStorage` or `transaction<F>(&self, f: F)` for Postgres
- Read-through cache wrapper (CachedStorage<S>) — see archive
- TTL support for Redis/Memory

**Risks:**
- Transaction API complexity; async transaction boundaries
- Cache invalidation

**Exit criteria:**
- Postgres transactions work
- Optional cache layer
- Documentation for cache usage

---

## Metrics of Readiness

| Metric | Target |
|--------|--------|
| **Correctness** | All backends pass same contract tests |
| **Latency** | get/set < 10ms (local); configurable timeouts |
| **Throughput** | Scale with connection pool |
| **Stability** | No panics; errors propagated |
| **Operability** | Connection health; pool stats |
