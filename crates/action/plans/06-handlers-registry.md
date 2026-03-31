# Handlers, Adapters, Registry, Versioning, and Plugin Integration

## ActionDescriptor and ActionFactory

Catalog отделён от instantiation.

### ActionDescriptor (catalog, static)

```rust
pub trait ActionDescriptor: Send + Sync + 'static {
    fn key(&self) -> &ActionKey;
    fn interface_version(&self) -> InterfaceVersion;
    fn metadata(&self) -> &ActionMetadata;
    fn action_kind(&self) -> ActionKind;
    fn components(&self) -> &ActionComponents;
    fn capability_manifest(&self) -> CapabilityManifest {
        CapabilityManifest::from_components(self.components())
    }
    fn trigger_event_schema(&self) -> Option<&serde_json::Value> { None }

    /// Parameter migration from older interface versions.
    /// Default: no migration (only current version supported).
    fn migrate_parameters(
        &self,
        _from_version: InterfaceVersion,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ActionError> {
        Ok(params)
    }
}

/// Semantic interface version for action contracts.
/// Bump when: parameters changed, ports changed, behavior changed.
/// NOT bumped for: bug fixes, performance improvements, internal refactoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct InterfaceVersion(pub u32);

impl InterfaceVersion {
    pub fn new(v: u32) -> Self { Self(v) }
}

impl std::fmt::Display for InterfaceVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "v{}", self.0)
    }
}
```

### ActionFactory (instantiation, runtime)

```rust
pub trait ActionFactory: Send + Sync + 'static {
    fn key(&self) -> &ActionKey;
    fn create(&self, cx: &ActionFactoryContext) -> Result<ActionInstance, ActionBuildError>;
}

pub struct ActionFactoryContext {
    pub plugin_config: Option<serde_json::Value>,
}

#[derive(Debug, thiserror::Error)]
pub enum ActionBuildError {
    #[error("missing dependency: {0}")]
    MissingDependency(String),
    #[error("configuration error: {0}")]
    Configuration(String),
    #[error("internal error: {0}")]
    Internal(String),
}
```

---

## ActionInstance (closed enum)

No AnyAction, no as_any(), no downcast. Factory returns ActionInstance.

```rust
pub enum ActionInstance {
    Stateless(Box<dyn StatelessHandler>),
    Stateful(Box<dyn StatefulHandler>),
    Trigger(Box<dyn TriggerHandler>),
    Resource(Box<dyn ResourceHandler>),
}

/// ActionKind mirrors ActionInstance — both are closed enums.
/// Adding a new variant is a breaking change requiring major version bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionKind {
    Stateless,
    Stateful,
    Trigger,
    Resource,
}
```

---

## Handler Traits (Type-Erased, Engine-Facing)

### StatelessHandler

```rust
#[async_trait]
pub trait StatelessHandler: Send + Sync {
    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: &ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError>;
}
```

### StatefulHandler

```rust
#[async_trait]
pub trait StatefulHandler: Send + Sync {
    async fn execute(
        &self,
        input: serde_json::Value,
        state: Option<serde_json::Value>,
        ctx: &ActionContext,
    ) -> Result<StatefulHandlerResult, ActionError>;

    fn state_version(&self) -> u32;
    fn migrate_state(
        &self,
        persisted: PersistedState,
        ctx: &ActionContext,
    ) -> Result<NextPersistedState, StateMigrationError>;
}

pub struct StatefulHandlerResult {
    pub flow_result: ActionResult<serde_json::Value>,
    pub next_state: NextPersistedState,
}
```

### TriggerHandler

```rust
#[async_trait]
pub trait TriggerHandler: Send + Sync {
    async fn start(
        &self,
        state: Option<serde_json::Value>,
        ctx: &TriggerContext,
    ) -> Result<TriggerStartResult, ActionError>;

    async fn run(
        &self,
        state: serde_json::Value,
        ctx: &TriggerContext,
    ) -> Result<TriggerCompletion, ActionError>;

    async fn stop(
        &self,
        state: Option<serde_json::Value>,
        ctx: &TriggerContext,
    ) -> Result<(), ActionError>;
    async fn health_check(&self, ctx: &TriggerContext) -> Result<TriggerHealth, ActionError>;
}

pub struct TriggerStartResult {
    pub start_mode: TriggerStartMode,
    pub next_state: serde_json::Value,
}
```

