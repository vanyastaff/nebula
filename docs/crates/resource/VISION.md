# nebula-resource — Target Design (AAAAA+ Edition)

> **Version**: 0.1.0 | **Created**: 2026-03-02  
> **Goal**: The best resource crate in the Rust ecosystem — simple, powerful, safe, performant, and future-proof for 10+ years.  
> **Baseline**: Rust 1.93, edition 2024, `impl Future` in traits (async fn in traits not yet stable).

This document shows **how everything would look** if we rebuilt from scratch with full knowledge of:
- Diverse resource types (HTTP, SSH, Telegram, Google Drive, LLM providers, databases, caches)
- Different auth patterns (API key, Bearer, OAuth2, certificates, bot tokens)
- Developer convenience (one-liner usage, no manual guard juggling)
- Per-call hooks (counters, logging, retry — inside the Instance, not just acquire/release)
- AI-decade: predictable patterns for code generation and discovery

---

## Part 1: What a Resource Author Writes

### Minimal resource (no auth, no hooks)

```rust
use nebula_sdk::prelude::*;

pub struct RedisResource;

#[derive(Clone, Deserialize)]
pub struct RedisConfig {
    pub url: String,
    #[serde(default = "default_pool")]
    pub max_connections: usize,
}

fn default_pool() -> usize { 8 }

impl Config for RedisConfig {
    fn validate(&self) -> resource::Result<()> {
        if self.url.is_empty() {
            return Err(resource::Error::configuration("url cannot be empty"));
        }
        Ok(())
    }
}

pub struct RedisInstance {
    conn: redis::Client,
}

impl RedisInstance {
    pub async fn get(&self, key: &str) -> Result<Option<String>, RedisError> {
        // ...
    }

    pub async fn set(&self, key: &str, value: &str) -> Result<(), RedisError> {
        // ...
    }
}

impl Resource for RedisResource {
    type Config = RedisConfig;
    type Instance = RedisInstance;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::build("redis_cache", "Redis Cache")
            .description("Shared Redis client for caching")
            .tag("protocol:tcp")
            .tag("service:redis")
            .finish()
    }

    fn create(
        &self,
        config: &Self::Config,
        _ctx: &Context,
    ) -> impl Future<Output = resource::Result<Self::Instance>> + Send {
        let url = config.url.clone();
        async move {
            let conn = redis::Client::open(url)
                .map_err(|e| resource::Error::initialization("redis_cache", e.to_string()))?;
            Ok(RedisInstance { conn })
        }
    }
}
```

**What the author wrote**: one struct, one Config, one Instance with domain methods (`get`/`set`), and `Resource` impl. Nothing else.

---

### Resource with auth (HTTP + credentials)

Auth is done in `create()` via `ctx.credentials().get("key")`; there is no separate `AuthStrategy` type. The Instance receives `ctx.recorder()` for optional Tier 2 call enrichment (e.g. `get`/`post` record `CallRecord` when enrichment is enabled).

