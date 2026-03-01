# nebula-storage Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Nebula needs to persist data: workflow definitions, execution state, credentials (via nebula-credential's StorageProvider), caches, and binary blobs. Different environments need different backends: in-memory for tests, Postgres for production, Redis for speed, S3 for large objects. A single abstraction lets the rest of the platform stay backend-agnostic.

**nebula-storage is the key-value storage abstraction for the Nebula platform.**

It answers: *How do other crates read and write key-value data (and optionally typed JSON) without depending on a specific database or service?*

```
caller (credential, engine, cache, etc.) holds Arc<dyn Storage> or typed StorageTyped<T>
    ↓
get(key) / set(key, value) / delete(key) / exists(key)
    ↓
Storage implementation: MemoryStorage | PostgresStorage | RedisStorage | S3Storage (optional features)
    ↓
same API for tests (memory) and production (postgres/redis/s3)
```

This is the storage contract: key-value with optional list/prefix scan and transactions as they are added; credential storage uses StorageProvider (credential crate), which may wrap this trait.

---

## User Stories

### Story 1 — Credential Crate Stores Encrypted Blobs (P1)

nebula-credential needs to store and retrieve encrypted credential state by key. It uses a StorageProvider (which may be implemented on top of nebula-storage). The storage backend might be local file, Postgres, or a cloud KV store.

**Acceptance**:
- Storage trait supports get/set/delete/exists
- Credential crate uses its own StorageProvider abstraction; storage crate provides implementations (e.g. PostgresStorage) that can back it
- No credential-specific types in nebula-storage; only bytes or typed JSON where applicable

### Story 2 — Engine or Runtime Caches Workflow/Execution Metadata (P2)

A consumer needs to cache workflow or execution metadata by key (e.g. workflow_id, execution_id). It uses Storage or StorageTyped<T> so that the same code runs with in-memory backend in tests and Postgres or Redis in production.

**Acceptance**:
- MemoryStorage and MemoryStorageTyped available for tests
- Postgres/Redis/S3 backends available via features; same get/set/delete API
- Errors are StorageError (NotFound, Serialization, Backend) for retry/classification

### Story 3 — Operator Chooses Backend Without Code Change (P2)

Deployment chooses storage backend via configuration. Application code uses the Storage trait only; no backend-specific logic in business crates.

**Acceptance**:
- Backend selection is configuration + factory (e.g. build PostgresStorage from config)
- No hardcoded backend in storage crate's public API; callers receive dyn Storage or typed wrapper

### Story 4 — Migrations and Schema Are Out of Scope (P3)

Table schema and SQL migrations for workflow/execution storage live at repo root (migrations/) or in owning crates. nebula-storage provides the key-value abstraction, not the domain schema.

**Acceptance**:
- Storage trait is key-value (and optional list/prefix); no workflow/execution-specific methods
- Domain storage (e.g. workflow definitions) is built on top of Storage by engine or api crate
- Migrations are documented in docs/database-migrations.md and not owned by storage crate

---

## Core Principles

### I. Trait-Based Backend Abstraction

**Storage is a trait (async where applicable); backends implement it. Callers depend on the trait, not on concrete backends.**

**Rationale**: Tests use in-memory backend; production uses Postgres or Redis. Swapping backends must not require changes in credential, engine, or cache code.

**Rules**:
- Public API is Storage trait + constructors for each backend (Memory, Postgres, Redis, S3)
- No backend-specific types in method signatures of consumers
- Optional features so minimal builds do not pull unused backends

### II. Key-Value Semantics First

**Core contract is key-value: get, set, delete, exists. List/prefix/scan and transactions are additive.**

**Rationale**: Key-value covers the majority of use cases (credential state, cache, blob refs). Rich query and transactions can be added without breaking the core contract.

**Rules**:
- Storage trait has at least get/set/delete/exists
- Additive methods (list_keys, prefix_scan, transaction) follow same abstraction
- No SQL or query language in the trait