### ResourceHandler

```rust
#[async_trait]
pub trait ResourceHandler: Send + Sync {
    async fn acquire(&self, ctx: &ActionContext) -> Result<Box<dyn Any + Send + Sync>, ActionError>;
    async fn release(&self, lease: Box<dyn Any + Send + Sync>, ctx: &ActionContext) -> Result<ReleaseOutcome, ActionError>;
}
```

---

## Adapters (Typed → Type-Erased)

### StatelessAdapter

```rust
pub struct StatelessAdapter<A: StatelessAction> { action: A }

#[async_trait]
impl<A: StatelessAction> StatelessHandler for StatelessAdapter<A>
where A::Input: DeserializeOwned, A::Output: Serialize,
{
    async fn execute(&self, input: serde_json::Value, ctx: &ActionContext)
        -> Result<ActionResult<serde_json::Value>, ActionError>
    {
        let typed_input: A::Input = serde_json::from_value(input)
            .map_err(|e| ActionError::validation(format!("input deserialization: {e}")))?;
        let result = self.action.execute(typed_input, ctx).await?;
        // Maps only Value(T) → Value(Value). Binary/Streaming pass through.
        result.try_map_value(|output| {
            serde_json::to_value(&output)
                .map_err(|e| ActionError::fatal(format!("output serialization: {e}")))
        })
    }
}
```

### StatefulAdapter (with pre-execute snapshot)

```rust
pub struct StatefulAdapter<A: StatefulAction> { action: A }

#[async_trait]
impl<A: StatefulAction> StatefulHandler for StatefulAdapter<A>
where A::Input: DeserializeOwned, A::Output: Serialize, A::State: Serialize + DeserializeOwned,
{
    async fn execute(&self, input: serde_json::Value, state_json: Option<serde_json::Value>, ctx: &ActionContext)
        -> Result<StatefulHandlerResult, ActionError>
    {
        let typed_input: A::Input = serde_json::from_value(input)
            .map_err(|e| ActionError::validation(format!("input bind: {e}")))?;

        let mut typed_state: A::State = match state_json {
            Some(json) => serde_json::from_value(json)
                .map_err(|e| ActionError::fatal(format!("state corrupt: {e}")))?,
            None => self.action.init_state(&typed_input, ctx).await?,
        };

        // Runtime wraps this in tokio::spawn for panic isolation — see Panic Safety policy
        let result = self.action.execute(typed_input, &mut typed_state, ctx).await?;

        let next_state = NextPersistedState {
            state_version: self.action.state_version(),
            payload: serde_json::to_value(&typed_state)
                .map_err(|e| ActionError::fatal(format!("state serialize: {e}")))?,
        };

        let flow_result = result.try_map_value(|out| {
            serde_json::to_value(&out)
                .map_err(|e| ActionError::fatal(format!("output serialize: {e}")))
        })?;

        Ok(StatefulHandlerResult { flow_result, next_state })
    }
}
```

---

## ActionRegistry (version-aware)

Registry key = `(ActionKey, InterfaceVersion)`. Multiple versions of the same
action coexist — workflows upgrade at their own pace.