```rust
use nebula_sdk::prelude::*;

pub struct HttpResource;

#[derive(Clone, Deserialize)]
pub struct HttpConfig {
    pub base_url: Option<String>,
    pub timeout_ms: Option<u64>,
    /// Credential key for Bearer token (e.g. "api_token")
    pub credential_key: Option<String>,
}

impl Config for HttpConfig {
    fn validate(&self) -> resource::Result<()> {
        if let Some(url) = &self.base_url {
            url::Url::parse(url)
                .map_err(|e| resource::Error::configuration(format!("invalid base_url: {e}")))?;
        }
        Ok(())
    }
}

pub struct HttpInstance {
    client: reqwest::Client,
    base_url: Option<url::Url>,
    recorder: Arc<dyn Recorder>,
    resource_key: ResourceKey,
}

impl HttpInstance {
    pub async fn get(&self, path: &str) -> Result<Response, HttpError> {
        let url = self.resolve_url(path);
        let started = Instant::now();
        let result = self.client.get(&url).send().await;
        if self.recorder.is_enrichment_enabled() {
            self.recorder.record_call(CallRecord { /* operation, duration, status, ... */ });
        }
        result.map_err(Into::into)
    }

    pub async fn post(&self, path: &str, body: impl Into<Body>) -> Result<Response, HttpError> {
        let url = self.resolve_url(path);
        let result = self.client.post(&url).body(body.into()).send().await;
        if self.recorder.is_enrichment_enabled() {
            self.recorder.record_call(CallRecord { /* ... */ });
        }
        result.map_err(Into::into)
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    fn resolve_url(&self, path: &str) -> String {
        match &self.base_url {
            Some(base) => format!("{}{}", base.trim_end_matches('/'), path),
            None => path.to_string(),
        }
    }
}

impl Resource for HttpResource {
    type Config = HttpConfig;
    type Instance = HttpInstance;

    fn metadata(&self) -> ResourceMetadata {
        let key = ResourceKey::try_from("http.client").expect("valid key");
        ResourceMetadata::build(key, "HTTP Client", "Shared HTTP client with connection pooling and timeouts")
            .tag("protocol:http")
            .build()
    }

    fn create(
        &self,
        config: &Self::Config,
        ctx: &Context,
    ) -> impl Future<Output = resource::Result<Self::Instance>> + Send {
        let cfg = config.clone();
        let resource_key = self.metadata().key.clone();
        let recorder = ctx.recorder();
        let credentials = ctx.credentials().map(Arc::clone);
        async move {
            let mut builder = reqwest::Client::builder();
            if let Some(ms) = cfg.timeout_ms {
                builder = builder.timeout(Duration::from_millis(ms));
            }
            if let Some(cred_key) = &cfg.credential_key {
                if let Some(creds) = &credentials {
                    let token = creds.get(cred_key).await?;
                    builder = builder.bearer_auth(token.expose().as_str());
                }
            }
            let client = builder.build()
                .map_err(|e| resource::Error::configuration(format!("HTTP client: {e}")))?;
            let base_url = cfg.base_url
                .map(|u| url::Url::parse(&u))
                .transpose()
                .map_err(|e| resource::Error::configuration(format!("base_url: {e}")))?;
            Ok(HttpInstance { client, base_url, recorder, resource_key })
        }
    }
}
```

---

### Resource with deps (Google Drive depends on HTTP + Credential)

```rust
pub struct GoogleDriveResource;

#[derive(Clone, Deserialize)]
pub struct GoogleDriveConfig {
    pub credential_key: String,
}

impl Config for GoogleDriveConfig {
    fn validate(&self) -> resource::Result<()> {
        if self.credential_key.is_empty() {
            return Err(resource::Error::configuration("credential_key is required"));
        }
        Ok(())
    }
}

pub struct GoogleDriveInstance {
    http: HttpInstance,
    access_token: String,
}

impl GoogleDriveInstance {
    pub async fn list_files(&self, folder_id: &str) -> Result<Vec<DriveFile>, DriveError> {
        let resp = self.http
            .get(&format!("/drive/v3/files?q='{}'+in+parents", folder_id))
            .await?;
        // parse response...
    }

    pub async fn download(&self, file_id: &str) -> Result<Bytes, DriveError> {
        let resp = self.http
            .get(&format!("/drive/v3/files/{}?alt=media", file_id))
            .await?;
        // return bytes...
    }
}

impl Resource for GoogleDriveResource {
    type Config = GoogleDriveConfig;
    type Instance = GoogleDriveInstance;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::build("google_drive", "Google Drive")
            .description("Google Drive file operations via REST API")
            .tag("service:google_drive")
            .tag("protocol:https")
            .finish()
    }

    fn create(
        &self,
        config: &Self::Config,
        ctx: &Context,
    ) -> impl Future<Output = resource::Result<Self::Instance>> + Send {
        let cred_key = config.credential_key.clone();
        let creds = ctx.credentials().map(Arc::clone);
        async move {
            let creds = creds.ok_or_else(|| {
                resource::Error::configuration("GoogleDriveResource requires credentials in context")
            })?;

            let token = creds.get(&cred_key).await?;

            let http_config = HttpConfig {
                base_url: Some("https://www.googleapis.com".into()),
                timeout_ms: Some(30_000),
                auth: Some(AuthStrategy::Bearer {
                    credential_key: cred_key,
                }),
            };
            let http = HttpResource.create(&http_config, ctx).await?;

            Ok(GoogleDriveInstance {
                http,
                access_token: token.expose().to_string(),
            })
        }
    }
}
```

---

### Resource with per-call interceptors (Telegram bot with counter)

