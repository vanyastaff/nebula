# Design: List/prefix, transactions, and caching

This document summarizes the intended design for list/prefix scan, transactional operations, and caching in `nebula-storage` and their use by credential (and other) consumers. Implementation is phased; see [ROADMAP.md](./ROADMAP.md).

## List / prefix scan

- **Trait:** Add optional `ListableStorage` extension trait with `list_prefix(&self, prefix: &str) -> Result<Vec<Self::Key>, StorageError>`.
- **Backends:** Memory: filter in-memory keys by prefix; Postgres: `SELECT key FROM storage_kv WHERE key LIKE $1` (parameterized); Redis: `SCAN` with pattern; S3: `list_objects_v2` with prefix.
- **Credential use:** `PostgresStorageProvider::list` can call `list_prefix("cred:")` and parse UUIDs from keys to return `Vec<CredentialId>`. Until `ListableStorage` exists, the provider returns an empty list.
- **Pagination:** For large key spaces, add optional `list_prefix_paginated(prefix, limit, cursor)` returning `(Vec<Key>, Option<Cursor>)` in a later phase.

## Transactions

- **Scope:** Postgres (and any backend that supports multi-statement transactions). Not required for Redis/S3 in the first iteration.
- **API option A:** Extension trait `TransactionalStorage` with `async fn transaction<F, R>(&self, f: F) -> Result<R, StorageError>` where `F: FnOnce(&mut TransactionHandle) -> Future<Output = Result<R, StorageError>>`. The handle exposes `get/set/delete` that run in the same transaction.
- **API option B:** `PostgresStorage` method `begin_transaction() -> Transaction`; caller calls `tx.set(...)`, `tx.delete(...)`, then `tx.commit().await`.
- **Credential use:** Rotation can persist new credential state and rotation bookkeeping in one transaction to avoid partial writes. Optional; credential can continue without it (eventual consistency).

## Caching

- **Pattern:** Read-through (and optionally write-through) cache wrapper: `CachedStorage<S: Storage>` that holds `S` and a cache (e.g. in-memory LRU or Redis). On `get`, check cache; on miss, call `S::get`, then populate cache. On `set`/`delete`, invalidate (or update) cache and delegate to `S`.
- **Configuration:** TTL per key or global; max cache size; key prefix to cache (e.g. only `cred:`).
- **Credential use:** `CredentialManager` or a wrapper can use `CachedStorage<PostgresStorage>` to reduce DB load for hot credentials. Cache key = storage key; value = raw bytes. Invalidation on `store`/`delete` for that key.
- **Placement:** Implement in `nebula-storage` as a wrapper type, or in `nebula-credential` as a provider wrapper. Prefer storage crate so other consumers (workflow, execution) can reuse.

## Integration summary

| Feature        | Storage crate          | Credential use                          |
|----------------|------------------------|-----------------------------------------|
| List/prefix    | `ListableStorage` (P001) | `PostgresStorageProvider::list`         |
| Transactions   | `TransactionalStorage` or `PostgresStorage::transaction` (Phase 4) | Rotation atomicity (optional) |
| Caching        | `CachedStorage<S>` (Phase 4) or credential-side wrapper | Hot credential reads |

No code changes required in this step; this document records the design for future implementation.