```rust
/// Composite key: action identity + interface version.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VersionedActionKey {
    pub key: ActionKey,
    pub version: InterfaceVersion,
}

pub struct ActionRegistry {
    actions: DashMap<VersionedActionKey, RegisteredAction>,
    /// Latest version per action key (for "use latest" in editor).
    latest: DashMap<ActionKey, InterfaceVersion>,
}

pub struct RegisteredAction {
    pub descriptor: Arc<dyn ActionDescriptor>,
    pub factory: Arc<dyn ActionFactory>,
}

impl ActionRegistry {
    pub fn new() -> Self {
        Self { actions: DashMap::new(), latest: DashMap::new() }
    }

    pub fn register(
        &self,
        descriptor: Arc<dyn ActionDescriptor>,
        factory: Arc<dyn ActionFactory>,
    ) -> Result<(), RegistrationError> {
        let vkey = VersionedActionKey {
            key: descriptor.key().clone(),
            version: descriptor.interface_version(),
        };
        if self.actions.contains_key(&vkey) {
            return Err(RegistrationError::VersionConflict(vkey));
        }
        let action_key = vkey.key.clone();
        let version = vkey.version;
        self.actions.insert(vkey, RegisteredAction { descriptor, factory });
        self.latest
            .entry(action_key)
            .and_modify(|v| { if version > *v { *v = version; } })
            .or_insert(version);
        Ok(())
    }

    /// All-or-nothing batch registration (plugin install).
    ///
    /// **Atomicity note:** Pre-validates all entries, then inserts in one pass.
    /// Uses DashMap bulk operations. In a concurrent scenario, a TOCTOU race
    /// is theoretically possible between check and insert — production impl
    /// should use a write lock or staging pattern for true atomicity.
    pub fn register_batch(
        &self,
        actions: Vec<(Arc<dyn ActionDescriptor>, Arc<dyn ActionFactory>)>,
    ) -> Result<(), RegistrationError> {
        // Phase 1: Build entries + pre-validate
        let entries: Vec<_> = actions.into_iter().map(|(desc, factory)| {
            let vkey = VersionedActionKey {
                key: desc.key().clone(),
                version: desc.interface_version(),
            };
            (vkey, RegisteredAction { descriptor: desc, factory })
        }).collect();

        for (vkey, _) in &entries {
            if self.actions.contains_key(vkey) {
                return Err(RegistrationError::VersionConflict(vkey.clone()));
            }
        }

        // Phase 2: Insert all (pre-validated, errors not expected)
        for (vkey, entry) in entries {
            let action_key = vkey.key.clone();
            let version = vkey.version;
            self.actions.insert(vkey, entry);
            self.latest
                .entry(action_key)
                .and_modify(|v| { if version > *v { *v = version; } })
                .or_insert(version);
        }
        Ok(())
    }

    /// Get specific version.
    pub fn get(&self, key: &ActionKey, version: InterfaceVersion)
        -> Option<Ref<VersionedActionKey, RegisteredAction>> { ... }

    /// Get latest version (for editor "use latest").
    pub fn get_latest(&self, key: &ActionKey)
        -> Option<Ref<VersionedActionKey, RegisteredAction>> { ... }

    /// List all versions of a specific action.
    pub fn versions(&self, key: &ActionKey) -> Vec<InterfaceVersion> { ... }

    /// Full catalog for UI (all actions, latest version each).
    pub fn catalog(&self) -> Vec<ActionCatalogEntry> { ... }
}

#[derive(Debug, thiserror::Error)]
pub enum RegistrationError {
    #[error("version conflict: {0:?} already registered")]
    VersionConflict(VersionedActionKey),
    #[error("build error: {0}")]
    BuildError(#[from] ActionBuildError),
}
```

---

## Action Versioning Strategy

### Three version levels

| Level | What | When to bump | Who migrates | Where stored |
|-------|------|-------------|-------------|-------------|
| **Plugin version** | Crate semver (1.2.3) | New actions, bug fixes, deps | Cargo / package manager | Cargo.toml |
| **Interface version** | Params, ports, behavior | Breaking param/port change | `migrate_parameters()` | ActionMetadata + workflow JSON |
| **State version** | StatefulAction::State | State struct changed | `migrate_state()` | PersistedState envelope |

### Interface version rules

**Bump** when:
- Parameter added/removed/renamed
- Parameter type changed (string → integer)
- Port added/removed
- Output schema changed
- Behavior semantics changed (same input → different output)

**Don't bump** when:
- Bug fixed (same contract, corrected behavior)
- Performance improved
- Internal refactoring
- New optional parameter with sensible default (non-breaking)

### Action author declares version

```rust
#[derive(Action)]
#[action(
    key = "telegram.send_message",
    version = 2,  // ← interface version
    name = "Send Telegram Message",
)]
struct SendTelegramMessageV2;
```

### Parameter migration between versions