```rust
pub struct TelegramBotResource;

#[derive(Clone, Deserialize)]
pub struct TelegramConfig {
    pub token_credential_key: String,
}

pub struct TelegramInstance {
    http: HttpInstance,
    base_url: String,
}

impl TelegramInstance {
    pub async fn send_message(
        &self,
        chat_id: i64,
        text: &str,
    ) -> Result<TelegramMessage, TelegramError> {
        let body = serde_json::json!({ "chat_id": chat_id, "text": text });
        let resp = self.http
            .post(&format!("{}/sendMessage", self.base_url), body.to_string())
            .await?;
        // parse...
    }

    pub async fn get_updates(&self) -> Result<Vec<Update>, TelegramError> {
        let resp = self.http
            .get(&format!("{}/getUpdates", self.base_url))
            .await?;
        // parse...
    }
}

impl Resource for TelegramBotResource {
    type Config = TelegramConfig;
    type Instance = TelegramInstance;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::build("telegram_bot", "Telegram Bot")
            .description("Telegram Bot API client")
            .tag("service:telegram")
            .tag("protocol:https")
            .finish()
    }

    fn create(
        &self,
        config: &Self::Config,
        ctx: &Context,
    ) -> impl Future<Output = resource::Result<Self::Instance>> + Send {
        let cred_key = config.token_credential_key.clone();
        let creds = ctx.credentials().map(Arc::clone);
        async move {
            let creds = creds.ok_or_else(|| {
                resource::Error::configuration("TelegramBotResource requires credentials")
            })?;
            let token = creds.get(&cred_key).await?;

            // HTTP client with a counter interceptor
            let mut http_config = HttpConfig {
                base_url: None,
                timeout_ms: Some(60_000),
                auth: None,
            };
            let mut http = HttpResource.create(&http_config, ctx).await?;

            // Add a per-call counter interceptor
            http.interceptors.push(CounterInterceptor::new("telegram_api_calls"));

            let base_url = format!("https://api.telegram.org/bot{}", token.expose());

            Ok(TelegramInstance { http, base_url })
        }
    }
}
```

**Key**: Per-call observability can live inside the Instance (e.g. custom interceptors) or use the shared Tier 2 [`Recorder`] from `ctx.recorder()`: when `recorder.is_enrichment_enabled()`, the Instance calls `recorder.record_call(CallRecord { ... })` for each operation. No changes to the core crate.

---

## Part 2: What an Action Author Writes

### Option A: Scoped access (recommended — zero guard management)

```rust
#[async_trait]
impl ProcessAction for FetchDataAction {
    type Input = FetchInput;
    type Output = FetchOutput;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        // One call, no guard in scope, instance auto-returned to pool
        let body = ctx.resources()
            .with::<HttpResource, _, _>(|http| async move {
                http.get(&input.url).await
            })
            .await?;

        Ok(ActionResult::success(FetchOutput { body }))
    }
}
```

### Option B: Explicit guard (when you need the instance across multiple calls)

```rust
async fn execute(&self, input: Self::Input, ctx: &ActionContext)
    -> Result<ActionResult<Self::Output>, ActionError>
{
    let http = ctx.resources().acquire::<HttpResource>().await?;
    // Guard<HttpInstance> — Deref to HttpInstance

    let users = http.get("/api/users").await?;
    let stats = http.get("/api/stats").await?;

    Ok(ActionResult::success(Output { users, stats }))
    // guard dropped here, instance returned to pool
}
```

### Option C: Multiple resources in one action

```rust
async fn execute(&self, input: Self::Input, ctx: &ActionContext)
    -> Result<ActionResult<Self::Output>, ActionError>
{
    let db = ctx.resources().acquire::<PostgresResource>().await?;
    let cache = ctx.resources().acquire::<RedisResource>().await?;
    let drive = ctx.resources().acquire::<GoogleDriveResource>().await?;

    let data = db.query("SELECT * FROM documents WHERE id = $1", &[&input.id]).await?;
    cache.set(&format!("doc:{}", input.id), &serde_json::to_string(&data)?).await?;
    drive.upload(&data.name, &data.content).await?;

    Ok(ActionResult::success(Output { synced: true }))
}
```

---

## Part 2.5: Tracing — Two-Tier Model

Execution view is guaranteed by the **kernel** (no reliance on resource authors doing the right thing).

- **Tier 1 (automatic)**  
  Every acquired resource is wrapped in an [`InstrumentedGuard`]. On drop, the kernel records a [`ResourceUsageRecord`]: `resource_key`, `acquired_at`, `wait_duration`, `hold_duration`, `drop_reason` (Released / Panic / Detached). No author effort; timing and success/panic are always captured.

