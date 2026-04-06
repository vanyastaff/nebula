# nebula-action v2 — Design Spec

## Goal

Implement the action architecture (plans 01–08 + addendum) with integration for parameter v4, credential v3, and resource v2. Derive macro reduces boilerplate; core traits unchanged.

## Philosophy

- **Protocol, not runtime.** Action crate defines contracts. Engine/runtime interpret them.
- **5 core traits, no extras.** StatelessAction, StatefulAction, TriggerAction, ResourceAction, AgentAction. No SimpleAction/Execute/TransformAction wrappers — developers implement the real traits directly.
- **Derive = boilerplate reduction, not new abstractions.** `#[derive(Action)]` generates metadata + dependencies + handler adapter. Developer implements the execution trait.
- **Credential access always keyed.** `ctx.credential::<T>(key)` — type + key. `ctx.credential_opt::<T>(key)` for optional.
- **Works with current stack.** Phase 1 uses existing parameter/credential/resource APIs. Typed upgrades come when upstream crates land.

## Post-Review Amendments

1. **Removed Execute, SimpleAction, TransformAction.** Unnecessary layers. Developer implements StatelessAction directly — it has full ActionResult access (Branch, Route, Wait, etc.).
2. **Credential access always keyed.** `ctx.credential::<T>(key)` + `ctx.credential_opt::<T>(key)`. No "magic single credential" resolution.
3. **Optional credentials supported.** `#[action(credential(optional) = "bearer_secret")]`.
4. **If/Switch uses StatelessAction** with `ActionResult::Branch`, not TransformAction.
5. **Derive scope limited.** `#[derive(Action)]` reads only `#[action(...)]` attributes. Calls `HasParameters::parameters()` at runtime — never parses `#[param]` attributes.
6. **Fallback path.** Phase 1 works with current crate versions. Typed access upgraded when upstream lands.
7. **StatefulAction example added.**
8. **Acknowledged as evolution**, not pure preservation of frozen plans.

## Source Plans

Architectural decisions from `crates/action/plans/` 01–08 + addendum.

---

## 1. Core Type Hierarchy — 5 Traits, No Extras

```
Action (base trait — metadata)
├── StatelessAction     — input → output (80% of nodes)
├── StatefulAction      — input + state → output (iteration, pagination)
├── TriggerAction       — start/run lifecycle (webhook, poll, cron)
├── ResourceAction      — configure/cleanup (graph-scoped DI)
└── AgentAction         — LLM agent with tools, budget, streaming
```

No convenience wrapper traits. Developer picks the right trait and implements it.

---

## 2. Derive Macro — Boilerplate Only

`#[derive(Action)]` generates:
- `Action` trait impl (metadata from `#[action(...)]` attributes)
- `ActionDependencies` impl (credentials/resources from `#[action(...)]`)
- Handler adapter registration helper

`#[derive(Action)]` does NOT generate execution logic. Developer writes `impl StatelessAction` (or other trait) themselves.

### Example: HTTP Request

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

// Developer writes the actual execution:
impl StatelessAction for HttpRequest {
    type Input = Self;      // struct IS the input
    type Output = Value;

    async fn execute(&self, _input: Self, ctx: &ActionContext) -> ActionResult<Value> {
        let cred = ctx.credential::<BearerSecret>("bearer_secret")?;
        // self.url, self.method — typed fields from parameters
        // Full ActionResult: Success, Branch, Route, Wait, Skip, etc.
        ActionResult::success(json!({ "status": 200 }))
    }
}
```

### Example: If/Switch (branching)

```rust
#[derive(Action, Parameters, Deserialize)]
#[action(key = "core.if", name = "If")]
struct IfSwitch {
    #[param(label = "Condition")]
    #[validate(required)]
    condition: String,

    #[param(label = "Mode", default = "expression")]
    mode: IfMode,
}

impl StatelessAction for IfSwitch {
    type Input = Self;
    type Output = Value;

    async fn execute(&self, _input: Self, ctx: &ActionContext) -> ActionResult<Value> {
        let input_data = ctx.input_data();
        let result = evaluate_condition(&self.condition, input_data);

        if result {
            ActionResult::branch("true", input_data.clone())
        } else {
            ActionResult::branch("false", input_data.clone())
        }
    }
}
```

### Example: Paginated Fetch (StatefulAction)

```rust
#[derive(Action, Parameters, Deserialize)]
#[action(key = "api.paginated_fetch", name = "Paginated Fetch")]
#[action(credential = "bearer_secret")]
struct PaginatedFetch {
    #[param(label = "URL")]
    #[validate(required, url)]
    url: String,

    #[param(label = "Max Pages", default = 10)]
    #[validate(range(1..=100))]
    max_pages: u32,
}