```rust
/// Trait for migrating parameters between interface versions.
pub trait ActionMigration: Action {
    /// Which older versions this action can migrate from.
    fn supported_versions(&self) -> &[InterfaceVersion] { &[] }

    /// Migrate parameters from an older interface version to current.
    fn migrate_parameters(
        &self,
        from_version: InterfaceVersion,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ActionError> {
        Err(ActionError::fatal("migration not supported"))
    }
}

// Example:
impl ActionMigration for SendTelegramMessageV2 {
    fn supported_versions(&self) -> &[InterfaceVersion] {
        &[InterfaceVersion(1)]
    }

    fn migrate_parameters(
        &self,
        from_version: InterfaceVersion,
        mut params: serde_json::Value,
    ) -> Result<serde_json::Value, ActionError> {
        match from_version.0 {
            1 => {
                // v1: chat_id was string, v2: integer
                if let Some(chat_id) = params["chat_id"].as_str() {
                    params["chat_id"] = json!(chat_id.parse::<i64>().unwrap_or(0));
                }
                // v2 added parse_mode with default
                if params.get("parse_mode").is_none() {
                    params["parse_mode"] = json!("HTML");
                }
                Ok(params)
            }
            _ => Err(ActionError::fatal(format!(
                "cannot migrate from v{}", from_version.0
            ))),
        }
    }
}
```

### Plugin registers all live versions

```rust
impl Plugin for TelegramPlugin {
    fn actions(&self) -> Vec<Arc<dyn ActionDescriptor>> {
        vec![
            // Both versions live — old workflows keep working
            Arc::new(SendTelegramMessageV1Descriptor),
            Arc::new(SendTelegramMessageV2Descriptor),
            // Single-version actions
            Arc::new(EditMessageDescriptor),    // v1 only
            Arc::new(WebhookTriggerDescriptor), // v1 only
        ]
    }
}
```

### Workflow node references specific version

```json
{
    "node_id": "node_abc",
    "action_key": "telegram.send_message",
    "interface_version": 2,
    "parameters": {
        "chat_id": 42,
        "text": "Hello!",
        "parse_mode": "HTML"
    }
}
```

### Runtime version resolution

```rust
// Engine loads workflow node:
let action = registry.get(&node.action_key, node.interface_version);

match action {
    Some(action) => {
        // Exact version found — execute normally
        execute(action, node.parameters)
    }
    None => {
        // Version removed — try auto-migration to latest
        let latest = registry.get_latest(&node.action_key)?;
        let migrated = latest.descriptor
            .migrate_parameters(node.interface_version, node.parameters)?;
        execute(latest, migrated)
    }
}
```

### UI/Editor upgrade flow

```
User opens workflow with v1 node
  → Editor: latest for "telegram.send_message" = v2
  → Banner: "⬆ Upgrade available: v1 → v2"
  → User clicks "Upgrade"
  → Editor calls migrate_parameters(v1, old_params) → v2_params
  → Node updated to v2 + migrated params
  → User reviews, saves
```

### Deprecation

```rust
pub trait ActionDescriptor: Send + Sync + 'static {
    // ... existing methods ...

    /// If this version is deprecated, return replacement info.
    fn deprecated(&self) -> Option<DeprecationInfo> { None }
}

pub struct DeprecationInfo {
    pub upgrade_to: InterfaceVersion,
    pub message: String,
    pub removal_date: Option<chrono::DateTime<chrono::Utc>>,
}
```

### Version lifecycle

```
v1 registered → workflows use v1
  │
v2 released → v1 + v2 both in registry, new workflows default to v2
  │
v1 deprecated → UI warning, v1 still works
  │
v1 removed (major plugin bump) → registry refuses v1, must migrate first
```

### Plugin compatibility

```rust
/// Plugin trait — provides actions, data tags, and resources to Nebula.
pub trait Plugin: Send + Sync {
    /// Plugin display name.
    fn name(&self) -> &str;

    /// Data tags this plugin defines (registered before actions).
    /// Default: no custom tags.
    fn data_tags(&self) -> Vec<DataTagInfo> { vec![] }

    /// Action descriptors provided by this plugin.
    fn actions(&self) -> Vec<Arc<dyn ActionDescriptor>>;

    /// Factory for creating action instances.
    fn action_factory(&self, key: &ActionKey) -> Result<Arc<dyn ActionFactory>, ActionBuildError>;
}

// Runtime loads plugin:
// 1. Register tags first (actions may reference them)
for tag_info in plugin.data_tags() {
    tag_registry.register(tag_info)?;
}
// 2. Register actions (validated against known tags)
for desc in plugin.actions() {
    let factory = plugin.action_factory(desc.key())?;
    action_registry.register(desc, factory)?;
}
```
