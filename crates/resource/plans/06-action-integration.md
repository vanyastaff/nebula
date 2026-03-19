# 06 — Action Integration & Plugin System

---

## ctx.resource() — unified API

Action author видит один метод. Topology скрыт.

```rust
/// Extension trait на ActionContext / TriggerContext.
/// Мост между nebula-action и nebula-resource.
pub trait ResourceContext {
    /// Получить managed resource. Topology dispatch внутри.
    /// Возвращает ResourceHandle<R> — Deref к R::Lease.
    /// Drop = automatic cleanup (checkin, release, unsubscribe, noop).
    fn resource<R: Resource>(
        &self,
    ) -> impl Future<Output = Result<ResourceHandle<R>, ActionError>> + Send;

    /// Typed credential из credential store.
    fn credential<C: CredentialType>(
        &self,
    ) -> impl Future<Output = Result<C, ActionError>> + Send;
}
```

### Resolution order

```
ctx.resource::<Postgres>()
  1. Scoped — ResourceAction в parent ветке графа?
     → если есть → acquire from scoped runtime
  2. Global — Manager registry по resource_id из node config?
     → если есть → manager.acquire::<R>(resource_id, ctx, options)
  3. Not found → ActionError::ResourceNotFound
```

### Implementation для ActionContext

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

### Implementation для TriggerContext

```rust
impl ResourceContext for TriggerContext {
    async fn resource<R: Resource>(&self) -> Result<ResourceHandle<R>, ActionError> {
        // Trigger: ТОЛЬКО global. Нет scoped resources (нет parent graph).
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

### Примеры в Actions

```rust
// StatelessAction — Postgres pool:
async fn execute(&self, input: Self::Input, ctx: &ActionContext) -> Result<ActionResult<Self::Output>> {
    let db = ctx.resource::<Postgres>().await?;
    // db: ResourceHandle<Postgres>
    // Deref → R::Lease (= PgConnection) → query, execute, transaction
    // prepare() уже вызван фреймворком (SET search_path)
    let rows = db.query("SELECT * FROM users WHERE id = $1", &[&input.user_id]).await?;
    // drop(db) → pool checkin → recycle
}

