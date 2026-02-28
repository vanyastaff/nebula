# Archived From "docs/archive/crates-architecture.md"

## 6. nebula-memory

**Purpose**: In-memory state management and caching.

```rust
// nebula-memory/src/lib.rs
pub mod cache;
pub mod state;
pub mod pool;

// nebula-memory/src/state.rs
pub struct InMemoryStateStore {
    states: Arc<RwLock<HashMap<ExecutionId, ExecutionState>>>,
    indexes: Arc<RwLock<StateIndexes>>,
}

impl InMemoryStateStore {
    pub async fn save_state(&self, state: ExecutionState) -> Result<(), Error> {
        let mut states = self.states.write().unwrap();
        let execution_id = state.execution_id.clone();
        
        states.insert(execution_id.clone(), state);
        self.update_indexes(&execution_id).await?;
        
        Ok(())
    }
    
    pub async fn get_state(&self, id: &ExecutionId) -> Option<ExecutionState> {
        self.states.read().unwrap().get(id).cloned()
    }
}

// nebula-memory/src/cache.rs
pub struct CacheManager {
    l1_cache: Arc<Mutex<LruCache<CacheKey, CachedValue>>>,
    stats: Arc<CacheStats>,
}

impl CacheManager {
    pub async fn get<T: DeserializeOwned>(&self, key: &CacheKey) -> Option<T> {
        let mut cache = self.l1_cache.lock().unwrap();
        
        if let Some(value) = cache.get(key) {
            self.stats.record_hit();
            serde_json::from_value(value.data.clone()).ok()
        } else {
            self.stats.record_miss();
            None
        }
    }
}

// nebula-memory/src/pool.rs
pub struct ResourcePool {
    pools: HashMap<TypeId, Box<dyn TypedPool>>,
}

impl ResourcePool {
    pub fn register_pool<T: 'static>(&mut self, pool: impl TypedPool<Resource = T> + 'static) {
        self.pools.insert(TypeId::of::<T>(), Box::new(pool));
    }
    
    pub async fn acquire<T: 'static>(&self) -> Result<PooledResource<T>, Error> {
        let pool = self.pools
            .get(&TypeId::of::<T>())
            .ok_or_else(|| Error::ResourceNotFound)?;
            
        pool.acquire().await
    }
}
```

