# nebula-storage

Абстракция над системами хранения данных (Infrastructure Layer).

## Trait Storage

Универсальный key-value trait:

- `get(key)` — получить значение
- `set(key, value)` — записать
- `delete(key)` — удалить
- `exists(key)` — проверить наличие

## Backends

| Backend        | Feature   | Статус   |
|----------------|-----------|----------|
| In-memory      | (встроен) | готово   |
| PostgreSQL     | `postgres`| опционально |
| Redis          | `redis`   | опционально |
| S3 / MinIO     | `s3`      | опционально |
| Local FS       | —         | планируется |

### PostgreSQL contract (`feature = "postgres"`)

- Таблица: `storage_kv(key TEXT PRIMARY KEY, value JSONB, updated_at TIMESTAMPTZ)`
- Тип значения: `serde_json::Value`
- Конструктор: `PostgresStorage::new(connection_string).await`

## Пример

```rust
use nebula_storage::{MemoryStorage, MemoryStorageTyped, Storage};

#[tokio::main]
async fn main() -> Result<(), nebula_storage::StorageError> {
    // Бинарные значения (Vec<u8>)
    let store = MemoryStorage::new();
    store.set(&"k1".to_string(), &vec![1, 2, 3]).await?;
    let v = store.get(&"k1".to_string()).await?.unwrap();

    // Типизированное хранилище (serde JSON)
    let typed: MemoryStorageTyped<serde_json::Value> = MemoryStorageTyped::new();
    typed.set(&"k2".to_string(), &serde_json::json!({"a": 1})).await?;
    let val = typed.get(&"k2".to_string()).await?.unwrap();

    Ok(())
}
```

## Зависимости по документации

- `nebula-core`
- `async-trait`, `sqlx` (postgres), `redis`, `aws-sdk-s3`
