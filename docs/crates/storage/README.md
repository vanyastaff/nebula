# nebula-storage

Key-value storage abstraction for Nebula: pluggable backends (memory, Postgres, Redis, S3).

**Status:** `Storage` trait + in-memory backend and **Postgres backend** (`PostgresStorage`, feature `postgres`) are implemented; Redis/S3 are still design-only.

## Scope

- **In scope:**
  - `Storage` trait — generic key-value: get, set, delete, exists
  - `MemoryStorage` — in-memory backend (String → Vec<u8>)
  - `MemoryStorageTyped<T>` — typed wrapper with serde_json (String → T)
  - `StorageError` — NotFound, Serialization, Backend
  - Optional backends via features: postgres, redis, s3

- **Out of scope:**
  - Credential storage (see `nebula-credential` StorageProvider)
  - Workflow/execution domain storage (consumers build on Storage trait)
  - Query system, transactions (planned; see ROADMAP)
  - Migration management (see repo-root migrations/)

## Current State

- **Maturity:** Postgres backend implemented (feature `postgres`); MemoryStorage, MemoryStorageTyped; Redis/S3 not yet
- **Key strengths:** Simple key-value abstraction; typed wrapper for JSON; PostgresStorage with `storage_kv` table; optional features
- **Key risks:** No list/prefix yet (see [DESIGN_LIST_TX_CACHE.md](./DESIGN_LIST_TX_CACHE.md)); Redis/S3 pending

## Target State

- **Production criteria:** PostgresStorage, RedisStorage, S3Storage implementations; optional list/prefix scan; transaction support
- **Compatibility guarantees:** Storage trait additive-only; Key/Value associated types stable

## Document Map

- [CONSTITUTION.md](./CONSTITUTION.md) — platform role, principles, production vision
- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [INTERACTIONS.md](./INTERACTIONS.md)
- [DECISIONS.md](./DECISIONS.md)
- [ROADMAP.md](./ROADMAP.md)
- [PROPOSALS.md](./PROPOSALS.md)
- [SECURITY.md](./SECURITY.md)
- [RELIABILITY.md](./RELIABILITY.md)
- [TEST_STRATEGY.md](./TEST_STRATEGY.md)
- [MIGRATION.md](./MIGRATION.md)
- [DESIGN_LIST_TX_CACHE.md](./DESIGN_LIST_TX_CACHE.md) — design for list/prefix, transactions, caching

## Archive

Legacy material:
- [`_archive/`](./_archive/)
