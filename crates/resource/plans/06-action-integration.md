# 06 — Action Integration & Plugin System

---

## ctx.resource() — THE primary API for action authors

**One line. One call. Zero topology knowledge.**

Action authors never deal with pools, connections, recovery, or reconnection.
They call `ctx.resource::<R>().await?` and get back a handle that `Deref`s
to whatever the resource exposes. Drop cleans up automatically.

```rust
// This is ALL an action author needs to know:
let db = ctx.resource::<Postgres>().await?;
db.query("SELECT 1", &[]).await?;
// drop(db) → automatic cleanup (pool checkin, release, noop — depends on topology, but author doesn't care)
```

### ResourceContext trait

```rust
/// Extension trait on ActionContext / TriggerContext.
/// Bridge between nebula-action and nebula-resource.
pub trait ResourceContext {
    /// Acquire a managed resource. Topology dispatch is internal.
    /// Returns ResourceHandle<R> — Deref to R::Lease.
    /// Drop = automatic cleanup (checkin, release, unsubscribe, noop).
    fn resource<R: Resource>(
        &self,
    ) -> impl Future<Output = Result<ResourceHandle<R>, ActionError>> + Send;

    /// Typed credential from credential store.
    fn credential<C: CredentialType>(
        &self,
    ) -> impl Future<Output = Result<C, ActionError>> + Send;
}
```

### Resolution order

```
ctx.resource::<Postgres>()
  1. Scoped — ResourceAction in parent graph branch?
     → yes → acquire from scoped runtime
  2. Global — Manager registry by resource_id from node config?
     → yes → manager.acquire::<R>(resource_id, ctx, options)
  3. Not found → ActionError::ResourceNotFound
```

Action authors don't control or even see this resolution. The framework picks
the right source automatically. Scoped resources (from a `ResourceAction` parent
node) take priority so that per-execution isolation works transparently.

### Implementation for ActionContext

```rust
impl ResourceContext for ActionContext {
    async fn resource<R: Resource>(&self) -> Result<ResourceHandle<R>, ActionError> {
        // 1. Scoped (from ResourceAction parent in graph).
        if let Some(scoped) = self.scoped_resources.get::<R>() {
            return scoped.acquire(self.ctx())
                .await
                .map_err(ActionError::resource);
        }

        // 2. Global (from Manager registry).
        let resource_id = self.node_config
            .resource_id_for::<R>()
            .ok_or_else(|| ActionError::ResourceNotConfigured {
                resource_type: R::KEY.to_string(),
            })?;

        self.manager
            .acquire::<R>(resource_id, self.ctx(), AcquireOptions::standard())
            .await
            .map_err(ActionError::resource)
    }

    async fn credential<C: CredentialType>(&self) -> Result<C, ActionError> {
        self.credential_store
            .get_typed::<C>(self.credential_id_for::<C>()?)
            .await
            .map_err(ActionError::credential)
    }
}
```

### Implementation for TriggerContext

```rust
impl ResourceContext for TriggerContext {
    async fn resource<R: Resource>(&self) -> Result<ResourceHandle<R>, ActionError> {
        // Triggers: ONLY global. No scoped resources (no parent graph).
        let resource_id = self.trigger_config
            .resource_id_for::<R>()
            .ok_or_else(|| ActionError::ResourceNotConfigured {
                resource_type: R::KEY.to_string(),
            })?;

        self.manager
            .acquire::<R>(resource_id, self.ctx(), AcquireOptions::standard())
            .await
            .map_err(ActionError::resource)
    }

    async fn credential<C: CredentialType>(&self) -> Result<C, ActionError> {
        self.credential_store
            .get_typed::<C>(self.credential_id_for::<C>()?)
            .await
            .map_err(ActionError::credential)
    }
}
```

### Action examples — one-line resource access

Every action type uses the same `ctx.resource::<R>()` pattern.
The topology (Pool, Resident, Service, etc.) is invisible to the action author.

