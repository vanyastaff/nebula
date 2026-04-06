# nebula-action v2 — Design Spec

## Goal

Implement the frozen action architecture (plans 01–08 + addendum) with integration for parameter v4, credential v3, and resource v2. Preserve all architectural decisions — this is an implementation spec, not a redesign.

## Philosophy

- **Protocol, not runtime.** Action crate defines contracts. Engine/runtime interpret them.
- **Preserve the frozen design.** 5 core types, port system, DataTag registry, state versioning, ExecutionGuard — all stay.
- **Integrate the updated stack.** Parameter v4 derive, credential v3 typed access, resource v2 typed access — wire them in.
- **DX via derive, power via manual impl.** Derive generates boilerplate for 80% case. Full trait impl for advanced use.

## Source Plans

All architectural decisions come from `crates/action/plans/`:
- 01-overview.md — 10 guarantees, protocol contract
- 02-core-types.md — 4+1 core types, state machine, durable commit
- 03-triggers.md — trigger lifecycle, start/run split
- 04-dx-types.md — 12 DX convenience types (blanket impls)
- 05-context-capabilities.md — slim context, rich resources
- 06-handlers-registry.md — type-erased handlers, versioned registry
- 07-errors-result-derives.md — error classification, ActionResult
- 08-testing-docs-phases.md — test harnesses, assertion macros
- addendum — AgentAction, Provide ports, Task<T>, CachePolicy

---

## 1. Core Type Hierarchy (from plans, unchanged)

```
Action (base trait — metadata)
├── StatelessAction     — input → output, no state (80% of nodes)
├── StatefulAction      — input + state → output, iteration/pagination
├── TriggerAction       — start/run lifecycle, spawns executions
├── ResourceAction      — configure/cleanup, graph-scoped DI
└── AgentAction         — LLM agent with tools, budget, streaming (addendum)
```

DX convenience layers (blanket impls, engine never sees):
```
StatelessAction
├── SimpleAction        — Output = Value, auto-wrap Success
└── TransformAction     — pure sync fn(Value) → Value

StatefulAction
├── PaginatedAction     — auto cursor management
└── BatchAction         — auto item iteration with progress

TriggerAction
├── WebhookAction       — HTTP webhook lifecycle hooks
├── PollAction          — interval-based polling
└── ScheduledTrigger    — cron expression trigger
```

### v1 scope (first 10 nodes)
Implement: SimpleAction, TransformAction, WebhookAction, PollAction

### v1.1 scope
Implement: PaginatedAction, BatchAction, ScheduledTrigger

### Deferred (needs engine support)
InteractiveAction, TransactionalAction, StreamingAction, ProcessAction, QueueAction

---

## 2. Parameter v4 Integration — Struct = Input = Schema

Action struct with `#[derive(Parameters)]` IS the typed input AND the parameter schema:

```rust
#[derive(Action, Parameters, Deserialize)]
#[action(key = "http.request", name = "HTTP Request")]
#[action(credential = "bearer_secret")]
struct HttpRequest {
    #[param(label = "URL", hint = "url")]
    #[validate(required, url)]
    url: String,

    #[param(default = "GET")]
    method: HttpMethod,

    #[param(label = "Headers")]
    headers: Option<Vec<Header>>,

    #[param(label = "Timeout (s)")]
    #[validate(range(1..=300))]
    timeout: Option<u32>,
}
```

**What `#[derive(Action)]` generates:**
```rust
// 1. Action trait — metadata with parameters from #[derive(Parameters)]
impl Action for HttpRequest {
    fn metadata() -> &'static ActionMetadata {
        static META: OnceLock<ActionMetadata> = OnceLock::new();
        META.get_or_init(|| ActionMetadata {
            key: action_key!("http.request"),
            name: "HTTP Request".into(),
            parameters: Self::parameters(),  // from HasParameters
            // ... ports, version, etc from #[action(...)] attrs
        })
    }
}

// 2. ActionDependencies — from #[action(credential, resource)] attrs
impl ActionDependencies for HttpRequest {
    fn credentials() -> Vec<CredentialKey> {
        vec![credential_key!("bearer_secret")]
    }
}

// 3. InternalHandler adapter — deserializes JSON → Self, calls execute
// (framework generates the adapter at registration time)
```

**The developer writes only:**
```rust
impl Execute for HttpRequest {
    type Output = Value;

    async fn execute(&self, ctx: &ActionContext) -> ActionResult<Value> {
        let cred = ctx.credential::<BearerSecret>()?;
        // self.url, self.method, self.headers, self.timeout — typed!
        // Full ActionResult access: Success, Branch, Route, Wait, etc.
        ActionResult::success(json!({ "status": 200 }))
    }
}
```

