# 01 — Core: Resource, Ctx, Error

---

## Resource trait

Центральный trait. Описывает lifecycle одного runtime instance — connection, client, process, session.

```rust
pub trait Resource: Send + Sync + 'static {
    /// Конфигурация ресурса. Operational settings: timeouts, pool size, application name.
    /// НЕ содержит secrets — secrets приходят через ctx.credential().
    type Config: ResourceConfig;

    /// Internal runtime instance. Managed by framework.
    /// Для Postgres: PgConnection (wrapper над tokio_postgres::Client).
    /// Для HTTP: reqwest::Client.
    /// Для Telegram: TelegramBotRuntime (infrastructure: bot client + broadcast channel).
    type Runtime: Send + Sync + 'static;

    /// Caller-facing handle. ResourceHandle<R> Deref target.
    /// For most topologies: = Runtime (Pool, Resident, Exclusive).
    /// Service: = Token (TelegramBotHandle — send_message, get_me).
    /// Transport: = Session (SshSession — spawned child process).
    ///
    /// Stable Rust: explicit `type Lease = ...` in each impl.
    /// When associated type defaults stabilize: `= Self::Runtime` default in trait.
    type Lease: Send + Sync + 'static;

    /// Typed error. Каждый resource определяет свой enum ошибок.
    /// Must impl Into<nebula_resource::Error> для классификации transient/permanent/exhausted.
    type Error: std::error::Error + Send + Sync + Into<crate::Error> + 'static;

    /// Уникальный ключ типа ресурса. Compile-time validated via `resource_key!()` macro.
    /// Uses ResourceKey from nebula-core (= DomainKey<ResourceDomain>).
    /// `resource_key!("postgres")` — validates key format at compile time (lowercase ASCII,
    /// separators `.`/`_`/`-`, no leading/trailing separators).
    /// Примеры: "postgres", "redis.shared", "telegram.bot", "ssh", "http.client".
    /// Используется для Registry lookup, metrics labels, UI.
    const KEY: ResourceKey;  // = resource_key!("postgres") in impl

    /// Credential тип этого resource. `()` для ресурсов без secrets.
    ///
    /// Framework резолвит через CredentialStore ПЕРЕД вызовом create().
    /// Resource author декларирует тип — framework берёт resolver на себя.
    ///
    /// NOTE: associated type defaults не стабильны в Rust. Каждый impl указывает явно:
    ///   `type Credential = ();`           — HTTP client, etc.
    ///   `type Credential = DatabaseCred;` — Postgres, MySQL, etc.
    type Credential: Credential;

    /// Создать один runtime instance.
    ///
    /// Вызывается:
    ///   Pool: при нехватке idle instances (до max_size).
    ///   Resident: один раз при первом acquire (или eager при register).
    ///   Service/Transport: один раз — создаёт "infrastructure" runtime.
    ///   Exclusive: один раз при register.
    ///
    /// `credential` — уже резолвленный framework-ом через CredentialStore.
    /// `config` содержит только operational settings (без secrets).
    fn create(
        &self,
        config:     &Self::Config,
        credential: &Self::Credential,
        ctx:        &dyn Ctx,
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send;

    /// Graceful shutdown hint. Вызывается ПЕРЕД destroy.
    ///
    /// Для Resident (Clone-based): clone-ы у callers ещё живы.
    /// shutdown() = "подготовься к завершению" (flush buffers, stop background tasks).
    /// destroy() вызывается позже, когда clone-ы drop-нулись.
    ///
    /// Для Pool: вызывается на каждом instance перед destroy.
    ///
    /// Default: noop.
    fn shutdown(
        &self,
        _runtime: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }

    /// Final destroy. Framework гарантирует: это единственный owner.
    ///
    /// Для Postgres: drop client, ждать завершения connection task.
    /// Для SSH: close session, kill child processes.
    /// Для Browser: close page, kill process.
    ///
    /// Default: noop (drop делает cleanup через Rust Drop trait).
    fn destroy(
        &self,
        runtime: Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = runtime;
        async { Ok(()) }
    }

    /// Liveness check. Дешёвая проверка что runtime ещё жив.
    ///
    /// Postgres: SELECT 1.
    /// Redis: PING.
    /// SSH: check session alive.
    /// HTTP client: не нужен (stateless), default Ok(()).
    ///
    /// Когда вызывается зависит от topology:
    ///   Pool + test_on_checkout=true: при каждом checkout из idle.
    ///   Pool + CheckPolicy::Interval(30s): каждые 30 секунд.
    ///   Resident + stale_after(15s): раз в 15 секунд.
    ///   Service/Transport: через WatchdogHandle (если настроен).
    ///
    /// Default: Ok(()) — ресурс всегда "жив".
    fn check(
        &self,
        _runtime: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }

    /// Metadata для UI и diagnostics. Optional override.
    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::KEY)
    }
}
```

