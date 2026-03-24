# Resource Prototypes — Architecture Validation

> **Purpose:** Validate nebula-resource architecture against real-world resources.
> Each prototype is a complete skeleton impl: types, traits, registration, action usage.
> Comments mark friction points, design questions, and "this works well" confirmations.

---

## 1. Postgres — Pool topology

The most common resource. Tests: credential separation, prepare() per-tenant,
recycle() with InstanceMetrics, is_broken() sync check, full pool lifecycle.

### Types

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio_postgres::NoTls;
use nebula_resource::*;

// ── Credential (encrypted, rotatable) ─────────────────────────────

/// Framework resolves via CredentialStore before create().
/// Contains connection target + auth. NOT in Config.
pub struct DatabaseCredential {
    pub host:     String,
    pub port:     u16,
    pub database: String,
    pub username: String,
    pub password: SecretString,
    pub ssl_mode: SslMode,
}

impl Credential for DatabaseCredential {
    const KIND: &'static str = "database";
}

// ── Config (plain, operational) ───────────────────────────────────

/// No secrets here. Only operational settings.
/// fingerprint() hashes fields that affect connection compatibility.
#[derive(Debug, Clone)]
pub struct PgResourceConfig {
    pub connect_timeout:   Duration,
    pub statement_timeout: Option<Duration>,
    pub application_name:  String,
    pub search_path:       Option<String>,
    pub recycle_method:    RecycleMethod,
}

#[derive(Debug, Clone, Copy)]
pub enum RecycleMethod {
    /// DISCARD ALL always.
    Full,
    /// DISCARD ALL only if was_in_transaction or had_error.
    Smart,
}

impl Default for PgResourceConfig {
    fn default() -> Self {
        Self {
            connect_timeout:   Duration::from_secs(5),
            statement_timeout: Some(Duration::from_secs(30)),
            application_name:  "nebula".into(),
            search_path:       None,
            recycle_method:    RecycleMethod::Smart,
        }
    }
}

impl ResourceConfig for PgResourceConfig {
    fn validate(&self) -> Result<()> {
        if self.connect_timeout.is_zero() {
            return Err(Error::permanent("connect_timeout must be > 0"));
        }
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        // ✅ WORKS WELL: fingerprint contract is clear.
        // Hash fields that affect existing connection behavior.
        // If statement_timeout changes → old connections are stale.
        let mut h = rustc_hash::FxHasher::default();
        use std::hash::Hash;
        self.statement_timeout.map(|d| d.as_millis()).hash(&mut h);
        self.application_name.hash(&mut h);
        self.search_path.hash(&mut h);
        std::hash::Hasher::finish(&h)
    }
}

// ── Runtime (internal, framework-managed) ─────────────────────────

/// What the pool holds internally. Not exposed to callers directly
/// (although for Pool topology, Lease = Runtime, so callers DO see this type).
pub struct PgConnection {
    pub client:              tokio_postgres::Client,
    conn_task:               AbortOnDrop<JoinHandle<()>>,
    // Sync flags for is_broken() — NO I/O in Drop path.
    had_error:               AtomicBool,
    was_in_transaction:      AtomicBool,
    // Cache to avoid redundant SET search_path per checkout.
    last_search_path:        parking_lot::Mutex<Option<String>>,
}

/// Abort-on-drop guard for connection task (cancel-safety contract #10).
struct AbortOnDrop<T>(T);
impl Drop for AbortOnDrop<JoinHandle<()>> {
    fn drop(&mut self) { self.0.abort(); }
}

// ── Error ─────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, nebula_resource::ClassifyError)]
pub enum PgError {
    #[error("authentication failed for user {user}")]
    #[classify(permanent)]
    Auth { user: String },

    #[error("database {database} does not exist")]
    #[classify(permanent)]
    DatabaseNotFound { database: String },

    #[error("connection failed: {0}")]
    #[classify(transient)]
    Connect(#[from] tokio_postgres::Error),

    #[error("connection closed by server")]
    #[classify(transient)]
    ConnectionClosed,

    #[error("query error: {0}")]
    #[classify(transient)]
    Query(String),

    #[error("health check failed: {0}")]
    #[classify(transient)]
    HealthCheck(String),

    #[error("too many connections")]
    #[classify(exhausted, retry_after = "30s")]
    TooManyConnections,

    #[error("serialization failure — retry")]
    #[classify(transient)]
    SerializationFailure,
}
```

### Resource + Pooled impls

```rust
pub struct Postgres;

impl Resource for Postgres {
    type Config     = PgResourceConfig;
    type Runtime    = PgConnection;
    type Lease      = PgConnection;       // ✅ Pool: Lease = Runtime
    type Error      = PgError;
    type Credential = DatabaseCredential;
    const KEY: ResourceKey = resource_key!("postgres");

    async fn create(
        &self,
        config: &PgResourceConfig,
        cred:   &DatabaseCredential,
        _ctx:   &dyn Ctx,
    ) -> Result<PgConnection, PgError> {
        // ✅ WORKS WELL: credential comes pre-resolved, typed.
        // No CredentialStore lookup here — framework did it.
        let mut pg_config = tokio_postgres::Config::new();
        pg_config
            .host(&cred.host)
            .port(cred.port)
            .dbname(&cred.database)
            .user(&cred.username)
            .password(cred.password.expose())
            .connect_timeout(config.connect_timeout);

        if let Some(timeout) = config.statement_timeout {
            pg_config.options(&format!("-c statement_timeout={}ms", timeout.as_millis()));
        }

        let (client, connection) = pg_config
            .connect(NoTls)
            .await
            .map_err(PgError::Connect)?;

        // ✅ Cancel-safety: AbortOnDrop ensures task is killed if PgConnection drops.
        let conn_task = AbortOnDrop(tokio::spawn(async move {
            if let Err(e) = connection.await {
                tracing::warn!("postgres connection error: {e}");
            }
        }));

        Ok(PgConnection {
            client,
            conn_task,
            had_error:          AtomicBool::new(false),
            was_in_transaction: AtomicBool::new(false),
            last_search_path:   parking_lot::Mutex::new(None),
        })
    }

