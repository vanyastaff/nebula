# nebula-storage

Key-value storage abstraction for Nebula: pluggable backends (memory, Postgres, Redis, S3).

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

- **Maturity:** MVP — Storage trait, MemoryStorage, MemoryStorageTyped; no persistent backends implemented yet
- **Key strengths:** Simple key-value abstraction; typed wrapper for JSON; optional features for postgres/redis/s3
- **Key risks:** Postgres, Redis, S3 backends are optional deps only — no implementations in crate yet; no list/scan

## Target State

- **Production criteria:** PostgresStorage, RedisStorage, S3Storage implementations; optional list/prefix scan; transaction support
- **Compatibility guarantees:** Storage trait additive-only; Key/Value associated types stable

## Document Map

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

## Archive

Legacy material:
- [`_archive/`](./_archive/)