### Примеры

**Postgres** — Pooled, connection-based:

```rust
pub struct Postgres;

impl Resource for Postgres {
    type Config     = PgResourceConfig;
    type Runtime    = PgConnection;
    type Lease      = PgConnection;       // = Runtime (Pool topology)
    type Error      = PgError;
    type Credential = DatabaseCredential; // ← framework резолвит перед create()
    const KEY: ResourceKey = resource_key!("postgres");

    async fn create(
        &self,
        config: &PgResourceConfig,
        cred:   &DatabaseCredential,  // ← резолвлен framework-ом
        _ctx:   &dyn Ctx,
    ) -> Result<PgConnection, PgError> {
        let (client, connection) = tokio_postgres::Config::new()
            .host(&cred.host)
            .port(cred.port)
            .dbname(&cred.database)
            .user(&cred.username)
            .password(cred.password.expose())
            .connect_timeout(config.connect_timeout)
            .connect(NoTls)
            .await
            .map_err(PgError::Connect)?;

        let conn_task = tokio::spawn(async move {
            if let Err(e) = connection.await {
                tracing::warn!("postgres connection error: {}", e);
            }
        });

        Ok(PgConnection::new(client, conn_task))
    }

    async fn destroy(&self, conn: PgConnection) -> Result<(), PgError> {
        drop(conn.client);
        let _ = tokio::time::timeout(Duration::from_secs(2), conn.conn_task).await;
        Ok(())
    }

    async fn check(&self, conn: &PgConnection) -> Result<(), PgError> {
        if conn.conn_task.is_finished() {
            return Err(PgError::ConnectionClosed);
        }
        conn.client.simple_query("SELECT 1").await
            .map_err(|e| PgError::HealthCheck(e.to_string()))?;
        Ok(())
    }
}
```

**HTTP Client** — Resident, stateless:

```rust
pub struct HttpClient;

impl Resource for HttpClient {
    type Config     = HttpConfig;
    type Runtime    = reqwest::Client;
    type Lease      = reqwest::Client;    // = Runtime (Resident topology, Clone)
    type Error      = HttpError;
    type Credential = ();                 // нет credentials
    const KEY: ResourceKey = resource_key!("http.client");

    async fn create(
        &self,
        config: &HttpConfig,
        _cred:  &(),        // noop credential
        _ctx:   &dyn Ctx,
    ) -> Result<reqwest::Client, HttpError> {
        reqwest::Client::builder()
            .timeout(config.timeout)
            .pool_max_idle_per_host(config.max_idle_per_host)
            .build()
            .map_err(HttpError::Build)
    }

    // shutdown, destroy, check — всё default. HTTP client stateless.
}
```

**Telegram Bot** — Service, long-lived + polling loop:

```rust
pub struct TelegramBot;

impl Resource for TelegramBot {
    type Config     = TelegramResourceConfig;
    type Runtime    = TelegramBotRuntime;   // infrastructure: bot client + broadcast channel
    type Lease      = TelegramBotHandle;    // caller-facing: send_message, get_me
    type Error      = TelegramError;
    type Credential = TelegramCredential;   // { token: SecretString }
    const KEY: ResourceKey = resource_key!("telegram.bot");

    async fn create(
        &self,
        config: &TelegramResourceConfig,
        cred:   &TelegramCredential,    // ← резолвлен framework-ом
        _ctx:   &dyn Ctx,
    ) -> Result<TelegramBotRuntime, TelegramError> {
        // Setup infrastructure ONLY. DO NOT start polling loop here.
        // Polling = Daemon::run(), started by framework.
        let bot = Bot::new(cred.token.expose());
        let info = bot.get_me().await.map_err(TelegramError::Api)?;
        let (update_tx, _) = broadcast::channel(config.buffer_size);
        Ok(TelegramBotRuntime {
            inner: Arc::new(BotInner { bot, info, update_tx }),
        })
    }

    async fn destroy(&self, runtime: TelegramBotRuntime) -> Result<(), TelegramError> {
        // inner: Arc<BotInner> — dropped here.
        // Tokens hold Arc<BotInner> clones — continue working until dropped.
        // Framework cancels Daemon separately via CancellationToken.
        drop(runtime);
        Ok(())
    }

    async fn check(&self, runtime: &TelegramBotRuntime) -> Result<(), TelegramError> {
        runtime.inner.bot.get_me().await.map_err(TelegramError::Api)?;
        Ok(())
    }
}

// Runtime = data + capabilities. No lifecycle state (no cancel, no JoinHandle).
pub struct TelegramBotRuntime {
    inner: Arc<BotInner>,
}

struct BotInner {
    bot: Bot,
    info: BotInfo,
    update_tx: broadcast::Sender<Update>,
}
```

---

## ResourceConfig

```rust
pub trait ResourceConfig: Send + Sync + Clone + 'static {
    /// Валидация конфигурации. Вызывается при registration.
    fn validate(&self) -> Result<()> { Ok(()) }

    /// Стабильный fingerprint. При изменении — stale instances евиктируются.
    ///
    /// Включает operational settings. НЕ включает credential data
    /// (credential fingerprint отслеживается отдельно через credential store).
    ///
    /// Пример: если statement_timeout изменился — connections stale.
    /// Если password изменился — credential rotation event (отдельный flow).
    fn fingerprint(&self) -> u64 { 0 }
}
```

**Postgres config** — operational only (host/password в credential):

```rust
#[derive(Debug, Clone)]
pub struct PgResourceConfig {
    /// Connect timeout.
    pub connect_timeout: Duration,
    /// Statement timeout на connection level.
    pub statement_timeout: Option<Duration>,
    /// Application name — видно в pg_stat_activity.
    pub application_name: String,
    /// Default search_path. Переопределяется в prepare() per-tenant.
    pub search_path: Option<String>,
    /// Recycle method.
    pub recycle_method: RecycleMethod,
}

impl ResourceConfig for PgResourceConfig {
    fn validate(&self) -> Result<()> {
        if self.connect_timeout.is_zero() {
            return Err(Error::permanent("connect_timeout must be > 0"));
        }
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.statement_timeout.map(|d| d.as_millis()).hash(&mut h);
        self.application_name.hash(&mut h);
        self.search_path.hash(&mut h);
        h.finish()
    }
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
```

**Разделение Credential vs Config:**

```
Credential (encrypted, ротируется):
  ✓ host, port, database
  ✓ username, password
  ✓ SSL certs
  ✓ API tokens, OAuth tokens
  ✓ Custom endpoints (GitHub Enterprise URL)

Config (plain, operational):
  ✓ timeouts (connect, statement, idle)
  ✓ pool size (min, max)
  ✓ application name, client name
  ✓ recycle method
  ✓ search_path defaults
  ✓ warmup strategy
```

---

## Credential

Resource декларирует `type Credential`. Framework резолвит через `CredentialStore`.