```rust
// StatelessAction — Postgres (Pooled topology behind the scenes):
async fn execute(&self, input: Self::Input, ctx: &ActionContext) -> Result<ActionResult<Self::Output>> {
    let db = ctx.resource::<Postgres>().await?;
    // db: ResourceHandle<Postgres>
    // Deref → R::Lease (= PgConnection) → query, execute, transaction
    // prepare() already called by framework (SET search_path)
    let rows = db.query("SELECT * FROM users WHERE id = $1", &[&input.user_id]).await?;
    // drop(db) → pool checkin → recycle
}

// StatelessAction — Telegram (Service topology behind the scenes):
async fn execute(&self, input: Self::Input, ctx: &ActionContext) -> Result<ActionResult<Self::Output>> {
    let bot = ctx.resource::<TelegramBot>().await?;
    // bot: ResourceHandle<TelegramBot>
    // Deref → R::Lease (= TelegramBotHandle) → send_message, send_html, etc.
    bot.send_message(input.chat_id, &input.text).await?;
    // drop(bot) → noop (Service token release)
}

// StatefulAction — batch import:
async fn execute(&self, input: Self::Input, state: &mut ImportState, ctx: &ActionContext) -> Result<...> {
    let db = ctx.resource::<Postgres>().await?;
    // Each execute() → fresh acquire. State holds cursor.
    for row in &input.rows[state.cursor..state.cursor + 1000] {
        db.execute("INSERT INTO ...", &[&row]).await?;
        state.cursor += 1;
    }
    // drop(db). State persisted. Next execute() → new acquire.
}

// TransactionalAction — saga compensation:
async fn compensate(&self, data: Self::CompensationData, ctx: &ActionContext) -> Result<()> {
    // Engine guarantees: ctx has same scope/tenant as execute_tx().
    let payment = ctx.resource::<PaymentApi>().await?;
    payment.refund(&data.payment_id).await?;
}
```

---

## ActionDependencies — declarative resource declaration

Actions declare their resource requirements at registration time via the
`ActionDependencies` trait from `nebula-action`. This is separate from runtime
acquisition (`ctx.resource::<R>()`) — it tells the engine what resources an
action *needs* so the engine can validate configuration before execution starts.

```rust
/// From nebula-action — declarative dependency declaration.
/// Engine calls these at registration time (not at execution time).
pub trait ActionDependencies {
    /// The credential required by this action, if any.
    fn credential() -> Option<Box<dyn AnyCredential>>
    where Self: Sized { None }

    /// Resources required by this action.
    /// Returns Vec<Box<dyn AnyResource>> — type-erased resource markers.
    fn resources() -> Vec<Box<dyn AnyResource>>
    where Self: Sized { vec![] }
}
```

**Example — declaring a Postgres dependency:**

```rust
impl ActionDependencies for QueryUsersAction {
    fn resources() -> Vec<Box<dyn AnyResource>> {
        vec![Box::new(Postgres)]
    }
}

// At execution time, the action just calls:
// let db = ctx.resource::<Postgres>().await?;
```

The engine uses `resources()` to:
- Validate that required resources are registered in the Manager before workflow starts.
- Build the dependency tree for UI display (see Plugin System below).
- Provide clear error messages at startup rather than runtime failures.

---

## EventTrigger — zero-boilerplate event-driven triggers

DX wrapper over TriggerAction. Sits alongside WebhookAction and PollAction.

```
TriggerAction (core — low level)
├── WebhookAction    (DX — HTTP endpoint + signature verification)
├── PollAction       (DX — cursor + interval)
└── EventTrigger     (DX — event stream from resource)
```

**What the trigger author writes:** just `on_event()`.
**What the engine handles:** resource acquisition, reconnection with exponential backoff,
error routing, cancellation, event emission. The author never sees retry loops.

