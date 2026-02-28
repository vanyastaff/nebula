# Archived From "docs/archive/final.md"

### nebula-storage
**Назначение:** Абстракция над различными системами хранения данных.

**Поддерживаемые backends:**
- PostgreSQL/MySQL - реляционные данные
- MongoDB - документы
- Redis - кеш и сессии
- S3/MinIO - бинарные данные
- Local filesystem - разработка

```rust
// Универсальный trait для хранилищ
#[async_trait]
pub trait Storage: Send + Sync {
    type Key;
    type Value;
    type Error;
    
    async fn get(&self, key: &Self::Key) -> Result<Option<Self::Value>, Self::Error>;
    async fn set(&self, key: &Self::Key, value: &Self::Value) -> Result<(), Self::Error>;
    async fn delete(&self, key: &Self::Key) -> Result<(), Self::Error>;
    async fn exists(&self, key: &Self::Key) -> Result<bool, Self::Error>;
}

// Специализированные хранилища
pub struct WorkflowStorage {
    backend: Box<dyn Storage<Key = WorkflowId, Value = WorkflowDefinition>>,
    cache: Arc<Cache>,
}

pub struct ExecutionStorage {
    backend: Box<dyn Storage<Key = ExecutionId, Value = ExecutionState>>,
    partitioner: ExecutionPartitioner,  // Для sharding по дате
}

pub struct BinaryStorage {
    backend: Box<dyn Storage<Key = String, Value = Vec<u8>>>,
    compression: CompressionStrategy,
}

// Транзакционность
pub struct TransactionalStorage {
    storage: Arc<dyn Storage>,
    tx_manager: TransactionManager,
}

impl TransactionalStorage {
    pub async fn transaction<F, T>(&self, f: F) -> Result<T>
    where F: FnOnce(&Transaction) -> Future<Output = Result<T>> {
        let tx = self.tx_manager.begin().await?;
        match f(&tx).await {
            Ok(result) => {
                tx.commit().await?;
                Ok(result)
            }
            Err(e) => {
                tx.rollback().await?;
                Err(e)
            }
        }
    }
}
```

---