- **Tier 2 (optional enrichment)**  
  The engine injects a [`Recorder`] into [`Context`] (default: [`NoopRecorder`]). Instance code can call `recorder.record_call(CallRecord { operation, started_at, duration, status, request/response, metadata })` for a richer execution timeline (e.g. "GET /users", status code, body summary). Stdlib resources (e.g. [`HttpResource`]) do Tier 2 when `recorder.is_enrichment_enabled()`; community resources may or may not.

Auth is **not** a separate strategy type. Each resource uses `ctx.credentials().get("key")` in `create()` and applies the secret (e.g. Bearer header, API key) when building the client or instance.

---

## Part 3: Core Types That Make It Work

### 3.1 ResourceMetadata (builder)

```rust
pub struct ResourceMetadata {
    pub key: ResourceKey,
    pub name: String,
    pub description: String,
    pub icon: Option<String>,
    pub icon_url: Option<String>,
    pub tags: Vec<String>,
}

impl ResourceMetadata {
    pub fn build(key: &str, name: &str) -> ResourceMetadataBuilder {
        ResourceMetadataBuilder {
            key: ResourceKey::try_from(key).expect("valid resource key"),
            name: name.to_string(),
            description: String::new(),
            icon: None,
            icon_url: None,
            tags: Vec::new(),
        }
    }
}

pub struct ResourceMetadataBuilder { /* fields */ }

impl ResourceMetadataBuilder {
    pub fn description(mut self, d: impl Into<String>) -> Self { self.description = d.into(); self }
    pub fn icon(mut self, i: impl Into<String>) -> Self { self.icon = Some(i.into()); self }
    pub fn tag(mut self, t: impl Into<String>) -> Self { self.tags.push(t.into()); self }
    pub fn finish(self) -> ResourceMetadata {
        ResourceMetadata {
            key: self.key,
            name: self.name,
            description: self.description,
            icon: self.icon,
            icon_url: None,
            tags: self.tags,
        }
    }
}
```

### 3.3 Auth (no AuthStrategy)

There is no `AuthStrategy` enum. Each resource that needs auth uses `ctx.credentials().get("key")` in `create()` and applies the secret when building the client (e.g. Bearer header, API key header). See the HTTP resource example in Part 1.

### 3.4 (Historical) AuthStrategy — deprecated

The following was a previous design; auth is now per-resource via `Context::credentials()` in `create()`.

```rust,ignore
// Deprecated: use ctx.credentials().get("key") in Resource::create() instead.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub enum AuthStrategy {
    ApiKey { header: String, credential_key: String },
    Bearer { credential_key: String },
    Basic { credential_key: String },
    OAuth2ClientCredentials { credential_key: String, token_url: String },
    None,
}

impl AuthStrategy {
    pub async fn apply_to_builder(
        &self,
        builder: reqwest::ClientBuilder,
        creds: Option<&dyn CredentialProvider>,
    ) -> resource::Result<reqwest::ClientBuilder> {
        // resolve credential_key via creds.get(), set headers/auth on builder
        // ...
    }
}
```

### 3.5 Per-call enrichment (Tier 2) and InterceptorChain

Optional per-call data is recorded via `Recorder::record_call` with a `CallRecord` (operation, duration, status, request/response). The engine injects a `Recorder` into `Context`; stdlib resources (e.g. HTTP) call it when `recorder.is_enrichment_enabled()`. There is no generic `InterceptorChain` in the core; each Instance type may implement its own hooks or use the shared Recorder.

The following is an alternative (historical) pattern; Tier 2 uses Recorder instead.

```rust,ignore
/// A call descriptor for interceptor context.
pub struct CallInfo {
    pub method: String,
    pub path: String,
    pub timestamp: Instant,
}

/// Trait for per-call interceptors inside an Instance.
///
/// NOT part of the Resource trait — each Instance type
/// defines what "a call" means and which interceptors it supports.
pub trait Interceptor: Send + Sync {
    fn before(&self, info: &CallInfo) {}
    fn after(&self, info: &CallInfo, success: bool) {}
    fn name(&self) -> &str;
}

/// Chain of interceptors, owned by an Instance.
pub struct InterceptorChain<C> {
    interceptors: Vec<Box<dyn Interceptor>>,
    _phantom: PhantomData<C>,
}

impl<C> InterceptorChain<C> {
    pub fn push(&mut self, interceptor: impl Interceptor + 'static) {
        self.interceptors.push(Box::new(interceptor));
    }
}

// Built-in interceptors:

/// Counts every call. Read the counter from metrics.
pub struct CounterInterceptor { label: String, count: AtomicU64 }

/// Logs every call with tracing.
pub struct TracingInterceptor;

/// Measures call duration as a histogram.
pub struct LatencyInterceptor { label: String }
```