```rust
/// Маркер: credential такого типа.
pub trait Credential: Send + Sync + Clone + 'static {
    /// Уникальный ключ. E.g. "database", "api_token", "telegram_bot", "ssh_key".
    const KIND: &'static str;
}

/// Нет credentials (HTTP client, статичный ресурс).
impl Credential for () { const KIND: &'static str = "none"; }

/// Credential store — резолвит credentials at runtime.
/// Реализуется платформой: vault, env vars, k8s secrets, nebula-credential.
///
/// Исправлено (#19): `fn resolve<C: Credential>` — generic method, не object-safe.
/// `dyn CredentialStore` + generic method = compile error. Решение: type erasure.
/// `resolve_erased` принимает `kind: &'static str`, возвращает `Box<dyn Any>`.
/// Typed доступ через `CredentialStoreExt` blanket impl — вызывается в typed context.
pub trait CredentialStore: Send + Sync {
    /// Type-erased resolve. Framework downcast-ит к нужному типу.
    /// `kind` = `C::KIND` (e.g. "database", "telegram_bot").
    fn resolve_erased(
        &self,
        scope: &ScopeLevel,
        kind:  &'static str,
    ) -> BoxFuture<'_, Result<Box<dyn Any + Send + Sync>, CredentialError>>;
}

/// Typed extension поверх CredentialStore. Blanket impl для всех T: CredentialStore.
/// Не dyn — вызывается в typed context (ManagedResource::create_instance).
pub trait CredentialStoreExt: CredentialStore {
    fn resolve<C: Credential + 'static>(
        &self,
        scope: &ScopeLevel,
    ) -> impl Future<Output = Result<C, CredentialError>> + Send {
        async move {
            let boxed = self.resolve_erased(scope, C::KIND).await?;
            boxed.downcast::<C>()
                .map(|b| *b)
                .map_err(|_| CredentialError::TypeMismatch {
                    expected: C::KIND,
                    got: "unknown",
                })
        }
    }
}
// ?Sized позволяет работать с dyn CredentialStore:
// ctx.credential_store() → &dyn CredentialStore → deref → dyn CredentialStore: CredentialStoreExt
impl<T: CredentialStore + ?Sized> CredentialStoreExt for T {}

/// Extension trait. Добавляет credential access к Ctx.
/// Отдельный trait = backward compatible (не ломает существующие Ctx impls).
/// `&dyn CredentialStore` object-safe (только resolve_erased — нет generics).
pub trait CredentialCtx: Ctx {
    fn credential_store(&self) -> &dyn CredentialStore;
}

// ── Credential rotation ──────────────────────────────────────────────
//
// При CredentialRotatedEvent (из nebula-eventbus):
//   Pool:    stale fingerprint → instances evicted at next recycle → recreate с новым cred.
//   Resident/Service/Transport/Exclusive: two-phase reload (destroy + create с новым cred).
//   Daemon:  cancel + restart с новым cred (via recreate path).
//
// Resource author ничего не делает — это ответственность framework.
// CredentialStore всегда возвращает актуальный credential.

// ── Примеры конкретных credential типов ─────────────────────────────

pub struct DatabaseCredential {
    pub host:     String,
    pub port:     u16,
    pub database: String,
    pub username: String,
    pub password: SecretString,
    pub ssl_mode: SslMode,
}
impl Credential for DatabaseCredential { const KIND: &'static str = "database"; }

pub struct TelegramCredential {
    pub token: SecretString,
}
impl Credential for TelegramCredential { const KIND: &'static str = "telegram_bot"; }

pub struct SshKeyCredential {
    pub username:    String,
    pub private_key: SecretString,
    pub passphrase:  Option<SecretString>,
}
impl Credential for SshKeyCredential { const KIND: &'static str = "ssh_key"; }
```

**Framework resolution** в `ManagedResource::create_instance()`:

```rust
async fn create_instance<R: Resource>(
    resource: &R,
    config:   &R::Config,
    ctx:      &dyn CredentialCtx,
) -> Result<R::Runtime, Error> {
    // .resolve() — via CredentialStoreExt blanket impl (typed, not dyn).
    // ctx.credential_store() returns &dyn CredentialStore (object-safe).
    // CredentialStoreExt is implemented for all T: CredentialStore,
    // including &dyn CredentialStore (via deref coercion to concrete impl).
    let credential = ctx.credential_store()
        .resolve::<R::Credential>(ctx.scope())  // ← CredentialStoreExt::resolve
        .await
        .map_err(|e| Error::permanent(e))?;

    resource.create(config, &credential, ctx)
        .await
        .map_err(Into::into)
}
```

---

## Ctx trait

Execution context. Передаётся в create(), prepare(), open_session() и другие lifecycle methods.

```rust
/// Resource execution context. Defined in nebula-resource (NOT in nebula-core).
/// nebula-core may provide a BaseCtx with ext<T>() for cross-crate extension support.
pub trait Ctx: Send + Sync {
    /// Scope текущего execution. Uses ScopeLevel from nebula-core.
    fn scope(&self) -> &ScopeLevel;