    async fn check(&self, conn: &PgConnection) -> Result<(), PgError> {
        if conn.conn_task.0.is_finished() {
            return Err(PgError::ConnectionClosed);
        }
        conn.client.simple_query("SELECT 1").await
            .map_err(|e| PgError::HealthCheck(e.to_string()))?;
        Ok(())
    }

    async fn destroy(&self, conn: PgConnection) -> Result<(), PgError> {
        drop(conn.client);
        // conn_task aborted by AbortOnDrop — no need to await.
        // But if we want graceful: timeout + join.
        let _ = tokio::time::timeout(Duration::from_secs(2), conn.conn_task.0).await;
        Ok(())
    }
}

impl Pooled for Postgres {
    fn is_broken(&self, conn: &PgConnection) -> BrokenCheck {
        // ✅ WORKS WELL: sync, O(1), no I/O. All checks are atomic flag reads.
        if conn.client.is_closed() {
            return BrokenCheck::Broken("TCP connection closed".into());
        }
        if conn.conn_task.0.is_finished() {
            return BrokenCheck::Broken("connection task finished".into());
        }
        // Error in transaction → connection state corrupted, must destroy.
        if conn.had_error.load(Ordering::Acquire)
            && conn.was_in_transaction.load(Ordering::Acquire)
        {
            return BrokenCheck::Broken("error in transaction".into());
        }
        BrokenCheck::Healthy
    }

    async fn recycle(
        &self,
        conn: &PgConnection,
        metrics: &InstanceMetrics,
    ) -> Result<RecycleDecision, PgError> {
        // ✅ WORKS WELL: InstanceMetrics gives us error_count, checkout_count, age().
        // Framework already filtered: stale fingerprint → destroy, max_lifetime → destroy.
        // We only do instance-level cleanup.

        if metrics.error_count >= 5 {
            return Ok(RecycleDecision::Drop); // unreliable instance
        }
        if metrics.checkout_count >= 1000 {
            return Ok(RecycleDecision::Drop); // force rotate
        }
        if conn.client.is_closed() {
            return Ok(RecycleDecision::Drop);
        }

        // Smart recycle: DISCARD ALL only if needed.
        let needs_discard = conn.was_in_transaction.load(Ordering::Acquire)
            || conn.had_error.load(Ordering::Acquire);

        if needs_discard {
            // ✅ No unwrap — contract #9: recycle must not panic.
            match conn.client.simple_query("DISCARD ALL").await {
                Ok(_) => {}
                Err(_) => return Ok(RecycleDecision::Drop),
            }
        }

        conn.was_in_transaction.store(false, Ordering::Release);
        conn.had_error.store(false, Ordering::Release);
        Ok(RecycleDecision::Keep)
    }

