# PostgresStorageProvider — Key/Value Layout and Error Mapping

This document specifies how `PostgresStorageProvider` in `nebula-credential` uses `nebula-storage`'s generic KV layer to persist credentials.

## Overview

- **Backend:** `PostgresStorageProvider` holds `Arc<dyn nebula_storage::Storage<Key = String, Value = Vec<u8>>>` (typically `PostgresStorage` from `nebula-storage` with feature `postgres`).
- **Scope enforcement:** Not enforced in the provider; the manager layer performs scope checks via `retrieve_scoped` / `list_scoped` (see [SCOPE_ENFORCEMENT.md](./SCOPE_ENFORCEMENT.md)).
- **Encryption boundary:** Encrypted blobs and metadata are produced/consumed in `nebula-credential`; the DB stores only opaque bytes.

## Key Scheme

- **Format:** `cred:{id}` where `id` is the UUID of `CredentialId` (e.g. `cred:550e8400-e29b-41d4-a716-446655440000`).
- **Scope:** Scope is stored inside the value (metadata); it is not part of the key. Listing and scope filtering are done at the manager layer after loading metadata.
- **Collisions:** Single key per credential; no collision (UUID is unique).

## Value Schema

- **Format:** JSON, same as `LocalStorageProvider`, for compatibility and debuggability.
- **Structure:** One JSON object per credential:

```json
{
  "version": 1,
  "encrypted_data": {
    "version": 1,
    "nonce": "<base64>",
    "ciphertext": "<base64>",
    "tag": "<base64>"
  },
  "metadata": {
    "created_at": "...",
    "last_accessed": null,
    "last_modified": "...",
    "owner_scope": null,
    "rotation_policy": null,
    "version": 1,
    "expires_at": null,
    "ttl_seconds": null,
    "tags": {}
  },
  "salt": null
}
```

- **Serialization:** `CredentialFile` (same as local provider): `version`, `encrypted_data` (`EncryptedData`), `metadata` (`CredentialMetadata`), optional `salt`. Serialized with `serde_json::to_vec`; stored as `Vec<u8>` in `Storage::set`.
- **Deserialization:** `Storage::get` returns `Option<Vec<u8>>`; provider deserializes with `serde_json::from_slice` into `CredentialFile`, then returns `(EncryptedData, CredentialMetadata)`.

## Error Mapping

| `nebula_storage::StorageError` | `nebula_credential::core::StorageError` |
|--------------------------------|----------------------------------------|
| `NotFound`                     | Not used (KV `get` returns `Ok(None)`); provider maps `None` to `StorageError::NotFound { id }`. |
| `Serialization(_)`             | Treated as corrupt data: `StorageError::ReadFailure { id, source }` with `source = std::io::Error::new(InvalidData, msg)`. |
| `Backend(s)`                   | `StorageError::ReadFailure` or `StorageError::WriteFailure` with `id` and `source = std::io::Error::new(Other, s)` (for retrieve vs set/delete). |

- **get returns None** → `StorageError::NotFound { id: id.to_string() }`.
- **get returns Some(bytes)** but `serde_json::from_slice` fails → `StorageError::ReadFailure { id, source }`.
- **set/delete** fails with Backend → `StorageError::WriteFailure { id, source }`.

## List Semantics

- **Current:** `Storage` has no `list_prefix`; provider cannot list keys from KV. Options:
  - **Temporary:** `list` returns empty `Vec` and document that listing requires a future `ListableStorage` or separate index; or
  - **Full scan:** If we add an optional `list_keys` to a wrapper or use a separate metadata table (out of scope for minimal provider).
- **Recommendation (minimal):** Implement `list` by holding an in-memory set of known credential IDs for the process lifetime (e.g. populated on store/delete), or return empty until `ListableStorage` is available. Document in API that Postgres provider may not support full list without list_prefix.

For the first version we implement `list` by scanning all keys with a prefix `cred:` once we add `list_prefix` to storage; until then we can return empty or add a best-effort path (e.g. optional secondary table). Spec updated: we will use a **prefix scan** when available; until then `list` returns empty and callers rely on manager-level indexing or other mechanisms.

## Rotation State

- `store_rotation_state` / `get_rotation_state` / `delete_rotation_state`: use same KV with key format `rotation:{transaction_id}`; value = JSON serialized state. Same error mapping as above.

## Configuration

- **PostgresStorageProvider** is constructed with:
  - `storage: Arc<dyn Storage<Key = String, Value = Vec<u8>>>` (e.g. from `nebula_storage::PostgresStorage::new(config).await`),
  - optional key prefix (default `cred:` for credentials, `rotation:` for rotation state).
- Database URL and pool config live in `nebula-storage`'s `PostgresStorageConfig`; credential crate does not duplicate them.