### III. Errors Are Classifiable

**StorageError allows callers to distinguish NotFound, Serialization, and Backend errors for retry and logging.**

**Rationale**: Retry policy (e.g. resilience crate) needs to know if failure is transient (backend) or permanent (not found, serialization). Fatal vs retryable should be derivable.

**Rules**:
- StorageError has clear variants
- Backend errors can carry cause or code for logging
- Document which errors are retryable

### IV. Typed Layer Is Optional and Clear

**StorageTyped<T> or equivalent provides get/set with serde_json (or similar) so callers can work with typed structs. Raw byte storage remains available.**

**Rationale**: Many consumers store JSON; typed API reduces boilerplate. Raw bytes are still needed for binary or credential blobs.

**Rules**:
- Typed API is a wrapper over key-value; no separate "typed backend"
- Serialization errors are StorageError::Serialization

### V. No Domain Logic in Storage Crate

**Storage does not know about workflows, executions, or credentials. It stores keys and values.**

**Rationale**: Workflow and execution schema belong to engine/api; credential encryption and scope belong to credential crate. Storage is infrastructure only.

**Rules**:
- No WorkflowId or ExecutionId in storage trait
- Domain crates build their keys (e.g. "workflow:{}", "credential:{}") and use Storage
- Migrations and schema are owned by domain or repo-root

### VI. Credential Storage Is Separate Abstraction

**Credential crate defines StorageProvider for its own needs; storage crate may provide implementations that implement that trait.**

**Rationale**: Credential storage has different guarantees (encryption, scope) and is owned by nebula-credential. nebula-storage is a general-purpose KV layer that can back it.

**Rules**:
- Storage trait is generic key-value
- If credential uses storage crate, it is via an adapter or a dedicated StorageProvider impl in credential crate that wraps Storage
- No credential-specific methods in nebula-storage

---

## Production Vision

### The storage layer in an n8n-class fleet

In a production Nebula deployment, multiple services use storage: credential store (encrypted blobs), engine/runtime (cache or metadata), optional binary store (large payloads). Each may use a different backend: Postgres for durable state, Redis for hot cache, S3 for binaries. The same Storage trait is used everywhere; only the implementation and configuration change.

```
Services
    │
    ├── Credential service: StorageProvider (wrapping PostgresStorage or custom)
    │       → encrypted credential state by key
    │
    ├── Engine/Runtime: Storage or StorageTyped for workflow/execution cache
    │       → MemoryStorage in dev, RedisStorage or PostgresStorage in prod
    │
    └── Binary/Blob service: Storage (e.g. S3Storage) for large objects
            → key = blob id, value = ref or metadata; body in object store
```

Backends are selected by config; connection pooling and health checks are backend-specific but behind the same trait. Observability (metrics, tracing) can be added via wrapper or middleware without changing the trait.

### From the archives: backends and responsibilities

The archive `_archive/archive-nebula-complete.md` lists StorageBackend, WorkflowStorage, ExecutionStorage, BinaryStorage as separate concepts with async methods and transaction support. The archive `_archive/from-archive/nebula-complete-docs-part3/nebula-storage.md` (if present) and current README align on: abstraction over backends, key-value first, optional Postgres/Redis/S3. Production vision: keep a single Storage trait as the core; WorkflowStorage/ExecutionStorage can be built on top by engine/api using the same Storage abstraction. BinaryStorage may be a dedicated backend (S3) or a key-value namespace. Transaction support is a gap to close for production.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|----------|-------|
| PostgresStorage / RedisStorage / S3Storage implementations | Critical | Currently optional deps only; no impl in crate yet |
| List/prefix scan API | High | Many consumers need "list keys by prefix" |
| Transaction support (optional) | High | Required for atomic credential or state updates |
| Connection pooling and health check | Medium | Backend-specific; document or provide in impl |
| Observability (metrics/tracing) | Medium | Wrapper or middleware for latency and error rates |
| Migration story for backend switch | Low | Document how to move data between backends |