### 3.5 Manager::with_resource (scoped API)

```rust
impl Manager {
    /// Scoped access: acquire → call closure → release.
    /// No guard leaks, no manual Drop, no downcast.
    pub async fn with_resource<R, F, Fut, T>(
        &self,
        ctx: &Context,
        f: F,
    ) -> Result<T>
    where
        R: Resource,
        R::Instance: Any,
        F: FnOnce(&R::Instance) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let guard = self.acquire_typed(R::default_instance(), ctx).await?;
        let result = f(guard.get()).await;
        // guard dropped here — instance returned to pool
        result
    }
}
```

### 3.6 ResourceAccessor (bridge to ActionContext)

```rust
/// Injected into ActionContext by the engine. Wraps Manager.
pub struct ResourceAccessor {
    manager: Arc<Manager>,
    ctx: Context,
}

impl ResourceAccessor {
    /// Scoped access — recommended pattern.
    pub async fn with<R, F, Fut, T>(&self, f: F) -> Result<T>
    where
        R: Resource, R::Instance: Any,
        F: FnOnce(&R::Instance) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        self.manager.with_resource::<R, F, Fut, T>(&self.ctx, f).await
    }

    /// Explicit guard — for multi-call scenarios.
    pub async fn acquire<R: Resource>(&self) -> Result<TypedResourceGuard<R::Instance>>
    where
        R::Instance: Any,
    {
        self.manager.acquire_typed(R::default_instance(), &self.ctx).await
    }
}
```

---

## Part 4: How Everything Connects

```
                    Action Author
                         │
                         ▼
              ctx.resources().with::<HttpResource>(|http| {
                  http.get("/api/data").await    ◄── rich Instance API
              })                                       with interceptors
                         │
                         ▼
                  ResourceAccessor
                   (wraps Manager + Context)
                         │
                         ▼
        ┌─────── Manager ────────┐
        │  scope check           │
        │  quarantine check      │
        │  health check          │
        │  lifecycle hooks       │  ◄── acquire/release hooks (platform-level)
        │  event emission        │
        └────────┬───────────────┘
                 │
                 ▼
         Pool<R> (typed)
           │
           ├─ idle instance? → is_valid → return Guard<Instance>
           │
           └─ no idle? → Resource::create(config, ctx) → new Instance
                                │
                                ├─ resolve AuthStrategy from ctx.credentials()
                                ├─ build client / connection
                                └─ attach InterceptorChain (counters, tracing, etc.)

        Guard<Instance> drops → Resource::recycle() → back to pool
                           or → Resource::cleanup() → destroyed
```

---

## Part 5: Auth Flow for Different Resource Types

| Resource | AuthStrategy | How it works |
|----------|-------------|--------------|
| HttpResource (generic) | Any (Bearer, ApiKey, Basic, OAuth2, None) | `AuthStrategy::apply_to_builder()` sets headers at `create` time |
| GoogleDriveResource | OAuth2ClientCredentials | Config has `credential_key`; `create` resolves token, passes as `Bearer` to inner HttpInstance |
| TelegramBotResource | Custom (bot token in URL) | Config has `token_credential_key`; `create` resolves token, builds base URL with token embedded |
| PostgresResource | Basic (user/password) | Config has `credential_key`; `create` resolves password, builds connection string |
| SshResource | Custom (key file or password) | Config references credential; `create` resolves key material, opens session |
| OpenAiResource | ApiKey | Config has `credential_key`; `create` resolves key, sets `Authorization: Bearer` header |

One `AuthStrategy` enum for common patterns; resources with exotic auth just call `ctx.credentials().get(key)` directly in `create`.

---

## Part 6: Hooks at Every Level

### Level 1: Lifecycle hooks (Manager/Pool — already exists)

| Hook | When | Who sets it | Can cancel? |
|------|------|-------------|-------------|
| Acquire (before/after) | Manager.acquire() | Platform operator | Before: yes |
| Release (before/after) | Guard drop | Platform operator | No (RAII) |
| Create (before/after) | Pool creates instance | Platform operator | Before: yes |
| Cleanup (before/after) | Pool destroys instance | Platform operator | No |

