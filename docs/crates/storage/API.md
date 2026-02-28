# API

## Public Surface

### Stable APIs

- `Storage` trait — `type Key`, `type Value`; `get`, `set`, `delete`, `exists`
- `MemoryStorage` — `new()`; implements `Storage<Key = String, Value = Vec<u8>>`
- `MemoryStorageTyped<T>` — `new()`; implements `Storage<Key = String, Value = T>` where T: Serialize + DeserializeOwned
- `StorageError` — NotFound, Serialization(serde_json::Error), Backend(String)

### Experimental / TODO

- PostgresStorage (feature `postgres`)
- RedisStorage (feature `redis`)
- S3Storage (feature `s3`)
- List/prefix scan

## Usage Patterns

### Binary storage (Vec<u8>)

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
    Ok(None) => { /* key not found */ }
    Err(StorageError::Serialization(e)) => { /* invalid JSON */ }
    Err(StorageError::Backend(msg)) => { /* backend error */ }
    Err(StorageError::NotFound) => { /* for ops that require key to exist */ }
}
```

## Minimal Example

```rust
use nebula_storage::{MemoryStorageTyped, Storage};

#[tokio::main]
async fn main() -> Result<(), nebula_storage::StorageError> {
    let store: MemoryStorageTyped<serde_json::Value> = MemoryStorageTyped::new();
    store.set(&"x".to_string(), &serde_json::json!({"a": 1})).await?;
    let v = store.get(&"x".to_string()).await?.unwrap();
    assert_eq!(v["a"], 1);
    Ok(())
}
```

## Advanced Example

```rust
// Custom type
#[derive(serde::Serialize, serde::Deserialize)]
struct WorkflowSummary { id: String, name: String }

let store: MemoryStorageTyped<WorkflowSummary> = MemoryStorageTyped::new();
store.set(&"wf:1".into(), &WorkflowSummary { id: "1".into(), name: "Test".into() }).await?;
let wf = store.get(&"wf:1".into()).await?.unwrap();

// Exists check
if store.exists(&"wf:1".into()).await? {
    store.delete(&"wf:1".into()).await?;
}
```

## Error Semantics

- **NotFound:** For operations that require key to exist (e.g. get with strict mode); get returns Option::None when absent.
- **Serialization:** serde_json error when serializing/deserializing in MemoryStorageTyped.
- **Backend:** Generic backend failure (connection, timeout, etc.).

## Compatibility Rules

- **Major version bump:** Storage trait method removal; Key/Value type changes.
- **Deprecation policy:** Minimum 2 minor releases.
