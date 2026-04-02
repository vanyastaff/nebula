# Migration

## Versioning Policy

- **compatibility promise:** Patch/minor preserve `StorageProvider`, `CredentialProvider`, `CredentialContext`, error variants, protocol types
- **deprecation window:** Minimum 6 months before removal of deprecated APIs (unless security-critical)

## Breaking Changes

- **currently planned:** None committed

- **potential future breaking candidates:**

| Proposal | Change | Impact |
|----------|--------|--------|
| P-001 | Provider capability negotiation | Startup validation changes; providers must implement `capabilities()` |
| P-002 | Strict scope enforcement mode | Operations without scope context may fail in strict mode |
| P-003 | Rotation policy versioning | Schema envelope change for persisted policies |
| P-007 | EncryptionProvider trait | `CredentialManagerBuilder` gains `encryption_provider` setter |
| P-009 | Credential lifecycle state machine | Status field becomes validated enum; invalid transitions rejected |
| D-008 | PKCE mandatory for OAuth2 | OAuth2 flows without PKCE will fail |
| D-010 | API key format (sk_ prefix) | Existing API keys without prefix need migration |

## Migration Path: Storage Provider (P-001)

```rust
// Before: minimal StorageProvider
impl StorageProvider for MyProvider {
    async fn store(...) { ... }
    async fn retrieve(...) { ... }
    async fn delete(...) { ... }
    async fn list(...) { ... }
}

// After: must also implement capabilities()
impl StorageProvider for MyProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            cas: true,          // compare-and-swap support
            list_filter: true,  // filtered listing support
            metrics: true,      // storage metrics support
            ..Default::default()
        }
    }
    // ... existing methods unchanged
}
```

## Migration Path: EncryptionProvider (P-007)

```rust
// Before: CredentialManager uses built-in AES-256-GCM
let manager = CredentialManager::builder()
    .storage(provider)
    .encryption_key(key)
    .build()?;

// After: explicit encryption provider
let manager = CredentialManager::builder()
    .storage(provider)
    .encryption_provider(LocalEncryptionProvider::new(key))
    // or: .encryption_provider(KmsEncryptionProvider::new(kms_client))
    .build()?;
```

## Migration to Postgres-backed storage (DB storage)

To move credentials from another provider (e.g. local filesystem or mock) to database-backed storage:

1. **Prerequisites**
   - PostgreSQL with the shared KV table (run repo-root migrations including `storage_kv`; see `migrations/` and [nebula-storage ROADMAP](../storage/ROADMAP.md)).
   - `nebula-storage` built with feature `postgres` and `nebula-credential` with feature `storage-postgres`.

2. **Create Postgres storage and provider**
   - Build KV storage: `nebula_storage::PostgresStorage::new(config).await` (e.g. from `PostgresStorageConfig` with `database_url`, `table: "storage_kv"`).
   - Wrap in credential provider: `PostgresStorageProvider::new(Arc::new(postgres_storage))`.

3. **Data migration (application-level)**
   - **Export:** Using the current provider, list credentials (e.g. `manager.list_ids(...)`) and for each ID call `retrieve`; persist the returned `(EncryptedData, CredentialMetadata)` in a safe format (e.g. temp files or in-memory list).
   - **Import:** With the new `PostgresStorageProvider` set on the manager (or a second manager instance), for each exported credential call `store(id, data, metadata, &context)`.
   - **Verify:** List credentials from the new provider (note: `PostgresStorageProvider::list` currently returns empty until `ListableStorage` is available; use `exists` per ID or a separate index if you need to verify).

4. **Switch configuration**
   - Point `CredentialManager::builder().with_storage(...)` to the new `PostgresStorageProvider` and deploy. Ensure the same encryption key is used so existing encrypted payloads remain readable.

5. **Rollback**
   - Keep the previous provider and config until verification is complete. To roll back, reconfigure the manager to use the old provider; no automatic data copy-back.

## Rollout Plan

1. **Preparation:** Introduce new APIs additively; document migration path
2. **Dual-run / feature-flag stage:** Allow old and new behavior side-by-side (e.g. strict scope mode opt-in)
3. **Cutover:** Switch defaults only in major release
4. **Cleanup:** Remove deprecated path after migration window

## Rollback Plan

- **Trigger conditions:** Consumer breakage in provider/manager contracts; rotation state corruption; encryption key rotation failure
- **Rollback steps:** Revert to previous stable version; restore encryption key if rotated; verify credential decryption
- **Data/state reconciliation:** Ensure persisted credentials remain decryptable; rotation state recoverable from backup

## Validation Checklist

- **API compatibility:** Compile-time checks for `StorageProvider`, `CredentialProvider`, `CredentialManager` signatures
- **Integration:** Action/resource fixtures; all provider implementations pass unified test suite
- **Performance:** Benchmark comparison; cache hit rate preserved; no latency regression
- **Security:** Encryption round-trip; scope enforcement; no secret leakage in new code paths
- **Serialization:** Persisted `CredentialMetadata` and `RotationPolicy` schemas backward-compatible (or migrated)