---

## Key Decisions

### D-001: Single Storage Trait, Not Per-Domain Traits

**Decision**: One Storage trait (key-value) that domain crates use. Workflow/execution/credential semantics are in their crates.

**Rationale**: Avoids trait proliferation and keeps storage crate free of domain types. Domain crates compose Storage with their key layout and types.

**Rejected**: WorkflowStorage, ExecutionStorage in storage crate — would pull domain into infrastructure.

### D-002: Optional Backends Behind Features

**Decision**: Memory backend is default; Postgres, Redis, S3 are optional features.

**Rationale**: Minimal builds (e.g. CLI or tests) do not need Postgres. Production enables the features it uses.

**Rejected**: All backends always compiled — would increase build time and dependency surface.

### D-003: StorageError as the Single Error Type

**Decision**: All storage operations return Result<T, StorageError> with variants (NotFound, Serialization, Backend, etc.).

**Rationale**: Callers and resilience layer need to classify failures. One enum keeps conversion simple.

**Rejected**: Backend-specific error types only — would force callers to know every backend.

### D-004: No Built-in Migrations in Storage Crate

**Decision**: Schema and SQL migrations live at repo root or in domain crates. Storage crate does not own migration tooling.

**Rationale**: Migrations are domain-specific (workflow table vs credential table). Storage provides the abstraction; schema is consumer's responsibility.

**Rejected**: Storage crate defining workflow/execution tables — would mix infrastructure and domain.

---

## Open Proposals

### P-001: List Keys and Prefix Scan

**Problem**: Callers need to enumerate keys or list by prefix.

**Proposal**: Add `list_keys(&self, prefix: Option<&str>) -> Result<Vec<String>>` or streaming equivalent to Storage trait.

**Impact**: Additive; all backends must implement (memory: filter in-memory; postgres: LIKE or prefix index; redis: SCAN).

### P-002: Transaction or Batch Interface

**Problem**: Atomic multi-key updates (e.g. credential rotation state) require transactions.

**Proposal**: Add `transaction(&self, f: impl Fn(&mut Transaction) -> Result<()>)` or similar; backends that support it implement; others return error or emulate where possible.

**Impact**: May require async trait or two-phase API; breaking if trait changes.

### P-003: StorageProvider Adapter in Credential Crate

**Problem**: Credential crate needs StorageProvider; storage crate has Storage. Clarify who implements the bridge.

**Proposal**: Credential crate defines StorageProvider; storage crate provides PostgresStorage, etc. Credential crate has a thin adapter that wraps Storage and implements StorageProvider (key layout, scope prefixing).

**Impact**: No change to storage trait; credential crate gains explicit adapter.

---

## Non-Negotiables

1. **Storage is a trait** — callers depend on the trait, not concrete backends.
2. **Key-value core** — get, set, delete, exists are the base contract.
3. **StorageError is classifiable** — NotFound, Serialization, Backend for retry and observability.
4. **No domain types in storage crate** — no WorkflowId, ExecutionId, or credential types in trait.
5. **Credential storage is credential crate's abstraction** — storage crate does not define StorageProvider.
6. **Backends are optional features** — memory for tests; postgres/redis/s3 for production.
7. **Migrations and schema are out of scope** — owned by repo root or domain crates.

---

## Governance

- **PATCH**: Bug fixes, docs, internal refactors. No change to Storage trait or StorageError.
- **MINOR**: Additive only (new methods like list_keys, new backend impls). No removal or change of existing method semantics.
- **MAJOR**: Breaking changes to Storage trait or error type. Requires MIGRATION.md.

Every PR must verify: no domain-specific types in public API; backends remain behind features; credential crate's StorageProvider is not reimplemented here (only Storage implementations that can back it).