**`Execute` trait** — simplified interface for the common case:
```rust
/// Simplified execution trait. Struct IS the input (deserialized from parameters).
pub trait Execute: HasParameters + Send + Sync + 'static {
    type Output: Serialize + Send;

    fn execute(
        &self,
        ctx: &ActionContext,
    ) -> impl Future<Output = ActionResult<Self::Output>> + Send;
}

/// Blanket: any Execute type is a StatelessAction with Input = Self
impl<A: Execute + DeserializeOwned> StatelessAction for A {
    type Input = A;
    type Output = A::Output;

    async fn execute(
        &self,
        _input: Self::Input, // = self, already deserialized
        ctx: &ActionContext,
    ) -> ActionResult<Self::Output> {
        Execute::execute(self, ctx).await
    }
}
```

For actions needing full `StatelessAction` control (custom Input type, non-self input):
```rust
// Full trait — no derive needed for StatelessAction
impl StatelessAction for CustomAction {
    type Input = CustomInput;  // different from Self
    type Output = CustomOutput;
    
    async fn execute(&self, input: Self::Input, ctx: &ActionContext)
        -> ActionResult<Self::Output>
    { ... }
}
```

---

## 3. Credential v3 Integration — Typed Access

### Current problem
`ctx.credential_typed::<S>(id)` works but requires manual type annotation + string ID.

### Solution
`#[action(credential = "...")]` declares requirements. `ctx.credential::<T>()` resolves typed, validated at registration.

```rust
impl ActionContext {
    /// Get typed credential. Type validated against ActionDependencies at registration.
    pub fn credential<S: AuthScheme>(&self) -> Result<S, ActionError> {
        self.credentials
            .resolve_typed::<S>()
            .map_err(|e| ActionError::fatal(format!("credential error: {e}")))
    }
}
```

For actions requiring multiple credentials:
```rust
#[action(credentials = ["oauth2", "signing_key"])]
struct MultiAuthAction { ... }

impl Execute for MultiAuthAction {
    async fn execute(&self, ctx: &ActionContext) -> ActionResult<Value> {
        let oauth = ctx.credential_by_key::<OAuth2Token>("oauth2")?;
        let signing = ctx.credential_by_key::<SigningKey>("signing_key")?;
        // ...
    }
}
```

---

## 4. Resource v2 Integration — Typed Access

### Current problem
`ctx.resource(key)` returns `Box<dyn Any>` — manual downcast.

### Solution
Typed accessor with compile-time safety:

```rust
impl ActionContext {
    /// Get typed resource. Resource validated against ActionDependencies at registration.
    pub fn resource<R: Resource>(&self) -> Result<R::Lease, ActionError> {
        self.resources
            .acquire_typed::<R>()
            .map_err(|e| ActionError::fatal(format!("resource error: {e}")))
    }
}

#[action(resource = "http_client")]
struct HttpRequest { ... }

impl Execute for HttpRequest {
    async fn execute(&self, ctx: &ActionContext) -> ActionResult<Value> {
        let client = ctx.resource::<HttpResource>()?;  // typed!
        // ...
    }
}
```

---

## 5. Handler Layer (from plan 06)

Type-erased handler adapters for engine consumption:

```rust
/// What the engine sees — type-erased.
pub trait InternalHandler: Send + Sync {
    fn execute(
        &self,
        input: Value,
        ctx: &ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionResult<Value>, ActionError>> + Send>>;

    fn metadata(&self) -> &ActionMetadata;
}

/// Adapter: StatelessAction → InternalHandler
pub struct StatelessAdapter<A: StatelessAction> { ... }

/// Adapter: StatefulAction → InternalHandler (manages state load/save)
pub struct StatefulAdapter<A: StatefulAction> { ... }

/// Adapter: TriggerAction → TriggerHandler (separate interface)
pub struct TriggerAdapter<A: TriggerAction> { ... }
```

---

## 6. ActionRegistry (from plan 06)

Version-aware action lookup:

```rust
pub struct ActionRegistry {
    /// (ActionKey, InterfaceVersion) → handler
    handlers: DashMap<VersionedActionKey, Arc<dyn InternalHandler>>,
    /// ActionKey → latest version
    latest: DashMap<ActionKey, InterfaceVersion>,
}

impl ActionRegistry {
    pub fn register<A: Action + StatelessAction>(&self, action: A) -> Result<()>;
    pub fn get(&self, key: &ActionKey, version: &InterfaceVersion) -> Option<Arc<dyn InternalHandler>>;
    pub fn get_latest(&self, key: &ActionKey) -> Option<Arc<dyn InternalHandler>>;
}
```

---

## 7. Test Fixtures (from plan 08)

```rust
/// Builder for test ActionContext with mock capabilities.
pub struct TestContextBuilder {
    credentials: HashMap<String, Value>,
    resources: HashMap<String, Box<dyn Any + Send + Sync>>,
    logger: SpyLogger,
}

impl TestContextBuilder {
    pub fn new() -> Self;
    pub fn with_credential<S: AuthScheme>(self, key: &str, scheme: S) -> Self;
    pub fn with_resource<R: Any + Send + Sync>(self, key: &str, resource: R) -> Self;
    pub fn build(self) -> ActionContext;
}

/// Logger that captures all log calls for assertion.
pub struct SpyLogger { entries: Arc<Mutex<Vec<LogEntry>>> }

impl SpyLogger {
    pub fn entries(&self) -> Vec<LogEntry>;
    pub fn contains(&self, message: &str) -> bool;
}

/// Assertion macros
macro_rules! assert_success {
    ($result:expr) => { assert!(matches!($result, ActionResult::Success { .. })) };
}
macro_rules! assert_branch {
    ($result:expr, $key:expr) => {
        assert!(matches!($result, ActionResult::Branch { selected, .. } if selected == $key))
    };
}
```