These are **platform-level**. Set once by the operator. Apply to ALL instances of a resource.

### Level 2: Per-call interceptors (Instance — new)

| Interceptor | When | Who sets it | Can cancel? |
|-------------|------|-------------|-------------|
| before | Before each API call (HTTP request, DB query, bot message) | Resource author in `create()` | No (log/count only) |
| after | After each API call | Resource author in `create()` | No |

These are **resource-level**. Set by the resource author (or overridden by the action author if the Instance exposes `interceptors`). Apply to individual operations, not acquire/release.

### Level 3: Action-level (ActionContext — not in resource crate)

Retry, circuit breaking, rate limiting are primarily applied by the engine/runtime around action execution.

Additionally, `nebula-resource` now uses `nebula-resilience` internally for pool operation protection:

- `create` and `recycle` paths are guarded by circuit breakers to prevent local failure storms.
- breaker-open state surfaces as explicit resource errors/events, preserving observability.

So the effective model is layered:

- pool-level self-protection in `nebula-resource`
- workflow/action-level resilience orchestration in engine/runtime

---

## Part 7: What Changes From Current State

| Area | Current | Target | Breaking? |
|------|---------|--------|-----------|
| `Resource` trait | `impl Future` + `dependencies()` | Same `impl Future`; remove `dependencies()`; remove `key()` (derive from metadata only) | Yes (minor) |
| `ResourceMetadata` | tags only | Keep builder pattern and icon metadata; discovery stays tag-driven | No |
| `Manager` | `acquire` returns `AnyGuard` | + `with_resource` (scoped) | No (additive) |
| `ResourceAccessor` | Does not exist | New type bridging Manager into ActionContext | No (additive) |
| `AuthStrategy` | Does not exist | New enum for common auth patterns | No (additive) |
| `InterceptorChain` | Does not exist | New type for per-call hooks inside Instance | No (additive) |
| `Instance` types | Thin wrappers around raw clients | Rich domain API (get/post/query/send_message) | N/A (per resource) |
| `ResourceProvider` | `impl Future` methods | Clean up `has`/`exists` duality; make `exists` the lightweight one | Yes (minor) |
| SDK facade | No resource exports | Export Resource, Config, Context, etc. | No (additive) |
| `dependencies()` | Exists alongside `Deps` | Deprecated → removed | Yes |
| `key()` | Method on Resource | Removed; `metadata().key` is single source of truth | Yes (minor) |
| Error | No `#[non_exhaustive]` on some variants | All public enums `#[non_exhaustive]` | Yes (minor) |
| Naming in Manager | `pools: DashMap<String, ...>` | `pools: DashMap<ResourceKey, ...>` internally | No (internal) |

---

## Part 8: What We Do NOT Change

- **bb8-style pool model**: acquire → guard → release. Proven, performant.
- **Scope as security boundary**: deny-by-default, no bypass.
- **Health + quarantine**: proactive detection, not reactive.
- **RAII Guard**: drop returns instance. Non-negotiable.
- **EventBus**: broadcast, non-blocking, fire-and-forget.
- **Flat Context**: no generics, same type for all resources.
- **Secrets only through Context**: `ctx.credentials()` or nothing.
- **`impl Future` in Resource trait**: until Rust stabilizes async fn in traits, this is the correct form.

---

## Summary: Why This Is AAAAA+

| Criterion | How we achieve it |
|-----------|-------------------|
| **Simplicity** | Resource author implements 3 things: Config, Instance, Resource. Action author writes one line (`ctx.resources().with::<R>(...)`) |
| **Convenience** | Rich Instance API (get/post/query/send_message), no guard juggling, AuthStrategy resolves creds automatically |
| **Performance** | Pool with Semaphore + VecDeque, zero alloc on hot path, interceptors are stack-local |
| **Safety** | Scope enforcement, RAII guard, credentials never in Config, non_exhaustive enums |
| **Extensibility** | InterceptorChain for per-call hooks, AuthStrategy for auth patterns, Deps for dependency graph |
| **Discoverability** | Stable tag vocabulary and metadata as passport for UI/API/AI |
| **AI-friendliness** | One pattern for all resources, predictable names, machine-readable metadata |
| **Longevity** | `impl Future` (stable Rust), `#[non_exhaustive]`, CONTRACT.md, single facade via SDK |