#[derive(Serialize, Deserialize, Clone)]
struct PaginationState {
    cursor: Option<String>,
    pages_fetched: u32,
}

impl StatefulAction for PaginatedFetch {
    type Input = Self;
    type Output = Value;
    type State = PaginationState;

    fn init_state(&self) -> PaginationState {
        PaginationState { cursor: None, pages_fetched: 0 }
    }

    async fn execute(
        &self,
        _input: Self,
        state: &mut PaginationState,
        ctx: &ActionContext,
    ) -> ActionResult<Value> {
        let cred = ctx.credential::<BearerSecret>("bearer_secret")?;
        let url = match &state.cursor {
            Some(c) => format!("{}?cursor={}", self.url, c),
            None => self.url.clone(),
        };

        let response = fetch_page(&url, &cred).await?;
        state.cursor = response.next_cursor.clone();
        state.pages_fetched += 1;

        if state.cursor.is_some() && state.pages_fetched < self.max_pages {
            ActionResult::r#continue(response.data, Some(state.pages_fetched as f64 / self.max_pages as f64))
        } else {
            ActionResult::break_completed(response.data)
        }
    }
}
```

### Example: Webhook Trigger

```rust
#[derive(Action)]
#[action(key = "webhook.trigger", name = "Webhook Trigger")]
struct WebhookTrigger;

impl TriggerAction for WebhookTrigger {
    async fn start(&self, ctx: &TriggerContext) -> Result<(), ActionError> {
        ctx.register_webhook("/my-webhook", HttpMethod::POST).await?;
        Ok(())
    }

    async fn on_event(&self, event: Value, ctx: &TriggerContext) -> Result<(), ActionError> {
        ctx.emit(event).await?;
        Ok(())
    }

    async fn stop(&self, ctx: &TriggerContext) -> Result<(), ActionError> {
        ctx.unregister_webhook("/my-webhook").await?;
        Ok(())
    }
}
```

---

## 3. Credential Access — Always Keyed

```rust
impl ActionContext {
    /// Get typed credential by key. Fails if not found or type mismatch.
    pub fn credential<S: AuthScheme>(&self, key: &str) -> Result<S, ActionError>;

    /// Get typed credential optionally. Returns None if not configured.
    pub fn credential_opt<S: AuthScheme>(&self, key: &str) -> Result<Option<S>, ActionError>;
}
```

Declaration in derive:
```rust
#[action(credential = "api_key")]                     // required
#[action(credential(optional) = "signing_key")]       // optional
#[action(credentials = ["oauth2", "signing_key"])]    // multiple
```

---

## 4. Resource Access — Typed

```rust
impl ActionContext {
    /// Get typed resource lease by key.
    pub fn resource<R: Resource>(&self, key: &str) -> Result<R::Lease, ActionError>;

    /// Get typed resource optionally.
    pub fn resource_opt<R: Resource>(&self, key: &str) -> Result<Option<R::Lease>, ActionError>;
}
```

---

## 5. Handler Layer (from plan 06)

Type-erased handler adapters for engine consumption:

```rust
pub trait InternalHandler: Send + Sync {
    fn execute(
        &self,
        input: Value,
        ctx: &ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionResult<Value>, ActionError>> + Send>>;

    fn metadata(&self) -> &ActionMetadata;
}

// Adapters for each core type:
pub struct StatelessAdapter<A: StatelessAction> { ... }
pub struct StatefulAdapter<A: StatefulAction> { ... }
// TriggerAdapter, ResourceAdapter — separate handler trait (TriggerHandler)
```

---

## 6. ActionRegistry — Version-Aware (from plan 06)

```rust
pub struct ActionRegistry {
    handlers: DashMap<VersionedActionKey, Arc<dyn InternalHandler>>,
    latest: DashMap<ActionKey, InterfaceVersion>,
}

impl ActionRegistry {
    pub fn register<A>(&self, action: A) -> Result<()>
    where
        A: Action + StatelessAction + Send + Sync + 'static;