    /// Unique execution id.
    fn execution_id(&self) -> ExecutionId;

    /// Cancellation token для graceful abort.
    fn cancellation(&self) -> Option<&CancellationToken> { None }

    // Credential access — design deferred.
    // Will be a separate trait/extension, not part of base Ctx.
    // See: credential integration design (Resource × Credential × Protocol axes).

    /// Type-safe extension data.
    ///
    /// Используется для передачи domain-specific context:
    ///   ctx.ext::<TenantContext>() → Some(&TenantContext { schema: "acme", role: "app_user" })
    ///
    /// prepare() в Pooled использует для SET search_path TO tenant_schema.
    fn ext<T: Send + Sync + 'static>(&self) -> Option<&T> { None }
}

// Scope hierarchy: uses ScopeLevel from nebula-core.
// ScopeLevel: Global, Organization, Project, Workflow, Execution, Action.
// NOT a custom Scope enum — reuse core's hierarchy.
use nebula_core::scope::ScopeLevel;

/// Pre-populated type map. Immutable after construction.
/// HashMap<TypeId, Box<dyn Any + Send + Sync>>.
/// Engine creates once per execution, resources read in prepare()/create().
/// Access cost: ~10ns (one hash lookup + one TypeId comparison).
pub struct Extensions {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,

    /// Только в debug builds: type_name string → TypeId.
    /// Обнаруживает TypeId collision при duplicated crate versions.
    /// В release builds поле отсутствует — нет overhead.
    #[cfg(debug_assertions)]
    name_index: HashMap<&'static str, TypeId>,
}

impl Extensions {
    pub fn new() -> Self {
        Self {
            map: HashMap::with_capacity(4),
            #[cfg(debug_assertions)]
            name_index: HashMap::new(),
        }
    }

    pub fn insert<T: Send + Sync + 'static>(&mut self, value: T) {
        let type_id = TypeId::of::<T>();

        // Debug: detect TypeId collision (два разных TypeId для одного type_name =
        // duplicated crate version в workspace).
        #[cfg(debug_assertions)]
        {
            let type_name = std::any::type_name::<T>();
            if let Some(&existing_id) = self.name_index.get(type_name) {
                if existing_id != type_id {
                    tracing::error!(
                        type_name,
                        "TypeId collision in Extensions: two versions of the same crate \
                         are loaded. ext::<{}> will silently return None for some callers. \
                         Fix: unify dependency versions in Cargo.toml / cargo deny.",
                        type_name
                    );
                }
            }
            self.name_index.insert(type_name, type_id);
        }

        self.map.insert(type_id, Box::new(value));
    }

    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.map.get(&TypeId::of::<T>()).and_then(|v| v.downcast_ref::<T>())
    }

    /// Debug helper: поиск по type_name строке.
    /// Полезно если TypeId lookup вернул None — проверить есть ли коллизия.
    #[cfg(debug_assertions)]
    pub fn debug_get_by_name(&self, type_name: &str) -> Option<&dyn std::any::Any> {
        let type_id = self.name_index.get(type_name)?;
        self.map.get(type_id).map(|v| v.as_ref() as &dyn std::any::Any)
    }
}

/// Minimal Ctx implementation для тестов и простых случаев.
pub struct BasicCtx {
    pub scope:        ScopeLevel,
    pub execution_id: ExecutionId,
    pub cancel:       Option<CancellationToken>,
    pub extensions:   Extensions,
}
```

---

## Error

Три категории ошибок + scope.

```rust
#[derive(Debug)]
pub struct Error {
    kind:   ErrorKind,
    scope:  ErrorScope,
    source: Box<dyn std::error::Error + Send + Sync>,
}

/// Категория ошибки. Определяет retry behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Connection refused, timeout, temporary network issue.
    /// Framework retries с backoff.
    Transient,

