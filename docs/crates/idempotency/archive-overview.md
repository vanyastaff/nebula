# Archived From "docs/archive/overview.md"

### nebula-idempotency
**Назначение:** Обеспечение идемпотентности операций для надежности.

**Ключевые компоненты:**
- Idempotency keys
- Result caching
- Deduplication
- Retry detection

```rust
pub struct IdempotencyManager {
    store: Arc<dyn IdempotencyStore>,
    ttl: Duration,
}

impl IdempotencyManager {
    pub async fn execute_once<F, T>(&self, key: &str, f: F) -> Result<T>
    where F: FnOnce() -> Future<Output = Result<T>> {
        // Проверяем, выполнялось ли уже
        if let Some(result) = self.store.get(key).await? {
            return Ok(result);
        }
        
        // Выполняем и сохраняем результат
        let result = f().await?;
        self.store.set(key, &result, self.ttl).await?;
        Ok(result)
    }
}

// Использование в Action
let result = context.idempotency_manager
    .execute_once(&request_id, || async {
        // Операция выполнится только один раз
        database.insert_user(user_data).await
    })
    .await?;
```