    async fn prepare(
        &self,
        conn: &PgConnection,
        ctx: &dyn Ctx,
    ) -> Result<(), PgError> {
        // ✅ WORKS WELL: ctx.ext::<T>() for domain-specific injection.
        // Multi-tenant: set search_path per execution.
        if let Some(tenant) = ctx.ext::<TenantContext>() {
            let path = format!("{},public", tenant.schema);
            // Avoid redundant SET if path hasn't changed.
            let current = conn.last_search_path.lock().clone();
            if current.as_deref() != Some(&path) {
                conn.client
                    .simple_query(&format!("SET search_path TO {path}"))
                    .await
                    .map_err(|e| PgError::Query(e.to_string()))?;
                *conn.last_search_path.lock() = Some(path);
            }
            // Optional: set role for row-level security.
            if let Some(ref role) = tenant.role {
                conn.client
                    .simple_query(&format!("SET ROLE {role}"))
                    .await
                    .map_err(|e| PgError::Query(e.to_string()))?;
            }
        }
        Ok(())
    }
}

// Domain-specific extension data, injected by engine.
pub struct TenantContext {
    pub schema: String,
    pub role:   Option<String>,
}
```

### Registration & Usage

```rust
// ── Registration ──────────────────────────────────────────────────

manager.register(Postgres)
    .config(PgResourceConfig {
        connect_timeout: Duration::from_secs(5),
        statement_timeout: Some(Duration::from_secs(30)),
        application_name: "nebula-worker-1".into(),
        ..Default::default()
    })
    .id(ResourceId::new("main-postgres"))
    .scope(ScopeLevel::Organization(org_id))
    .recovery_group(RecoveryGroupKey::new("pg-primary"))
    .acquire_resilience(AcquireResilience {
        timeout: Some(Duration::from_secs(5)),
        retry: Some(AcquireRetryConfig {
            max_attempts: 3,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(2),
            backoff: BackoffKind::Exponential,
        }),
        circuit_breaker: Some(AcquireCircuitBreakerPreset::Standard),
    })
    .pool(pool::Config {
        min_size: 2,
        max_size: 20,
        strategy: pool::Strategy::Lifo,
        warmup: pool::WarmupStrategy::Staggered {
            delay: Duration::from_millis(200),
        },
        idle_timeout: Duration::from_secs(300),
        max_lifetime: Duration::from_secs(3600),
        test_on_checkout: true,
        recycle_workers: 1,  // Postgres recycle is ~1ms
        max_acquire_attempts: 3,
        ..Default::default()
    })
    .build().await?;

// ── Action usage ──────────────────────────────────────────────────

async fn execute(&self, input: QueryInput, ctx: &ActionContext) -> Result<ActionResult<Vec<Row>>> {
    let db = ctx.resource::<Postgres>().await?;
    // db: ResourceHandle<Postgres> — Deref to PgConnection
    // prepare() already called: search_path set for tenant
    let rows = db.client.query(&input.sql, &[]).await
        .map_err(|e| ActionError::resource(PgError::Query(e.to_string())))?;
    Ok(ActionResult::new(rows))
    // drop(db) → pool checkin → recycle (DISCARD ALL if needed)
}
```

### Validation notes

- ✅ **Config/Credential separation** works naturally. Host/port/password in credential, timeouts in config.
- ✅ **fingerprint()** on config makes sense: statement_timeout change → connections stale.
- ✅ **prepare()** with ctx.ext::<TenantContext>() is clean per-tenant isolation.
- ✅ **recycle()** with InstanceMetrics gives enough info for intelligent recycling.
- ✅ **is_broken()** sync check covers all common failure modes without I/O.
- ✅ **AbortOnDrop** pattern for cancel-safety of spawned connection task.
- ✅ **ClassifyError** macro maps domain errors to framework retry decisions naturally.
- ⚠️ **Friction:** `last_search_path` cache in PgConnection requires `parking_lot::Mutex`
  for sync access in prepare(). This is fine (prepare is async, lock is held briefly),
  but alternatives: store as AtomicCell<Option<CompactString>> or just always SET.

---

## 2. Google Sheets — Resident topology

Tests: OAuth credential with token refresh, stateless HTTP client, Resident with
Clone semantics, no health check needed, simple config.

### Types

```rust
use reqwest::Client;

// ── Credential ────────────────────────────────────────────────────

/// OAuth2 service account or user token.
/// Framework handles token refresh via credential rotation events.
pub struct GoogleCredential {
    /// Service account JSON key or OAuth refresh token.
    pub auth: GoogleAuth,
    /// Target spreadsheet permissions scope.
    pub scopes: Vec<String>,
}

pub enum GoogleAuth {
    ServiceAccount { key_json: SecretString },
    OAuth { refresh_token: SecretString, client_id: String, client_secret: SecretString },
}

impl Credential for GoogleCredential {
    const KIND: &'static str = "google_oauth";
}

// ── Config ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct GoogleSheetsConfig {
    pub timeout:             Duration,
    pub max_retries:         usize,
    pub rate_limit_per_min:  Option<u32>,  // Google API quota: 60 req/min default
}

impl Default for GoogleSheetsConfig {
    fn default() -> Self {
        Self {
            timeout:            Duration::from_secs(30),
            max_retries:        3,
            rate_limit_per_min: Some(60),
        }
    }
}

impl ResourceConfig for GoogleSheetsConfig {
    fn validate(&self) -> Result<()> {
        if self.timeout.is_zero() {
            return Err(Error::permanent("timeout must be > 0"));
        }
        Ok(())
    }

    // ✅ fingerprint() = 0 is correct here.
    // Config changes (timeout, rate_limit) don't make existing client incompatible.
    // Resident topology: config change → destroy old + create new.
    // No stale detection needed — there's only one instance.
    fn fingerprint(&self) -> u64 { 0 }
}

// ── Runtime = Lease (Resident: Clone) ─────────────────────────────

/// Shared HTTP client with pre-configured auth headers.
/// Clone = Arc increment (cheap).
#[derive(Clone)]
pub struct GoogleSheetsClient {
    inner:   Client,
    token:   Arc<tokio::sync::RwLock<String>>,  // cached access token
    config:  GoogleSheetsConfig,
    // NOTE: rate limiter could live here if using nebula-resilience::RateLimiter.
}

// ── Error ─────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, ClassifyError)]
pub enum GoogleSheetsError {
    #[error("authentication failed: {0}")]
    #[classify(permanent)]
    AuthFailed(String),

    #[error("spreadsheet not found: {id}")]
    #[classify(permanent)]
    SpreadsheetNotFound { id: String },

    #[error("permission denied for spreadsheet {id}")]
    #[classify(permanent)]
    PermissionDenied { id: String },

    #[error("rate limited by Google API")]
    #[classify(exhausted, retry_after = "60s")]
    RateLimited,

    #[error("network error: {0}")]
    #[classify(transient)]
    Network(#[from] reqwest::Error),

    #[error("API error {status}: {message}")]
    #[classify(transient)]
    ApiError { status: u16, message: String },

    #[error("invalid response: {0}")]
    #[classify(permanent)]
    InvalidResponse(String),
}
```

### Resource + Resident impls

```rust
pub struct GoogleSheets;

impl Resource for GoogleSheets {
    type Config     = GoogleSheetsConfig;
    type Runtime    = GoogleSheetsClient;
    type Lease      = GoogleSheetsClient;    // ✅ = Runtime (Resident, Clone)
    type Error      = GoogleSheetsError;
    type Credential = GoogleCredential;
    const KEY: ResourceKey = resource_key!("google.sheets");