    /// Auth failed, invalid config, database not found.
    /// Framework НЕ retries. Permanent failure.
    Permanent,

    /// Budget exceeded, quota exhausted, rate limit (not transient).
    /// Framework НЕ retries сейчас, но может retry через retry_after.
    /// Отличается от Transient: не "сломалось", а "закончилось".
    Exhausted { retry_after: Option<Duration> },

    /// Pool full, semaphore exhausted. Backpressure.
    /// Caller может retry или back off.
    Backpressure,

    /// Resource not found in registry.
    NotFound,

    /// Operation cancelled (CancellationToken).
    Cancelled,
}

/// Scope ошибки: весь ресурс или конкретная цель.
#[derive(Debug, Clone, Default)]
pub enum ErrorScope {
    /// Ресурс целиком broken. taint() appropriate.
    #[default]
    Resource,

    /// Ошибка привязана к конкретной цели.
    /// Telegram: BotBlocked { chat_id: 123 } — бот заблокирован в одном чате.
    /// Ресурс НЕ broken — другие цели работают. taint() НЕ appropriate.
    Target { id: String },
}

impl Error {
    pub fn transient(e: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self { kind: ErrorKind::Transient, scope: ErrorScope::Resource, source: Box::new(e) }
    }

    pub fn permanent(e: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self { kind: ErrorKind::Permanent, scope: ErrorScope::Resource, source: Box::new(e) }
    }

    pub fn exhausted(e: impl std::error::Error + Send + Sync + 'static, retry_after: Option<Duration>) -> Self {
        Self { kind: ErrorKind::Exhausted { retry_after }, scope: ErrorScope::Resource, source: Box::new(e) }
    }

    pub fn with_target(mut self, id: impl Into<String>) -> Self {
        self.scope = ErrorScope::Target { id: id.into() };
        self
    }

    pub fn is_retryable(&self) -> bool   { matches!(self.kind, ErrorKind::Transient | ErrorKind::Backpressure) }
    pub fn is_permanent(&self) -> bool   { self.kind == ErrorKind::Permanent }
    pub fn is_exhausted(&self) -> bool   { matches!(self.kind, ErrorKind::Exhausted { .. }) }
    pub fn is_target_scoped(&self) -> bool { matches!(self.scope, ErrorScope::Target { .. }) }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{:?}] {}", self.kind, self.source)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.source)
    }
}
```

### ClassifyError derive macro

Генерирует `From<MyError> for nebula_resource::Error` автоматически:

```rust
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

    #[error("query timeout after {0:?}")]
    #[classify(transient)]
    QueryTimeout(Duration),

    #[error("serialization failure — retry")]
    #[classify(transient)]
    SerializationFailure,

    #[error("too many connections")]
    #[classify(exhausted, retry_after = "30s")]
    TooManyConnections,

    #[error("bot blocked by user {chat_id}")]
    #[classify(transient, scope = target)]
    BotBlocked { chat_id: i64 },
}

// Macro generates:
impl From<PgError> for nebula_resource::Error {
    fn from(e: PgError) -> Self {
        match &e {
            PgError::Auth { .. }           => Error::permanent(e),
            PgError::DatabaseNotFound { .. } => Error::permanent(e),
            PgError::Connect(_)            => Error::transient(e),
            PgError::ConnectionClosed      => Error::transient(e),
            PgError::QueryTimeout(_)       => Error::transient(e),
            PgError::SerializationFailure  => Error::transient(e),
            PgError::TooManyConnections    => Error::exhausted(e, Some(Duration::from_secs(30))),
            PgError::BotBlocked { chat_id } => Error::transient(e).with_target(chat_id.to_string()),
        }
    }
}
```
