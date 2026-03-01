# Architecture

## Problem Statement

- **Business problem:** Nebula needs persistent storage for workflows, executions, credentials, and binary data. Different deployments use different backends (Postgres, Redis, S3, local).
- **Technical problem:** Provide a unified key-value abstraction that allows swapping backends without changing consumer code. Support both binary (Vec<u8>) and typed (serde) values.

## Current Architecture

### Module Map

| Location | Responsibility |
|----------|----------------|
| `storage` (mod) | Storage trait — get, set, delete, exists |
| `backend/mod.rs` | Backend exports |
| `backend/memory.rs` | MemoryStorage, MemoryStorageTyped |
| `error.rs` | StorageError — NotFound, Serialization, Backend |

### Data/Control Flow

1. Consumer holds `Arc<dyn Storage<Key = K, Value = V>>` or concrete `MemoryStorage` / `MemoryStorageTyped<T>`.
2. `get(key)` → `Ok(None)` absent, `Ok(Some(v))` found; `set` overwrites; `delete` idempotent; `exists` lock-free read.
3. `MemoryStorage` internals: `RwLock<HashMap<String, Vec<u8>>>` — concurrent reads via shared lock, exclusive writes.
4. `MemoryStorageTyped<T>` wraps `MemoryStorage` transparently: `set` calls `serde_json::to_vec` then delegates to raw backend; `get` delegates then calls `serde_json::from_slice`. All serialization errors surface as `StorageError::Serialization`.
5. `MemoryStorageTyped<T>` uses `PhantomData<fn() -> T>` — covariant in `T`, zero runtime cost.

### Known Bottlenecks

- **No persistent backends:** Only MemoryStorage implemented; postgres/redis/s3 are optional deps, no impl
- **No list/scan:** Cannot enumerate keys; limits use cases (e.g. list workflows)
- **No transactions:** Single-key ops only
- **Key type fixed:** MemoryStorage uses String; generic Storage allows other Key types

## Target Architecture

### Target Module Map

```
nebula-storage/
├── storage.rs      — Storage trait (current)
├── error.rs        — StorageError (current)
├── backend/
│   ├── memory.rs   — MemoryStorage, MemoryStorageTyped (current)
│   ├── postgres.rs — (Phase 2) PostgresStorage
│   ├── redis.rs    — (Phase 2) RedisStorage
│   └── s3.rs       — (Phase 2) S3Storage
└── list.rs         — (Phase 2) List/prefix scan extension
```

### Public Contract Boundaries

- `Storage<Key, Value>` — get, set, delete, exists; async; Send + Sync
- `StorageError` — NotFound, Serialization, Backend
- Backends implement Storage for specific Key/Value (e.g. String/Vec<u8>, WorkflowId/Workflow)

### Internal Invariants

- get returns None when key absent (not NotFound error for optional get)
- set overwrites; delete is idempotent
- Serialization errors map to StorageError::Serialization

## Design Reasoning

### Key Trade-off 1: Generic vs domain-specific trait

- **Current:** Generic Storage<Key, Value>; consumers choose types.
- **Archive:** StorageBackend with save_workflow, load_execution, etc. — domain-specific.
- **Decision:** Generic key-value first; domain layers (workflow, execution) build on top. Simpler; more reusable.

### Key Trade-off 2: Binary vs typed

- **MemoryStorage:** String → Vec<u8>; raw bytes.
- **MemoryStorageTyped<T>:** String → T; serde_json. Covers most use cases.
- **Consequence:** Two backends; typed wraps raw. S3/Redis may prefer binary; Postgres may use JSONB.

### Rejected Alternatives

- **Single domain trait:** Too narrow; credential has its own StorageProvider.
- **Sync only:** Async required for I/O backends.

## Comparative Analysis

Sources: n8n, Temporal, Prefect, Redis, S3.

| Pattern | Verdict | Rationale |
|---------|---------|-----------|
| Key-value abstraction | **Adopt** | Redis, S3; simple, universal |
| Typed wrapper | **Adopt** | serde_json; workflow/execution as JSON |
| Optional backends | **Adopt** | Feature flags; minimal default deps |
| List/prefix scan | **Adopt** | Phase 2; needed for list workflows |
| Transactions | **Defer** | Phase 2; Postgres supports; Redis has MULTI |
| Domain-specific (WorkflowStorage) | **Defer** | Build on Storage; ports or workflow crate |

## Breaking Changes (if any)

- None planned; trait is minimal.

## Open Questions

- Q1: List/prefix scan — extend Storage trait or separate ListableStorage?
- Q2: TTL support for Redis/Memory — add to trait or backend-specific?