```rust
/// Event stream from a managed resource.
/// Author writes ONLY on_event(). Reconnection, backoff, emit — engine handles it.
pub trait EventTrigger: Action {
    /// Resource type — the event source.
    type Source: Resource;

    /// Event type emitted to create workflow executions.
    type Event: Serialize + DeserializeOwned;

    /// Transform raw data from resource into a trigger event.
    ///
    /// Called in a loop. Return None to skip/filter.
    /// Engine lifecycle: acquire resource → loop { on_event → emit } → on error → reconnect.
    /// `source` = &R::Lease — already acquired and health-checked by engine.
    async fn on_event(
        &self,
        source: &<Self::Source as Resource>::Lease,
        ctx: &TriggerContext,
    ) -> Result<Option<Self::Event>>;

    /// Called on resource or on_event error. Default: reconnect with backoff.
    /// Override to customize error handling (e.g., stop on auth errors).
    async fn on_error(
        &self,
        error: crate::Error,
        ctx: &TriggerContext,
    ) -> ErrorAction {
        ErrorAction::Reconnect
    }
}

pub enum ErrorAction {
    /// Re-acquire resource with exponential backoff (1s → 2s → ... → 60s cap).
    Reconnect,
    /// Stop the trigger permanently.
    Stop,
    /// Ignore this error, continue the listen loop.
    Ignore,
}
```

### What the engine generates behind the scenes

The trigger author never writes this. The engine generates a `TriggerAction`
implementation that handles the full lifecycle:

```rust
/// Engine-generated TriggerAction impl for any EventTrigger.
async fn run_event_trigger<T: EventTrigger>(trigger: &T, ctx: &TriggerContext) {
    let mut backoff = Duration::from_secs(1);

    loop {
        // (Re-)acquire resource — engine handles this, not the trigger author.
        let handle = match ctx.resource::<T::Source>().await {
            Ok(h) => { backoff = Duration::from_secs(1); h }
            Err(e) => {
                match trigger.on_error(e.into(), ctx).await {
                    ErrorAction::Reconnect => {
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(Duration::from_secs(60));
                        continue;
                    }
                    ErrorAction::Stop => return,
                    ErrorAction::Ignore => continue,
                }
            }
        };

        // Listen loop — engine drives this.
        loop {
            tokio::select! {
                _ = ctx.cancelled() => return,
                result = trigger.on_event(&handle, ctx) => {
                    match result {
                        Ok(Some(event)) => { ctx.emit(event).await.ok(); }
                        Ok(None)        => continue,
                        Err(e) => {
                            match trigger.on_error(e.into(), ctx).await {
                                ErrorAction::Reconnect => break, // → outer loop re-acquires
                                ErrorAction::Stop      => return,
                                ErrorAction::Ignore    => continue,
                            }
                        }
                    }
                }
            }
        }
    }
}
```

### EventTrigger examples

Trigger authors write only the event transformation. No reconnection logic,
no backoff, no resource lifecycle management.

```rust
// Telegram incoming messages — author writes ~10 lines:
struct IncomingMessageTrigger;

impl EventTrigger for IncomingMessageTrigger {
    type Source = TelegramBot;
    type Event  = IncomingMessage;

    async fn on_event(&self, bot: &TelegramBotHandle, _ctx: &TriggerContext)
        -> Result<Option<IncomingMessage>>
    {
        let update = bot.recv_update().await?;
        match update.kind {
            UpdateKind::Message { text: Some(text), message_id } => {
                Ok(Some(IncomingMessage { chat_id: update.chat_id.unwrap(), text, message_id }))
            }
            _ => Ok(None), // skip non-text updates
        }
    }
    // on_error: default Reconnect — handles Telegram API disconnects automatically
}

// Redis Pub/Sub — minimal implementation:
struct OrderEventTrigger;

impl EventTrigger for OrderEventTrigger {
    type Source = RedisSubscriber;
    type Event  = OrderEvent;

    async fn on_event(&self, sub: &RedisPubSubLease, _ctx: &TriggerContext)
        -> Result<Option<OrderEvent>>
    {
        let msg = sub.recv().await?;
        if msg.channel.starts_with("orders.") {
            Ok(Some(serde_json::from_str(&msg.payload)?))
        } else {
            Ok(None)
        }
    }
}
```

---

## ResourceAction — per-execution scoped resources

Use `ResourceAction` when a resource should be created for one execution
(or one branch of a graph) and destroyed when that branch completes.
Downstream actions use the same `ctx.resource::<R>()` — they don't know
whether the resource is scoped or global.

