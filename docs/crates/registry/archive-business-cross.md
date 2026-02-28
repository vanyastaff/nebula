# Archived From "docs/archive/business-cross.md"

### nebula-registry
**Назначение:** Централизованный реестр Actions, Nodes, Workflows и Resources.

**Ключевые компоненты:**
- ActionRegistry - каталог actions
- NodeRegistry - группировка actions
- WorkflowRegistry - deployed workflows
- Discovery API

```rust
pub struct Registry {
    actions: Arc<RwLock<HashMap<ActionId, ActionMetadata>>>,
    nodes: Arc<RwLock<HashMap<NodeId, NodeDefinition>>>,
    workflows: Arc<RwLock<HashMap<WorkflowId, WorkflowDefinition>>>,
    search_index: Arc<SearchIndex>,
}

impl Registry {
    // Регистрация Action
    pub async fn register_action<A: Action>(&self) -> Result<()> {
        let metadata = A::metadata();
        self.actions.write().await.insert(metadata.id.clone(), metadata);
        self.search_index.index_action(&metadata).await?;
        Ok(())
    }
    
    // Поиск Actions по критериям
    pub async fn search_actions(&self, query: &SearchQuery) -> Vec<ActionMetadata> {
        self.search_index.search(query).await
    }
    
    // Discovery для UI
    pub async fn get_actions_by_category(&self, category: &str) -> Vec<ActionMetadata> {
        self.actions.read().await
            .values()
            .filter(|a| a.category == category)
            .cloned()
            .collect()
    }
    
    // Версионирование
    pub async fn get_compatible_actions(&self, version: &Version) -> Vec<ActionMetadata> {
        self.actions.read().await
            .values()
            .filter(|a| a.version.is_compatible_with(version))
            .cloned()
            .collect()
    }
}

// Auto-registration через макрос
#[register_action]
pub struct MyAction;

// Или програмно
registry.register_action::<EmailSendAction>().await?;
registry.register_node(slack_node).await?;
```

---

## Cross-Cutting Concerns Layer