Usage:
```rust
#[tokio::test]
async fn test_http_request() {
    let ctx = TestContextBuilder::new()
        .with_credential("bearer_secret", BearerSecret { token: "test".into() })
        .build();

    let action = HttpRequest {
        url: "https://example.com".into(),
        method: HttpMethod::Get,
        headers: None,
        timeout: Some(30),
    };

    let result = action.execute(&ctx).await;
    assert_success!(result);
}
```

---

## 8. DX Types v1 (from plan 04)

### SimpleAction
```rust
/// Blanket impl: any Execute with Output = Value is a SimpleAction.
/// Auto-wraps return value in ActionResult::Success.
pub trait SimpleAction: HasParameters + Send + Sync + 'static {
    fn run(&self, ctx: &ActionContext) -> impl Future<Output = Result<Value, ActionError>> + Send;
}

impl<A: SimpleAction + DeserializeOwned> Execute for A {
    type Output = Value;
    async fn execute(&self, ctx: &ActionContext) -> ActionResult<Value> {
        match self.run(ctx).await {
            Ok(value) => ActionResult::success(value),
            Err(e) => ActionResult::from_error(e),
        }
    }
}
```

### TransformAction
```rust
/// Pure synchronous transformation. No context, no side effects.
/// Ideal for If/Switch, data mapping, formatting nodes.
pub trait TransformAction: HasParameters + Send + Sync + 'static {
    fn transform(&self, input: Value) -> Result<Value, ActionError>;
}

// Blanket to SimpleAction → Execute → StatelessAction
impl<A: TransformAction + DeserializeOwned> SimpleAction for A {
    async fn run(&self, ctx: &ActionContext) -> Result<Value, ActionError> {
        let input = ctx.input_data().clone();
        self.transform(input)
    }
}
```

---

## 9. Port System (from plans, unchanged)

```rust
pub enum PortKind {
    /// Main data flow.
    Flow(FlowKind),
    /// Sub-node input for composition (AI tools, memory, models).
    Support(SupportPort),
    /// Capability provision (inverse of Support).
    Provide(ProvideKind),
    /// Config-driven dynamic ports.
    Dynamic(DynamicPort),
}
```

DataTag registry (58+ tags) for typed wiring — editor-only enforcement.

---

## 10. What Changes vs Current

| Area | Current | New |
|------|---------|-----|
| Action authoring | Manual impl Action + StatelessAction + ActionDependencies | `#[derive(Action, Parameters)]` + impl Execute |
| Input typing | `type Input` separate from action struct | Struct IS the input (Parameters generates form) |
| Credential access | `ctx.credential_typed::<S>(id)` manual | `ctx.credential::<S>()` typed, validated at registration |
| Resource access | `Box<dyn Any>` downcast | `ctx.resource::<R>()` typed |
| Metadata | Manual ActionMetadata construction | Generated from derive attributes |
| Dependencies | Manual ActionDependencies impl | From `#[action(credential, resource)]` attrs |
| Registry | Basic HashMap | VersionedActionKey with version-aware lookup |
| Testing | Manual context construction | TestContextBuilder + SpyLogger + assertion macros |
| DX types | None | SimpleAction, TransformAction, WebhookAction, PollAction (v1) |
| Handler layer | StatelessActionAdapter only | Full adapter set (Stateless, Stateful, Trigger, Resource) |

---

## 11. Implementation Phases (from plan 08, adapted)

| Phase | What | Depends on |
|-------|------|------------|
| 1 | Execute trait + derive(Action) macro + StatelessAdapter | parameter v4 |
| 2 | Typed credential/resource accessors on ActionContext | credential v3, resource v2 |
| 3 | SimpleAction + TransformAction blanket impls | Phase 1 |
| 4 | ActionRegistry with VersionedActionKey | Phase 1 |
| 5 | Test fixtures (TestContextBuilder, SpyLogger, macros) | Phase 2 |
| 6 | WebhookAction + PollAction | Phase 1 + trigger lifecycle |
| 7 | Handler adapters (Stateful, Trigger, Resource) | Phase 1 |
| 8 | StatefulAction durable commit + state migration | engine integration |
| 9 | AgentAction + AgentContext | Phase 7 |
| 10 | DataTag registry | Phase 4 |

---

## 12. Not In Scope (preserved from plans, deferred)

- InteractiveAction (needs engine suspended execution support)
- TransactionalAction (needs engine rollback orchestration)
- StreamingAction (needs runtime SpillToBlob)
- ProcessAction (needs runtime sandbox Phase 2)
- QueueAction (deployment topology decision pending)
- CachePolicy / incremental execution (engine DAG-level concern)
- OutputPort::Provide enforcement (editor concern)
- Task<T> structured concurrency (Phase 2)
