# Archived From "docs/archive/overview.md"

### nebula-memory
**Назначение:** Управление памятью и кешированием с учетом scopes.

**Ключевые компоненты:**
- Scoped arenas (Global/Workflow/Execution/Action)
- Expression result caching
- Automatic cleanup
- Tiered cache

```rust
pub struct MemoryManager {
    global_arena: Arc<GlobalArena>,
    execution_arenas: Arc<DashMap<ExecutionId, ExecutionArena>>,
    workflow_arenas: Arc<DashMap<WorkflowId, WorkflowArena>>,
    cache: Arc<TieredMemoryCache>,
}

// Многоуровневый кеш
pub struct TieredMemoryCache {
    l1_hot: LruCache<CacheKey, Arc<CacheEntry>>,     // В памяти
    l2_warm: RwLock<BTreeMap<CacheKey, CacheEntry>>, // Теплый кеш
    l3_external: Option<Box<dyn ExternalCache>>,     // Redis
    expression_cache: ExpressionResultCache,          // Для expressions
}

// Использование — данные передаются как serde_json::Value
let data = context.allocate_scoped_memory(
    large_dataset,  // serde_json::Value
    ResourceLifecycle::Execution  // Очистится в конце execution
).await?;
```

---