    async fn create(
        &self,
        config: &GoogleSheetsConfig,
        cred:   &GoogleCredential,
        _ctx:   &dyn Ctx,
    ) -> Result<GoogleSheetsClient, GoogleSheetsError> {
        // ✅ WORKS WELL: credential pre-resolved, typed.
        // Build HTTP client with auth.
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(GoogleSheetsError::Network)?;

        // Exchange credential for initial access token.
        let access_token = match &cred.auth {
            GoogleAuth::ServiceAccount { key_json } => {
                // Use google-cloud-auth or similar to get token from service account.
                todo!("exchange service account key for access token")
            }
            GoogleAuth::OAuth { refresh_token, client_id, client_secret } => {
                // OAuth refresh flow.
                todo!("exchange refresh token for access token")
            }
        };

        Ok(GoogleSheetsClient {
            inner:  client,
            token:  Arc::new(tokio::sync::RwLock::new(access_token)),
            config: config.clone(),
        })
    }

    // ✅ No check() override — HTTP client is stateless.
    // Token refresh handled externally via credential rotation.
    // If token expires mid-use → API returns 401 → action error → re-acquire
    // (which triggers credential resolve → fresh token).

    // ✅ No destroy() override — Client drops cleanly.
}

impl Resident for GoogleSheets {
    // ✅ All defaults. HTTP-based, stateless, no periodic health check.
    // is_alive_sync: default true.
    // stale_after: default None.
    //
    // QUESTION: should we add stale_after for token expiry monitoring?
    // ANSWER: No. Token refresh is credential rotation concern, not resource health.
    // Framework resolves fresh credential on next create(). Resident config change
    // or credential rotation triggers destroy + create with fresh token.
}
```

### Registration & Usage

```rust
// ── Registration ──────────────────────────────────────────────────

manager.register(GoogleSheets)
    .config(GoogleSheetsConfig::default())
    .id(ResourceId::new("google-sheets"))
    .scope(ScopeLevel::Project(project_id))
    .resident(resident::Config { eager_create: true })
    .build().await?;

// ── Action: read spreadsheet ──────────────────────────────────────

async fn execute(&self, input: ReadSheetInput, ctx: &ActionContext) -> Result<ActionResult<SheetData>> {
    let sheets = ctx.resource::<GoogleSheets>().await?;
    // sheets: ResourceHandle<GoogleSheets> — Deref to GoogleSheetsClient

    let token = sheets.token.read().await;
    let resp = sheets.inner
        .get(&format!(
            "https://sheets.googleapis.com/v4/spreadsheets/{}/values/{}",
            input.spreadsheet_id, input.range
        ))
        .bearer_auth(&*token)
        .send()
        .await
        .map_err(GoogleSheetsError::Network)?;

    match resp.status().as_u16() {
        200 => {
            let data: SheetData = resp.json().await.map_err(GoogleSheetsError::Network)?;
            Ok(ActionResult::new(data))
        }
        401 => Err(GoogleSheetsError::AuthFailed("token expired".into()).into()),
        403 => Err(GoogleSheetsError::PermissionDenied { id: input.spreadsheet_id }.into()),
        404 => Err(GoogleSheetsError::SpreadsheetNotFound { id: input.spreadsheet_id }.into()),
        429 => Err(GoogleSheetsError::RateLimited.into()),
        s   => Err(GoogleSheetsError::ApiError {
            status: s,
            message: resp.text().await.unwrap_or_default(),
        }.into()),
    }
    // drop(sheets) → noop (Owned handle, Resident clone)
}
```

### Validation notes

- ✅ **Resident** is the right topology. Clone = Arc increment. Stateless HTTP.
- ✅ **fingerprint() = 0** correct — Resident has only one instance, config change = recreate.
- ✅ **Credential separation** works well for OAuth: refresh_token in credential, timeout in config.
- ✅ **Token refresh** maps to credential rotation flow naturally.
- ⚠️ **Friction: token caching inside Runtime.** GoogleSheetsClient holds `Arc<RwLock<String>>`
  for the access token. This is internal to the resource — framework doesn't know about it.
  On credential rotation → framework destroys + creates new → fresh token. But what about
  mid-lifetime token expiry? Two options:
  - (a) Resource internally refreshes (background task in create()) — but then we need
    the refresh_token inside Runtime, not just at create time. This conflicts with
    "credential only at create".
  - (b) Short-lived access token in credential, framework re-resolves frequently.
  - **Decision for v1:** Use (b) — CredentialStore resolves fresh access token each time.
    create() just uses the token directly. No internal refresh logic.
    This means Resident with short stale_after (e.g., 45 min for 1-hour token).

- ⚠️ **Rate limiting:** Google API has per-user-per-project quotas. Rate limiter should
  live in the resource (like LlmRuntime example in 04-recovery-resilience.md).
  GoogleSheetsClient can hold `Arc<GovernorRateLimiter>` internally. Works fine.

---

## 3. Telegram Bot — Service + EventSource + Daemon (hybrid)

The most complex resource. Tests: 3 topology traits on one struct, Runtime ≠ Lease,
token-based access, incoming events, background polling loop, credential for bot token.

### Types

```rust
use std::sync::Arc;
use teloxide::prelude::*;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

// ── Credential ────────────────────────────────────────────────────

pub struct TelegramCredential {
    pub token: SecretString,
}

impl Credential for TelegramCredential {
    const KIND: &'static str = "telegram_bot";
}

// ── Config ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TelegramResourceConfig {
    pub buffer_size:     usize,   // broadcast channel capacity
    pub polling_timeout: Duration, // long polling timeout
    pub allowed_updates: Vec<String>,
}

