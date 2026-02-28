# Archived From "docs/archive/crates-architecture.md"

## 5. nebula-registry

**Purpose**: Node registry and plugin management.

```rust
// nebula-registry/src/lib.rs
pub mod action;
pub mod plugin;
pub mod discovery;

// nebula-registry/src/action.rs
pub struct ActionRegistry {
    actions: RwLock<HashMap<String, Arc<dyn Action>>>,
    metadata: RwLock<HashMap<String, ActionMetadata>>,
}

impl ActionRegistry {
    pub fn register<A: Action + 'static>(&self, action: A) -> Result<(), Error> {
        let metadata = action.metadata();
        let id = metadata.id.clone();
        
        self.actions.write().unwrap().insert(id.clone(), Arc::new(action));
        self.metadata.write().unwrap().insert(id, metadata);
        
        Ok(())
    }
    
    pub fn get_action(&self, id: &str) -> Option<Arc<dyn Action>> {
        self.actions.read().unwrap().get(id).cloned()
    }
    
    pub fn list_actions(&self) -> Vec<ActionMetadata> {
        self.metadata.read().unwrap().values().cloned().collect()
    }
}

// nebula-registry/src/plugin.rs
pub struct PluginManager {
    plugins: Vec<LoadedPlugin>,
    sandbox: Arc<PluginSandbox>,
}

pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    pub handle: PluginHandle,
    pub actions: Vec<String>,
}

impl PluginManager {
    pub async fn load_plugin(&mut self, path: &Path) -> Result<(), Error> {
        // Load and validate plugin
        let manifest = self.read_manifest(path)?;
        let handle = self.sandbox.load(path).await?;
        
        // Register plugin actions
        self.register_plugin_actions(&handle).await?;
        
        Ok(())
    }
}
```