**When to use ResourceAction vs Global registration:**
- **Global** (Manager registration): shared across workflows, long-lived (e.g., shared Postgres pool).
- **ResourceAction** (scoped): per-execution or per-branch lifecycle (e.g., temporary test database,
  isolated transaction pool, per-tenant connection with dynamic credentials).

```rust
pub trait ResourceAction: Action {
    /// The resource type this action manages.
    type Resource: Resource;

    /// Provide config for the scoped resource. Called by engine BEFORE downstream nodes run.
    async fn configure(
        &self,
        ctx: &ActionContext,
    ) -> Result<<Self::Resource as Resource>::Config>;

    /// Topology for the scoped resource. Default: Resident (single shared instance).
    fn topology(&self) -> ScopedTopology {
        ScopedTopology::Resident
    }

    /// Custom cleanup after all downstream nodes complete.
    /// Framework calls Resource::shutdown() + Resource::destroy() automatically —
    /// this is only for extra per-execution cleanup (e.g., logging, audit).
    /// Default: noop.
    async fn cleanup(
        &self,
        ctx: &ActionContext,
    ) -> Result<()> {
        Ok(())
    }
}

pub enum ScopedTopology {
    /// Single shared instance for all downstream. Clone on acquire.
    Resident,
    /// Pool of instances for downstream. LeaseGuard on acquire.
    Pool(pool::Config),
    /// Single exclusive owner at a time.
    Exclusive,
}
```

### Graph example

```
Graph:
  [PostgresPool ResourceAction]  ← configure() creates pool config
       │
       ├── [QueryUsers StatelessAction]     ← ctx.resource::<Postgres>() → checkout from scoped pool
       │
       └── [QueryOrders StatelessAction]    ← ctx.resource::<Postgres>() → checkout from scoped pool

  After both complete: cleanup() → framework destroys pool
```

Downstream actions (`QueryUsers`, `QueryOrders`) call `ctx.resource::<Postgres>()`
exactly as they would with a global resource — the scoped vs global distinction
is completely transparent.

---

## Plugin System

Plugin = self-contained package. Installed as a unit. After installation,
components live in flat registries indexed by key.

```rust
pub trait Plugin: Send + Sync + 'static {
    fn key(&self) -> &str;
    fn manifest(&self) -> PluginManifest;
    fn resources(&self) -> Vec<Box<dyn ResourceDescriptor>>;
    fn credentials(&self) -> Vec<Box<dyn CredentialDescriptor>>;
    fn actions(&self) -> Vec<Box<dyn ActionDescriptor>>;
}
```

### Descriptors — metadata for UI and runtime

Descriptors provide everything the UI and engine need: JSON schemas for
configuration forms, credential requirements for dropdown filtering,
inter-resource dependencies, and self-registration from database configs.

```rust
pub trait ResourceDescriptor: Send + Sync {
    fn key(&self) -> &str;
    fn manifest(&self) -> ResourceManifest;
    /// ParamDef schema for UI config form generation.
    fn config_schema(&self) -> ParamDef;
    /// Which credential types are accepted. UI filters dropdown by these.
    fn credential_requirements(&self) -> Vec<CredentialRequirement>;
    /// Dependencies on other resources (e.g., a cache resource that needs Redis).
    fn required_resources(&self) -> Vec<ResourceRef>;
    /// Self-register from JSON config (loaded from DB). Generic — works for any resource type.
    fn register(&self, manager: &Manager, id: ResourceId, scope: Scope, config: Value, platform: &PlatformContext)
        -> BoxFuture<'_, Result<()>>;
    fn validate_config(&self, config: &Value) -> Result<()>;
    fn unregister(&self, manager: &Manager, id: ResourceId) -> BoxFuture<'_, Result<()>>;
}

pub trait CredentialDescriptor: Send + Sync {
    fn key(&self) -> &str;
    fn manifest(&self) -> CredentialManifest;
    /// ParamDef schema for UI credential form.
    fn schema(&self) -> ParamDef;
    fn create(&self, input: Value, store: &dyn CredentialStore) -> BoxFuture<'_, Result<CredentialId>>;
    fn validate(&self, id: CredentialId, store: &dyn CredentialStore) -> BoxFuture<'_, Result<()>>;
}

pub trait ActionDescriptor: Send + Sync {
    fn key(&self) -> &str;
    fn manifest(&self) -> ActionManifest;
    /// ParamDef schema for UI input form.
    fn input_schema(&self) -> ParamDef;
    /// ParamDef schema for output display.
    fn output_schema(&self) -> ParamDef;
    /// ParamDef schema for trigger events (None for non-triggers).
    fn event_schema(&self) -> Option<ParamDef>;
    fn action_type(&self) -> ActionType;
    /// Resource dependencies — matches ActionDependencies::resources() for validation.
    fn required_resources(&self) -> Vec<ResourceRef>;
    fn create_action(&self) -> Box<dyn AnyAction>;
}
```