impl Default for TelegramResourceConfig {
    fn default() -> Self {
        Self {
            buffer_size:     256,
            polling_timeout: Duration::from_secs(30),
            allowed_updates: vec!["message".into(), "callback_query".into()],
        }
    }
}

impl ResourceConfig for TelegramResourceConfig {
    fn fingerprint(&self) -> u64 { 0 } // Service: config change = create new, drain old
}

// ── Runtime (internal, framework-managed) ─────────────────────────

/// Infrastructure: bot client + broadcast channel for updates.
/// NOT Clone. NOT exposed to callers.
pub struct TelegramBotRuntime {
    inner: Arc<BotInner>,
}

struct BotInner {
    bot:       Bot,
    info:      BotInfo,
    update_tx: broadcast::Sender<TelegramUpdate>,
}

// ── Lease (caller-facing, lightweight token) ──────────────────────

/// What callers see via Deref. Cheap clone (all Arc).
/// Can send messages AND receive updates.
pub struct TelegramBotHandle {
    bot:       Bot,                                  // Clone = Arc
    info:      Arc<BotInfo>,
    update_rx: broadcast::Receiver<TelegramUpdate>,  // per-caller subscriber
}

impl TelegramBotHandle {
    pub async fn send_message(&self, chat_id: i64, text: &str) -> Result<(), TelegramError> {
        self.bot.send_message(ChatId(chat_id), text)
            .await
            .map_err(TelegramError::Api)?;
        Ok(())
    }

    pub async fn recv_update(&mut self) -> Result<TelegramUpdate, TelegramError> {
        self.update_rx.recv().await.map_err(|_| TelegramError::ChannelClosed)
    }

    pub fn bot_info(&self) -> &BotInfo { &self.info }
}

// ── Events ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TelegramUpdate {
    pub update_id: i64,
    pub chat_id:   Option<i64>,
    pub kind:      TelegramUpdateKind,
}

#[derive(Debug, Clone)]
pub enum TelegramUpdateKind {
    Message { text: Option<String>, message_id: i64 },
    CallbackQuery { data: String, message_id: i64 },
    Other,
}

// ── Error ─────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, ClassifyError)]
pub enum TelegramError {
    #[error("API error: {0}")]
    #[classify(transient)]
    Api(#[from] teloxide::RequestError),

    #[error("bot token invalid or revoked")]
    #[classify(permanent)]
    InvalidToken,

    #[error("broadcast channel closed")]
    #[classify(transient)]
    ChannelClosed,

    #[error("bot blocked by user {chat_id}")]
    #[classify(transient, scope = target, field = "chat_id")]
    BotBlocked { chat_id: i64 },

    #[error("rate limited by Telegram API")]
    #[classify(exhausted, retry_after = "30s")]
    RateLimited,

    #[error("polling error: {0}")]
    #[classify(transient)]
    Polling(String),
}
```

### Resource + Service + EventSource + Daemon impls

```rust
pub struct TelegramBot;

impl Resource for TelegramBot {
    type Config     = TelegramResourceConfig;
    type Runtime    = TelegramBotRuntime;
    type Lease      = TelegramBotHandle;      // ✅ ≠ Runtime (Service topology)
    type Error      = TelegramError;
    type Credential = TelegramCredential;
    const KEY: ResourceKey = resource_key!("telegram.bot");

    async fn create(
        &self,
        config: &TelegramResourceConfig,
        cred:   &TelegramCredential,
        _ctx:   &dyn Ctx,
    ) -> Result<TelegramBotRuntime, TelegramError> {
        // ✅ Setup infrastructure ONLY. DO NOT start polling loop here.
        // Polling = Daemon::run(), started by framework separately.
        let bot = Bot::new(cred.token.expose());
        let info = bot.get_me().await.map_err(TelegramError::Api)?;
        let (update_tx, _) = broadcast::channel(config.buffer_size);

        Ok(TelegramBotRuntime {
            inner: Arc::new(BotInner { bot, info, update_tx }),
        })
    }

    async fn check(&self, runtime: &TelegramBotRuntime) -> Result<(), TelegramError> {
        runtime.inner.bot.get_me().await.map_err(TelegramError::Api)?;
        Ok(())
    }

    async fn destroy(&self, runtime: TelegramBotRuntime) -> Result<(), TelegramError> {
        // Arc<BotInner> dropped. Existing tokens (TelegramBotHandle) continue
        // working until they are dropped — Arc keeps BotInner alive.
        // Framework cancels Daemon separately via CancellationToken.
        drop(runtime);
        Ok(())
    }
}

impl Service for TelegramBot {
    const TOKEN_MODE: TokenMode = TokenMode::Cloned;

    async fn acquire_token(
        &self,
        runtime: &TelegramBotRuntime,
        _ctx: &dyn Ctx,
    ) -> Result<TelegramBotHandle, TelegramError> {
        // ✅ WORKS WELL: cheap token creation. Bot clone = Arc.
        // Each caller gets own broadcast::Receiver (independent read position).
        Ok(TelegramBotHandle {
            bot:       runtime.inner.bot.clone(),
            info:      Arc::clone(&runtime.inner.info),
            update_rx: runtime.inner.update_tx.subscribe(),
        })
    }

    // release_token: default noop. Cloned mode — no tracking needed.
}

impl EventSource for TelegramBot {
    type Event = TelegramUpdate;
    type Subscription = broadcast::Receiver<TelegramUpdate>;

    async fn subscribe(
        &self,
        runtime: &TelegramBotRuntime,
        _ctx: &dyn Ctx,
    ) -> Result<Self::Subscription, TelegramError> {
        Ok(runtime.inner.update_tx.subscribe())
    }

