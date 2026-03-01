# API

## Public Surface

### Stable APIs

- `Storage` trait — `get`, `set`, `delete`, `exists`; all async; `Send + Sync`
- `MemoryStorage` — `new()`, `Default`; `Storage<Key = String, Value = Vec<u8>>`
- `MemoryStorageTyped<T>` — `new()`, `Default`; `Storage<Key = String, Value = T>` where `T: Serialize + DeserializeOwned + Send + Sync`
- `StorageError` — `NotFound`, `Serialization(serde_json::Error)`, `Backend(String)`

### Planned (feature-gated)

- `PostgresStorage` (feature `postgres`)
- `RedisStorage` (feature `redis`)
- `S3Storage` (feature `s3`)
- List/prefix scan extension

## `Storage` Trait

```rust
#[async_trait]
pub trait Storage: Send + Sync {
    type Key: Send + Sync;
    type Value: Send + Sync;

    async fn get(&self, key: &Self::Key) -> Result<Option<Self::Value>, StorageError>;
    async fn set(&self, key: &Self::Key, value: &Self::Value) -> Result<(), StorageError>;
    async fn delete(&self, key: &Self::Key) -> Result<(), StorageError>;
    async fn exists(&self, key: &Self::Key) -> Result<bool, StorageError>;
}
```

Behavioral contracts:
- `get` returns `Ok(None)` when key is absent (not `StorageError::NotFound`)
- `set` always overwrites; no insert-only variant
- `delete` is idempotent — deleting a missing key is not an error
- `exists` is a lightweight check (no value returned)

## `StorageError`

```rust
pub enum StorageError {
    NotFound,                                    // key required to exist but missing
    Serialization(#[from] serde_json::Error),    // serde roundtrip failed
    Backend(String),                             // connection/timeout/backend failure
}
```

`Serialization` auto-converts from `serde_json::Error` via `From`. `Backend` errors may be retryable; `Serialization` and `NotFound` are not.

## Dependency Injection Pattern

Consumer code should accept `Arc<dyn Storage<Key = K, Value = V>>` to allow backend swapping:

```rust
struct WorkflowRepo {
    store: Arc<dyn Storage<Key = String, Value = Workflow>>,
}
```

In production, swap `MemoryStorageTyped` for `PostgresStorage` without changing `WorkflowRepo`.

## Key Naming Conventions

There is no enforced scheme — the consumer owns key format. Recommended convention:

| Consumer | Key format |
|---|---|
| workflow repo | `"workflow:{workflow_id}"` |
| execution repo | `"execution:{execution_id}"` |
| idempotency | `"idempotency:{key}"` |
| credential | uses `StorageProvider` trait (separate abstraction) |

## Usage Patterns

### Binary storage (`Vec<u8>`)

```rust
use nebula_storage::{MemoryStorage, Storage};

let store = MemoryStorage::new();
store.set(&"k1".to_string(), &vec![1, 2, 3]).await?;
let v = store.get(&"k1".to_string()).await?.unwrap();
store.delete(&"k1".to_string()).await?;
```

### Typed storage (serde JSON)

```rust
use nebula_storage::{MemoryStorageTyped, Storage};

let typed: MemoryStorageTyped<serde_json::Value> = MemoryStorageTyped::new();
typed.set(&"workflow:1".to_string(), &serde_json::json!({"name": "My Workflow"})).await?;
let val = typed.get(&"workflow:1".to_string()).await?.unwrap();
```

### Error handling

```rust
match store.get(&key).await {
    Ok(Some(v)) => { /* use v */ }
    Ok(None) => { /* key absent — normal for get */ }
    Err(StorageError::Serialization(e)) => { /* invalid JSON — non-retryable */ }
    Err(StorageError::Backend(msg)) => { /* backend failure — may retry */ }
    Err(StorageError::NotFound) => { /* used by ops requiring existence */ }
}
```

## Minimal Example

```rust
use nebula_storage::{MemoryStorageTyped, Storage};

#[derive(serde::Serialize, serde::Deserialize)]
struct WorkflowSummary { id: String, name: String }

#[tokio::main]
async fn main() -> Result<(), nebula_storage::StorageError> {
    let store: MemoryStorageTyped<WorkflowSummary> = MemoryStorageTyped::new();
    store.set(&"wf:1".into(), &WorkflowSummary { id: "1".into(), name: "My Flow".into() }).await?;
    let wf = store.get(&"wf:1".into()).await?.unwrap();
    if store.exists(&"wf:1".into()).await? {
        store.delete(&"wf:1".into()).await?;
    }
    Ok(())
}
```

## Compatibility Rules

- **Major version bump:** `Storage` trait method removal or signature change; `Key`/`Value` associated type contract changes.
- **Deprecation policy:** Minimum 2 minor releases.