### PluginRegistry — flat, cross-plugin references by key

```rust
pub struct PluginRegistry {
    plugins:     HashMap<String, Arc<dyn Plugin>>,
    resources:   HashMap<String, Arc<dyn ResourceDescriptor>>,
    credentials: HashMap<String, Arc<dyn CredentialDescriptor>>,
    actions:     HashMap<String, Arc<dyn ActionDescriptor>>,
}

impl PluginRegistry {
    /// Install plugin. Components go to flat registries by key.
    pub fn install(&mut self, plugin: impl Plugin + 'static) {
        let plugin = Arc::new(plugin);
        for cred in plugin.credentials()    { self.credentials.entry(cred.key().into()).or_insert(cred.into()); }
        for res in plugin.resources()       { self.resources.entry(res.key().into()).or_insert(res.into()); }
        for action in plugin.actions()      { self.actions.insert(action.key().into(), action.into()); }
        self.plugins.insert(plugin.key().into(), plugin);
    }

    /// Generic loader. Works for any plugin — no type-specific code.
    pub async fn load_resources_from_db(
        &self,
        manager:  &Manager,
        db:       &Database,
        platform: &PlatformContext,
    ) -> Result<()> {
        let rows = db.query("SELECT id, type_key, scope, config FROM resources").await?;
        for row in rows {
            let type_key: String = row.get("type_key");
            let desc = self.resources.get(&type_key)
                .ok_or_else(|| Error::not_found(&type_key))?;
            desc.register(manager, row.get("id"), row.get("scope"), row.get("config"), platform).await?;
        }
        Ok(())
    }

    /// Dependency tree for UI: "this action needs these resources and credentials".
    /// Used by workflow editor to show required setup before an action can be used.
    pub fn action_dependency_tree(&self, action_key: &str) -> DependencyTree { ... }

    /// Catalogs for UI — list all available components.
    pub fn plugin_catalog(&self) -> Vec<PluginManifest> { ... }
    pub fn resource_catalog(&self) -> Vec<ResourceManifest> { ... }
    pub fn action_catalog(&self) -> Vec<ActionManifest> { ... }
}
```

### Credential in resource config UI

```rust
pub struct CredentialRequirement {
    /// Human-readable slot label: "Authentication", "Database Credentials".
    pub label: String,
    /// Accepted credential types. UI filters dropdown and "Add new" options by these.
    pub accepted_types: Vec<String>,
    pub required: bool,
}
```

UI flow: dropdown "Select existing" (filtered by `accepted_types`) or "+ Add new" → pick type → fill form → return.

---

## DX summary — what each audience needs to know

| Audience | Needs to read | API surface |
|----------|--------------|-------------|
| **Action author** | This section only | `ctx.resource::<R>().await?` — one line |
| **Trigger author** | EventTrigger section | `on_event()` — one method, engine handles the rest |
| **Resource author** | 01-core + 02-topology + 09-guide | `Resource` trait + topology trait |
| **Plugin author** | Plugin System section | Descriptors for UI integration |

Action authors never need to know about topologies, pools, recovery gates,
circuit breakers, or connection lifecycle. They call `ctx.resource::<R>()`
and get back a handle. That's it.