    async fn recv(
        &self,
        subscription: &mut Self::Subscription,
    ) -> Result<TelegramUpdate, TelegramError> {
        subscription.recv().await.map_err(|_| TelegramError::ChannelClosed)
    }
}

impl Daemon for TelegramBot {
    async fn run(
        &self,
        runtime: &TelegramBotRuntime,
        _ctx: &dyn Ctx,
        cancel: CancellationToken,
    ) -> Result<(), TelegramError> {
        // ✅ Contract #6: MUST respect CancellationToken.
        let mut offset = 0i64;

        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Ok(()),
                result = runtime.inner.bot.get_updates()
                    .offset(offset)
                    .timeout(30) // Telegram long polling
                    .send() =>
                {
                    let updates = result.map_err(|e| TelegramError::Polling(e.to_string()))?;
                    for update in updates {
                        offset = update.id + 1;
                        let tg_update = convert_update(update);
                        // Best-effort broadcast. Lagged receivers drop old messages.
                        let _ = runtime.inner.update_tx.send(tg_update);
                    }
                }
            }
        }
    }
}

fn convert_update(update: teloxide::types::Update) -> TelegramUpdate {
    todo!("convert teloxide Update to our TelegramUpdate type")
}
```

### Registration & Usage

```rust
// ── Registration (hybrid: 3 topologies) ───────────────────────────

manager.register(TelegramBot)
    .config(TelegramResourceConfig::default())
    .id(ResourceId::new("main-bot"))
    .service(service::Config::default())                    // primary
    .also_event_source(event_source::Config::default())     // secondary
    .also_daemon(daemon::Config {
        restart_policy: RestartPolicy::OnFailure,
        max_restarts: 5,
        restart_backoff: BackoffConfig {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(60),
            multiplier: 2.0,
        },
        ..Default::default()
    })                                                       // secondary
    .build().await?;

// ── Action: send message ──────────────────────────────────────────

async fn execute(&self, input: SendMsgInput, ctx: &ActionContext) -> Result<ActionResult<()>> {
    let bot = ctx.resource::<TelegramBot>().await?;
    // bot: ResourceHandle<TelegramBot> — Deref to TelegramBotHandle
    bot.send_message(input.chat_id, &input.text).await?;
    Ok(ActionResult::new(()))
    // drop(bot) → noop (Owned handle, Cloned token)
}

// ── Trigger: incoming messages ────────────────────────────────────

struct IncomingMessageTrigger;

impl EventTrigger for IncomingMessageTrigger {
    type Source = TelegramBot;
    type Event  = IncomingMessage;

    async fn on_event(
        &self,
        bot: &TelegramBotHandle,
        _ctx: &TriggerContext,
    ) -> Result<Option<IncomingMessage>> {
        // ✅ WORKS WELL: bot is &R::Lease, not &R::Runtime.
        // Trigger author sees TelegramBotHandle API, not internal BotRuntime.
        let update = bot.recv_update().await?;
        match update.kind {
            TelegramUpdateKind::Message { text: Some(text), message_id } => {
                Ok(Some(IncomingMessage {
                    chat_id: update.chat_id.unwrap_or(0),
                    text,
                    message_id,
                }))
            }
            _ => Ok(None),
        }
    }
}
```

### Validation notes

- ✅ **Hybrid registration** works naturally. `.service().also_event_source().also_daemon()`.
- ✅ **Runtime ≠ Lease** separation clean. Runtime = infrastructure (bot + broadcast).
  Lease = caller handle (bot clone + receiver).
- ✅ **Daemon::run()** correctly separated from create(). Infrastructure setup in create(),
  polling loop in run(). Framework manages restart.
- ✅ **EventSource** shares broadcast channel with Daemon. Daemon writes, EventSource reads.
- ✅ **BotBlocked** error with `scope = target` — correct: one user blocking doesn't
  affect the bot resource itself.
- ✅ **Service drain on config reload:** old TelegramBotRuntime drains via Arc refcount.
  Existing TelegramBotHandle tokens (which hold Arc<BotInner>) continue working.
  New tokens are created from new runtime.
- ⚠️ **Friction: recv_update() on TelegramBotHandle requires &mut self** for
  broadcast::Receiver. But ResourceHandle gives &self via Deref. This means either:
  - (a) Handle holds `Mutex<broadcast::Receiver>` — works but slightly ugly.
  - (b) Lease has interior mutability for the receiver.
  - (c) EventTrigger receives `&mut Self::Subscription` not `&R::Lease`.
  - **Resolution:** EventTrigger already receives `&mut Self::Subscription` in the
    EventSource trait. For actions using recv_update(), use interior mutability.
    This is a known pattern for broadcast receivers.

---

## 4. SSH — Transport topology

Tests: expensive connection + cheap sessions, keepalive, close_session with
healthy flag, credential with private key, max_sessions semaphore.

### Types

```rust
use openssh::{Session as OsshSession, Child};
use std::time::Instant;

// ── Credential ────────────────────────────────────────────────────

pub struct SshKeyCredential {
    pub host:        String,
    pub port:        u16,
    pub username:    String,
    pub private_key: SecretString,
    pub passphrase:  Option<SecretString>,
}

impl Credential for SshKeyCredential {
    const KIND: &'static str = "ssh_key";
}

// ── Config ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SshResourceConfig {
    pub connect_timeout:    Duration,
    pub keepalive_interval: Duration,
    pub max_sessions:       usize,    // bounded via semaphore (contract #5)
    pub session_timeout:    Option<Duration>,
}