// StatelessAction — Telegram send:
async fn execute(&self, input: Self::Input, ctx: &ActionContext) -> Result<ActionResult<Self::Output>> {
    let bot = ctx.resource::<TelegramBot>().await?;
    // bot: ResourceHandle<TelegramBot>
    // Deref → R::Lease (= TelegramBotHandle) → send_message, send_html, etc.
    bot.send_message(input.chat_id, &input.text).await?;
    // drop(bot) → noop (HandleInner::Owned)
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

## EventTrigger — DX для event-driven triggers

DX type над TriggerAction. Рядом с WebhookAction и PollAction.

```
TriggerAction (core — low level)
├── WebhookAction    (DX — HTTP endpoint + signature)
├── PollAction       (DX — cursor + interval)
└── EventTrigger     (DX — event stream от resource)
```

```rust
/// Event stream от managed resource.
/// Разработчик пишет ТОЛЬКО on_event(). Reconnection, backoff, emit — движок.
pub trait EventTrigger: Action {
    /// Resource type — источник событий.
    type Source: Resource;

    /// Event type для создания workflow execution.
    type Event: Serialize + DeserializeOwned;

    /// Трансформировать raw data из resource в trigger event.
    ///
    /// Вызывается в цикле. None = skip (фильтр).
    /// Движок: acquire resource → loop { on_event → emit } → reconnect.
    /// `source` = ResourceHandle<Self::Source> — Deref to Self::Source::Lease.
    async fn on_event(
        &self,
        source: &<Self::Source as Resource>::Lease,
        ctx: &TriggerContext,
    ) -> Result<Option<Self::Event>>;

    /// При ошибке resource или on_event. Default: reconnect.
    async fn on_error(
        &self,
        error: crate::Error,
        ctx: &TriggerContext,
    ) -> ErrorAction {
        ErrorAction::Reconnect
    }
}

pub enum ErrorAction {
    /// Переподключиться. Re-acquire resource, continue loop.
    Reconnect,
    /// Остановить trigger.
    Stop,
    /// Игнорировать ошибку, continue loop.
    Ignore,
}
```

### Что движок генерирует за кулисами

```rust
/// Engine-generated TriggerAction impl для EventTrigger.
async fn run_event_trigger<T: EventTrigger>(trigger: &T, ctx: &TriggerContext) {
    let mut backoff = Duration::from_secs(1);

    loop {
        // (Re-)acquire resource.
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

        // Listen loop.
        loop {
            tokio::select! {
                _ = ctx.cancelled() => return,
                result = trigger.on_event(&handle, ctx) => {
                    match result {
                        Ok(Some(event)) => { ctx.emit(event).await.ok(); }
                        Ok(None)        => continue,
                        Err(e) => {
                            match trigger.on_error(e.into(), ctx).await {
                                ErrorAction::Reconnect => break, // → outer loop
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

### Примеры EventTrigger

```rust
// Telegram incoming messages:
struct IncomingMessageTrigger;

impl EventTrigger for IncomingMessageTrigger {
    type Source = TelegramBot;
    type Event  = IncomingMessage;

    async fn on_event(&self, bot: &TelegramBotHandle, _ctx: &TriggerContext)  // R::Lease = TelegramBotHandle
        -> Result<Option<IncomingMessage>>
    {
        let update = bot.recv_update().await?;
        match update.kind {
            UpdateKind::Message { text: Some(text), message_id } => {
                Ok(Some(IncomingMessage { chat_id: update.chat_id.unwrap(), text, message_id }))
            }
            _ => Ok(None), // skip non-text
        }
    }
}

// Redis Pub/Sub:
struct OrderEventTrigger;

impl EventTrigger for OrderEventTrigger {
    type Source = RedisSubscriber;
    type Event  = OrderEvent;

    async fn on_event(&self, sub: &RedisPubSubLease, _ctx: &TriggerContext)  // R::Lease
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

## ResourceAction — scoped resource в графе

```rust
pub trait ResourceAction: Action {
    /// Resource type.
    type Resource: Resource;

    /// Определяет конфиг. Вызывается движком ДО downstream nodes.
    async fn configure(
        &self,
        ctx: &ActionContext,
    ) -> Result<<Self::Resource as Resource>::Config>;

    /// Topology. Default Resident.
    fn topology(&self) -> ScopedTopology {
        ScopedTopology::Resident
    }

    /// Cleanup. Вызывается ПОСЛЕ всех downstream.
    /// Framework calls Resource::shutdown() + Resource::destroy() — resource author handles teardown.
    /// Default: noop. Override for custom per-execution cleanup.
    async fn cleanup(
        &self,
        ctx: &ActionContext,
    ) -> Result<()> {
        Ok(())
    }
}

pub enum ScopedTopology {
    /// Один shared instance для всех downstream. Clone при acquire.
    Resident,
    /// Pool instances для downstream. LeaseGuard при acquire.
    Pool(pool::Config),
    /// Один exclusive owner за раз.
    Exclusive,
}
```

```
Graph:
  [PostgresPool ResourceAction]  ← configure() → create pool
       │
       ├── [QueryUsers StatelessAction]     ← ctx.resource::<Postgres>() → checkout from scoped pool
       │
       └── [QueryOrders StatelessAction]    ← ctx.resource::<Postgres>() → checkout from scoped pool
       
  After both done: cleanup() → destroy pool
```

**Когда ResourceAction vs Global:**
- Shared across workflows → **Global** (Manager registration).
- Per-execution or per-branch → **ResourceAction** (scoped).

---

## Plugin System

Plugin = self-contained пакет. Устанавливается целиком. После install — components в плоских реестрах.

```rust
pub trait Plugin: Send + Sync + 'static {
    fn key(&self) -> &str;
    fn manifest(&self) -> PluginManifest;
    fn resources(&self) -> Vec<Box<dyn ResourceDescriptor>>;
    fn credentials(&self) -> Vec<Box<dyn CredentialDescriptor>>;
    fn actions(&self) -> Vec<Box<dyn ActionDescriptor>>;
}
```

### Descriptors

```rust
pub trait ResourceDescriptor: Send + Sync {
    fn key(&self) -> &str;
    fn manifest(&self) -> ResourceManifest;
    /// ParamDef schema для UI формы.
    fn config_schema(&self) -> ParamDef;
    /// Какие credential types подходят.
    fn credential_requirements(&self) -> Vec<CredentialRequirement>;
    /// Зависимости от других resources.
    fn required_resources(&self) -> Vec<ResourceRef>;
    /// Зарегистрировать из JSON config (из БД). Self-registration.
    fn register(&self, manager: &Manager, id: ResourceId, scope: Scope, config: Value, platform: &PlatformContext)
        -> BoxFuture<'_, Result<()>>;
    fn validate_config(&self, config: &Value) -> Result<()>;
    fn unregister(&self, manager: &Manager, id: ResourceId) -> BoxFuture<'_, Result<()>>;
}

pub trait CredentialDescriptor: Send + Sync {
    fn key(&self) -> &str;
    fn manifest(&self) -> CredentialManifest;
    fn schema(&self) -> ParamDef;
    fn create(&self, input: Value, store: &dyn CredentialStore) -> BoxFuture<'_, Result<CredentialId>>;
    fn validate(&self, id: CredentialId, store: &dyn CredentialStore) -> BoxFuture<'_, Result<()>>;
}

pub trait ActionDescriptor: Send + Sync {
    fn key(&self) -> &str;
    fn manifest(&self) -> ActionManifest;
    fn input_schema(&self) -> ParamDef;
    fn output_schema(&self) -> ParamDef;
    fn event_schema(&self) -> Option<ParamDef>;
    fn action_type(&self) -> ActionType;
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
    /// Install plugin. Components go to flat registries.
    pub fn install(&mut self, plugin: impl Plugin + 'static) {
        let plugin = Arc::new(plugin);
        for cred in plugin.credentials()    { self.credentials.entry(cred.key().into()).or_insert(cred.into()); }
        for res in plugin.resources()       { self.resources.entry(res.key().into()).or_insert(res.into()); }
        for action in plugin.actions()      { self.actions.insert(action.key().into(), action.into()); }
        self.plugins.insert(plugin.key().into(), plugin);
    }

    /// Generic loader. Zero match. Works for any plugin.
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
    pub fn action_dependency_tree(&self, action_key: &str) -> DependencyTree { ... }

    /// Catalogs for UI.
    pub fn plugin_catalog(&self) -> Vec<PluginManifest> { ... }
    pub fn resource_catalog(&self) -> Vec<ResourceManifest> { ... }
    pub fn action_catalog(&self) -> Vec<ActionManifest> { ... }
}
```

### Credential в resource config UI

```rust
pub struct CredentialRequirement {
    /// Человекочитаемое название слота: "Authentication".
    pub label: String,
    /// Подходящие типы. UI фильтрует dropdown и "Add new" по этим.
    pub accepted_types: Vec<String>,
    pub required: bool,
}
```

UI flow: dropdown "Select existing" (filtered by type) или "+ Add new" → выбрать тип → заполнить → вернуться.
