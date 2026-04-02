# Implementation Plan: nebula-storage

**Crate**: `nebula-storage` | **Path**: `crates/storage` | **Roadmap**: [ROADMAP.md](ROADMAP.md)

## Summary

nebula-storage provides a key-value storage abstraction with pluggable backends (memory, Postgres, Redis, S3) for the Nebula workflow engine. Phase 1 (Postgres backend) is currently in progress; subsequent phases add Redis/S3 backends, list/prefix scan, and transactions/caching.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio
**Key Dependencies**: async-trait, thiserror, serde, serde_json, tokio, nebula-core, sqlx (optional, postgres), redis (optional), aws-config + aws-sdk-s3 (optional, s3)
**Testing**: `cargo test -p nebula-storage`

## Current Status

| Phase | Status | Notes |
|-------|--------|-------|
| Phase 1: Postgres Backend | 🔄 In Progress | PostgresStorage, storage_kv migration, connection pooling |
| Phase 2: Redis and S3 Backends | ⬜ Planned | RedisStorage, S3Storage, feature flags |
| Phase 3: List and Prefix Scan | ⬜ Planned | ListableStorage trait, pagination |
| Phase 4: Transactions and Caching | ⬜ Planned | TransactionalStorage, CachedStorage, TTL |

## Phase Details

### Phase 1: Postgres Backend (In Progress)

**Goal**: Implement PostgresStorage as the first durable backend.

**Deliverables**:
- PostgresStorage implementing Storage<String, Vec<u8>> or Storage<String, serde_json::Value>
- Key-value table (storage_kv: key TEXT PRIMARY KEY, value JSONB, updated_at TIMESTAMPTZ)
- Connection pooling via sqlx PgPool
- Feature `postgres` enables implementation

**Exit Criteria**:
- PostgresStorage::new(connection_string) works
- get/set/delete/exists against Postgres
- Tests with testcontainers or embedded Postgres

**Risks**:
- Schema design for key namespace (workflow:, execution:, etc.)
- Migration in repo-root migrations/

### Phase 2: Redis and S3 Backends

**Goal**: Add Redis and S3 as additional storage backends.

**Deliverables**:
- RedisStorage implementing Storage<String, Vec<u8>>
- S3Storage implementing Storage<String, Vec<u8>> (key = S3 key path)
- Features `redis`, `s3`
- Optional: list/prefix scan for Redis (SCAN), S3 (list_objects_v2)

**Exit Criteria**:
- RedisStorage, S3Storage work with get/set/delete/exists
- Integration tests

**Risks**:
- Redis connection management; S3 bucket/prefix config
- TTL for Redis -- add to trait or backend-specific?

### Phase 3: List and Prefix Scan

**Goal**: Enable key enumeration by prefix across all backends.

**Deliverables**:
- `list_prefix(&self, prefix: &str) -> Result<Vec<Key>, StorageError>` or `ListableStorage` trait
- Implementations for Memory, Postgres, Redis, S3
- Pagination if needed (limit/offset or cursor)

**Exit Criteria**:
- Consumers can list keys by prefix
- All backends support list

**Risks**:
- API design; some backends (S3) have different list semantics
- Performance for large key spaces

### Phase 4: Transactions and Caching

**Goal**: Add transactional support and optional caching layer.

**Deliverables**:
- Optional `TransactionalStorage` or `transaction<F>(&self, f: F)` for Postgres
- Read-through cache wrapper (CachedStorage<S>)
- TTL support for Redis/Memory

**Exit Criteria**:
- Postgres transactions work
- Optional cache layer
- Documentation for cache usage

**Risks**:
- Transaction API complexity; async transaction boundaries
- Cache invalidation

## Dependencies

| Depends On | Why |
|-----------|-----|
| nebula-core | Core identifiers and type definitions |

| Depended By | Why |
|------------|-----|
| nebula-credential | Uses storage backends for credential persistence |

## Verification

- [ ] `cargo check -p nebula-storage`
- [ ] `cargo test -p nebula-storage`
- [ ] `cargo clippy -p nebula-storage -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-storage`