impl Default for SshResourceConfig {
    fn default() -> Self {
        Self {
            connect_timeout:    Duration::from_secs(10),
            keepalive_interval: Duration::from_secs(30),
            max_sessions:       10,
            session_timeout:    None,
        }
    }
}

impl ResourceConfig for SshResourceConfig {
    fn validate(&self) -> Result<()> {
        if self.max_sessions == 0 {
            return Err(Error::permanent("max_sessions must be > 0"));
        }
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        // ✅ max_sessions change doesn't make existing connection incompatible.
        // But keepalive_interval change does affect the connection.
        let mut h = rustc_hash::FxHasher::default();
        use std::hash::Hash;
        self.keepalive_interval.as_millis().hash(&mut h);
        std::hash::Hasher::finish(&h)
    }
}

// ── Runtime (one TCP connection) ──────────────────────────────────

pub struct SshRuntime {
    session: OsshSession,
}

// ── Lease (multiplexed session) ──────────────────────────────────

pub struct SshSession {
    child:     Child<OsshSession>,
    opened_at: Instant,
}

impl SshSession {
    pub async fn exec(&mut self, command: &str) -> Result<String, SshError> {
        todo!("execute command on SSH session, return stdout")
    }

    pub async fn upload(&mut self, local: &Path, remote: &Path) -> Result<(), SshError> {
        todo!("SCP upload")
    }
}

// ── Error ─────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, ClassifyError)]
pub enum SshError {
    #[error("connection failed: {0}")]
    #[classify(transient)]
    Connect(String),

    #[error("authentication failed for user {user}")]
    #[classify(permanent)]
    Auth { user: String },

    #[error("session open failed: {0}")]
    #[classify(transient)]
    SessionOpen(String),

    #[error("keepalive failed")]
    #[classify(transient)]
    Keepalive,

    #[error("command execution failed: {0}")]
    #[classify(transient)]
    Exec(String),

    #[error("connection closed by remote")]
    #[classify(transient)]
    ConnectionClosed,

    #[error("session limit reached ({max})")]
    #[classify(backpressure)]
    SessionLimitReached { max: usize },
}
```

### Resource + Transport impls

```rust
pub struct Ssh;

impl Resource for Ssh {
    type Config     = SshResourceConfig;
    type Runtime    = SshRuntime;
    type Lease      = SshSession;          // ✅ ≠ Runtime (Transport topology)
    type Error      = SshError;
    type Credential = SshKeyCredential;
    const KEY: ResourceKey = resource_key!("ssh");

    async fn create(
        &self,
        config: &SshResourceConfig,
        cred:   &SshKeyCredential,
        _ctx:   &dyn Ctx,
    ) -> Result<SshRuntime, SshError> {
        // ✅ Expensive operation: TCP + key exchange + auth.
        // Done once. Sessions are cheap on top.
        let session = openssh::SessionBuilder::default()
            .known_hosts_check(openssh::KnownHosts::Accept) // TODO: configurable
            .connect_timeout(config.connect_timeout)
            .user(cred.username.clone())
            .port(cred.port)
            .keyfile(write_temp_key(&cred.private_key)?) // temp file with private key
            .connect(&cred.host)
            .await
            .map_err(|e| SshError::Connect(e.to_string()))?;

        Ok(SshRuntime { session })
    }

    async fn check(&self, runtime: &SshRuntime) -> Result<(), SshError> {
        // Check SSH connection is still alive.
        runtime.session.check().await.map_err(|_| SshError::ConnectionClosed)?;
        Ok(())
    }

    async fn destroy(&self, runtime: SshRuntime) -> Result<(), SshError> {
        runtime.session.close().await.map_err(|e| SshError::Connect(e.to_string()))?;
        Ok(())
    }
}

impl Transport for Ssh {
    async fn open_session(
        &self,
        transport: &SshRuntime,
        _ctx: &dyn Ctx,
    ) -> Result<SshSession, SshError> {
        // ✅ Cheap: new channel on existing TCP connection.
        // Framework manages max_sessions via Semaphore (contract #5).
        let child = transport.session
            .command("bash")
            .spawn()
            .await
            .map_err(|e| SshError::SessionOpen(e.to_string()))?;

        Ok(SshSession {
            child,
            opened_at: Instant::now(),
        })
    }

    async fn close_session(
        &self,
        _transport: &SshRuntime,
        session: SshSession,
        healthy: bool,
    ) -> Result<(), SshError> {
        // ✅ healthy flag: if session errored, we may want to log differently.
        if !healthy {
            tracing::debug!("SSH session closed after error, held for {:?}", session.opened_at.elapsed());
        }
        // Child process killed on drop. Explicit close for clean shutdown.
        drop(session.child);
        Ok(())
    }

    async fn keepalive(&self, transport: &SshRuntime) -> Result<(), SshError> {
        // ✅ Periodic probe on TRANSPORT level (not session).
        // Prevents server from closing idle connection (sshd ClientAliveInterval).
        transport.session.check().await.map_err(|_| SshError::Keepalive)?;
        Ok(())
    }
}

fn write_temp_key(key: &SecretString) -> Result<PathBuf, SshError> {
    todo!("write private key to temp file with restricted permissions")
}
```

### Registration & Usage

```rust
// ── Registration ──────────────────────────────────────────────────