    pub fn get(&self, key: &ActionKey, version: &InterfaceVersion) -> Option<Arc<dyn InternalHandler>>;
    pub fn get_latest(&self, key: &ActionKey) -> Option<Arc<dyn InternalHandler>>;
}
```

---

## 7. Test Fixtures (from plan 08)

```rust
pub struct TestContextBuilder {
    credentials: HashMap<String, Box<dyn Any + Send + Sync>>,
    resources: HashMap<String, Box<dyn Any + Send + Sync>>,
    input_data: Value,
    logger: SpyLogger,
}

impl TestContextBuilder {
    pub fn new() -> Self;
    pub fn with_credential<S: AuthScheme>(self, key: &str, scheme: S) -> Self;
    pub fn with_resource<R: Any + Send + Sync>(self, key: &str, resource: R) -> Self;
    pub fn with_input(self, data: Value) -> Self;
    pub fn build(self) -> ActionContext;
}

pub struct SpyLogger { /* captures log entries */ }

// Assertion macros
macro_rules! assert_success { ($result:expr) => { ... } }
macro_rules! assert_branch { ($result:expr, $key:expr) => { ... } }
macro_rules! assert_continue { ($result:expr) => { ... } }
```

Usage:
```rust
#[tokio::test]
async fn test_http_request() {
    let action = HttpRequest {
        url: "https://example.com".into(),
        method: HttpMethod::Get,
        headers: None,
        timeout: Some(30),
    };

    let ctx = TestContextBuilder::new()
        .with_credential::<BearerSecret>("bearer_secret", BearerSecret {
            token: SecretString::new("test-token"),
        })
        .with_input(json!({}))
        .build();

    let result = action.execute(action.clone(), &ctx).await;
    assert_success!(result);
}
```

---

## 8. Port System + DataTag Registry (from plans, unchanged)

Port system: Flow / Support / Provide / Dynamic — unchanged.

DataTag registry: 58+ hierarchical tags — unchanged.

Both are implementation tasks per plans, no design changes needed.

---

## 9. Implementation Phases

| Phase | What | Depends on |
|-------|------|------------|
| 1 | `#[derive(Action)]` macro + StatelessAdapter | parameter HasParameters trait |
| 2 | Keyed credential/resource access on ActionContext | current credential/resource APIs |
| 3 | ActionRegistry with VersionedActionKey | Phase 1 |
| 4 | Test fixtures (TestContextBuilder, SpyLogger, macros) | Phase 2 |
| 5 | StatefulAdapter + state init/migration | Phase 1 |
| 6 | TriggerAction lifecycle (start/run/stop) | Phase 1 |
| 7 | Handler adapters (Resource, Agent) | Phase 5 |
| 8 | DataTag registry impl | Phase 3 |
| 9 | Upgrade credential/resource access when v3/v2 land | upstream crates |
| 10 | AgentAction + AgentContext | Phase 7 |

**Phase 1-4 = minimum viable** (first node works end-to-end).

---

## 10. What Changes vs Current

| Area | Current | New |
|------|---------|-----|
| Action authoring | Manual impl Action + StatelessAction + ActionDependencies | `#[derive(Action, Parameters)]` + impl StatelessAction |
| Input typing | Separate `type Input` | Struct IS the input (`type Input = Self`) |
| Credential access | `credential_typed::<S>(id)` | `credential::<S>(key)` + `credential_opt::<S>(key)` |
| Resource access | `Box<dyn Any>` downcast | `resource::<R>(key)` typed |
| Metadata | Manual ActionMetadata construction | Generated from `#[action(...)]` attrs |
| Dependencies | Manual ActionDependencies impl | From `#[action(credential, resource)]` |
| Registry | Basic HashMap | VersionedActionKey with version-aware lookup |
| Testing | Manual context construction | TestContextBuilder + SpyLogger + assertion macros |
| Extra traits | None | None (removed Execute, SimpleAction, TransformAction) |

---

## 11. Not In Scope

- Execute / SimpleAction / TransformAction convenience traits (removed)
- InteractiveAction (needs engine suspended execution)
- TransactionalAction (needs engine rollback)
- StreamingAction (needs runtime SpillToBlob)
- ProcessAction (needs sandbox Phase 2)
- QueueAction (deployment topology pending)
- CachePolicy (engine DAG concern)
- Task<T> structured concurrency (Phase 2)

---

## Post-Conference Round 2 Amendments

### B1. IdempotencyManager must be durable (Stripe)
`IdempotencyManager` backed by `Storage` trait (Postgres in production), not in-memory HashSet. Keys are deterministic (execution_id + node_id + attempt) — survives process restarts. V1 blocker for financial workflows.

### B2. Compensating transactions — author responsibility (Stripe)
No engine-level saga for v1. Document: if node N succeeds but N+1 fails, action N handles compensation. Engine saga (TransactionalAction) is post-v1.

### B3. Deserialization depth limit (Notion)
Add serde_json recursion limit (default 128) at StatelessAdapter deserialization boundary. Prevents stack overflow from deeply nested inputs.

### B4. BlobStorage accepts AsyncRead (Instagram)
`BlobStorage::write` changed to streaming to avoid double-memory for large payloads.

### B5. Derive macro semver contract (Figma)
`#[derive(Action)]` output stability is governed by semver on the `Action` trait, `ActionMetadata` struct, and `ActionDependencies` trait. Breaking changes to these types = major version bump. Macro internals (code generation patterns) are NOT part of the public API — only the generated trait impls are.

---

## Serialization Strategy

See `2026-04-06-serialization-strategy-design.md` for cross-cutting serialization decisions affecting this crate.
