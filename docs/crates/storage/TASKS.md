# Tasks: nebula-storage

**Roadmap**: [ROADMAP.md](ROADMAP.md) | **Plan**: [PLAN.md](PLAN.md)

## Legend

- `[P]` — Can run in parallel with other `[P]` tasks in same phase
- `STG-TXXX` — Task ID
- `→` — Depends on previous task

---

## Phase 1: Postgres Backend (In Progress)

**Goal**: Implement PostgresStorage as the first durable backend.

- [ ] STG-T001 [P] Define storage_kv table schema (key TEXT PRIMARY KEY, value JSONB, updated_at TIMESTAMPTZ)
- [ ] STG-T002 [P] Create sqlx migration in `migrations/` for storage_kv table
- [ ] STG-T003 Implement PostgresStorage struct with PgPool connection pooling (`crates/storage/src/postgres.rs`) (→ T001)
- [ ] STG-T004 Implement `get` method for PostgresStorage (→ T003)
- [ ] STG-T005 Implement `set` method for PostgresStorage (→ T003)
- [ ] STG-T006 Implement `delete` method for PostgresStorage (→ T003)
- [ ] STG-T007 Implement `exists` method for PostgresStorage (→ T003)
- [ ] STG-T008 Implement `PostgresStorage::new(connection_string)` constructor (→ T003)
- [ ] STG-T009 Write integration tests for get/set/delete/exists against Postgres (→ T004, T005, T006, T007)
- [ ] STG-T010 Verify feature flag `postgres` correctly gates implementation (→ T003)

**Checkpoint**: PostgresStorage::new(connection_string) works; get/set/delete/exists pass against Postgres.

---

## Phase 2: Redis and S3 Backends

**Goal**: Add Redis and S3 as additional storage backends.

- [ ] STG-T011 [P] Implement RedisStorage struct with connection management (`crates/storage/src/redis.rs`)
- [ ] STG-T012 [P] Implement S3Storage struct with bucket/prefix config (`crates/storage/src/s3.rs`)
- [ ] STG-T013 Implement get/set/delete/exists for RedisStorage (→ T011)
- [ ] STG-T014 Implement get/set/delete/exists for S3Storage (→ T012)
- [ ] STG-T015 Write integration tests for RedisStorage (→ T013)
- [ ] STG-T016 Write integration tests for S3Storage (→ T014)
- [ ] STG-T017 Verify feature flags `redis` and `s3` correctly gate implementations (→ T011, T012)

**Checkpoint**: RedisStorage and S3Storage work with get/set/delete/exists; integration tests pass.

---

## Phase 3: List and Prefix Scan

**Goal**: Enable key enumeration by prefix across all backends.

- [ ] STG-T018 Design `ListableStorage` trait or `list_prefix` method (`crates/storage/src/lib.rs`)
- [ ] STG-T019 Implement list_prefix for MemoryStorage (→ T018)
- [ ] STG-T020 Implement list_prefix for PostgresStorage (→ T018)
- [ ] STG-T021 Implement list_prefix for RedisStorage using SCAN (→ T018)
- [ ] STG-T022 Implement list_prefix for S3Storage using list_objects_v2 (→ T018)
- [ ] STG-T023 Add pagination support (limit/offset or cursor) if needed (→ T019, T020, T021, T022)
- [ ] STG-T024 Write contract tests verifying all backends support list (→ T019, T020, T021, T022)

**Checkpoint**: Consumers can list keys by prefix; all backends support list.

---

## Phase 4: Transactions and Caching

**Goal**: Add transactional support and optional caching layer.

- [ ] STG-T025 [P] Design TransactionalStorage trait or `transaction<F>` method
- [ ] STG-T026 [P] Design CachedStorage<S> read-through cache wrapper
- [ ] STG-T027 Implement TransactionalStorage for PostgresStorage (→ T025)
- [ ] STG-T028 Implement CachedStorage<S> wrapper (→ T026)
- [ ] STG-T029 [P] Add TTL support for RedisStorage
- [ ] STG-T030 [P] Add TTL support for MemoryStorage
- [ ] STG-T031 Write tests for Postgres transactions (→ T027)
- [ ] STG-T032 Write tests for CachedStorage behavior (→ T028)
- [ ] STG-T033 Document cache usage and TTL configuration (→ T028, T029, T030)

**Checkpoint**: Postgres transactions work; optional cache layer available; documentation complete.

---

## Dependencies & Execution Order

- Phase 1 → Phase 2 → Phase 3 → Phase 4
- [P] tasks within a phase can run in parallel

## Verification (after all phases)

- [ ] `cargo check -p nebula-storage --all-features`
- [ ] `cargo test -p nebula-storage`
- [ ] `cargo clippy -p nebula-storage -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-storage`