manager.register(Ssh)
    .config(SshResourceConfig {
        connect_timeout: Duration::from_secs(10),
        keepalive_interval: Duration::from_secs(30),
        max_sessions: 5,
        session_timeout: Some(Duration::from_secs(300)),
    })
    .id(ResourceId::new("build-server-ssh"))
    .transport(transport::Config {
        max_sessions: 5,     // ✅ Framework creates Semaphore(5)
        keepalive_interval: Some(Duration::from_secs(30)),
        ..Default::default()
    })
    .build().await?;

// ── Action: run command ───────────────────────────────────────────

async fn execute(&self, input: RunCommandInput, ctx: &ActionContext) -> Result<ActionResult<CommandOutput>> {
    let ssh = ctx.resource::<Ssh>().await?;
    // ssh: ResourceHandle<Ssh> — Deref to SshSession
    // Framework already: open_session() called, semaphore permit acquired.
    let output = ssh.exec(&input.command).await?;
    Ok(ActionResult::new(CommandOutput { stdout: output }))
    // drop(ssh) → close_session() via ReleaseQueue
}

// ── Action: detach for long tunnel ────────────────────────────────

async fn execute(&self, input: TunnelInput, ctx: &ActionContext) -> Result<ActionResult<TunnelHandle>> {
    let ssh = ctx.resource::<Ssh>().await?;
    // ✅ Detach: caller takes ownership. Pool no longer tracks.
    let session = ssh.detach()?;
    // session: SshSession — caller's responsibility to close.
    // Semaphore permit released by detach (framework handles).
    Ok(ActionResult::new(TunnelHandle { session }))
}
```

### Validation notes

- ✅ **Transport** topology fits SSH perfectly. One expensive TCP connection, many cheap sessions.
- ✅ **max_sessions** via config → framework Semaphore. Resource author doesn't implement Semaphore.
- ✅ **keepalive()** on transport level (not session) prevents server disconnect.
- ✅ **close_session(healthy)** flag useful for diagnostics.
- ✅ **detach()** for long-running tunnels — correct use case, semaphore permit released.
- ✅ **Credential:** private key in SecretString, written to temp file for openssh.
- ⚠️ **Friction: temp key file.** SSH libraries typically need a file path for private keys.
  `write_temp_key()` creates a temp file — must be cleaned up. Two options:
  - (a) Cleanup in destroy() — but create() is where we write, and cancel-safety matters.
  - (b) Use openssh's `raw_command` with key passed via stdin/env.
  - **Decision:** Use temp file with restrictive permissions (0600). Cleanup in destroy().
    If create() cancelled, temp file leaks — acceptable (OS cleans /tmp eventually).
    Better approach for v2: SSH agent forwarding.

- ⚠️ **Friction: max_sessions in two places.** `SshResourceConfig.max_sessions` and
  `transport::Config.max_sessions`. Which is the source of truth?
  - **Resolution:** `transport::Config.max_sessions` is the framework config (controls
    Semaphore). `SshResourceConfig.max_sessions` should NOT exist — remove it.
    Resource config is for resource-specific settings, session limit is topology config.

---

## Cross-cutting validation summary

### What works well across all 4 prototypes

| Aspect | Verdict |
|--------|---------|
| Config / Credential separation | ✅ Clean in all cases. Secrets in credential, operational in config. |
| `create(config, credential, ctx)` signature | ✅ Natural for all resources. Credential pre-resolved. |
| `ClassifyError` derive macro | ✅ Per-variant mapping covers all domain error patterns. |
| `ctx.resource::<R>().await?` DX | ✅ One line, topology invisible. Works for Pool, Resident, Service, Transport. |
| RAII cleanup via Drop | ✅ Never leak. Pool recycles, Transport closes, Service noops. |
| `InstanceMetrics` in recycle | ✅ error_count, checkout_count, age() useful for intelligent recycling. |
| `ctx.ext::<T>()` for domain injection | ✅ TenantContext in prepare() is clean. |
| Typestate builder | ✅ Compile-time prevents wrong topology for wrong resource. |
| Hybrid registration | ✅ `.service().also_event_source().also_daemon()` natural for Telegram. |
| ResourceHandle Deref | ✅ Caller works with R::Lease API directly. No wrapper noise. |

### Friction points identified

| # | Issue | Affected | Severity | Resolution |
|---|-------|----------|----------|------------|
| 1 | Token caching for OAuth-based APIs | Google Sheets | Medium | Use short-lived tokens from CredentialStore, Resident stale_after for token TTL |
| 2 | broadcast::Receiver needs &mut self | Telegram | Low | Interior mutability or use EventSource's &mut Subscription path |
| 3 | SSH temp key file lifecycle | SSH | Low | Temp file in create(), cleanup in destroy(), OS fallback for cancellation |
| 4 | max_sessions in two configs | SSH | Low | Remove from resource config, keep only in transport::Config |
| 5 | parking_lot::Mutex for prepare() cache | Postgres | Low | Acceptable pattern, or use AtomicCell/always-SET alternative |

### Architecture confirmations

1. **7 topologies justified.** Each prototype naturally falls into exactly one primary topology.
   No resource feels "forced" into a wrong pattern.
2. **Lease ≠ Runtime separation** essential for Service (Telegram) and Transport (SSH).
   Pool and Resident don't need it (Lease = Runtime), but the option is there.
3. **Credential rotation** maps naturally to all resources. Postgres: pool stale → evict.
   Google Sheets: Resident recreate with fresh token. Telegram: destroy + create.
4. **fingerprint() contract** clear: Postgres hashes operational fields, Google Sheets
   returns 0 (Resident recreates anyway), SSH hashes keepalive interval.
5. **Error classification** covers all patterns: permanent auth errors, transient network,
   exhausted rate limits, target-scoped bot blocks, backpressure session limits.
