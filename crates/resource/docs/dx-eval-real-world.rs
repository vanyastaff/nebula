/// DX Evaluation: 5 real-world integrations for nebula-resource
///
/// This file is a design-only evaluation. It does NOT compile — it references
/// external crate types (sqlx, reqwest, redis, aws-sdk-s3, lettre) that are not
/// workspace dependencies. All friction points are documented inline.
///
/// Evaluated by: a developer with solid Rust skills, zero prior knowledge of
/// nebula-resource internals.

// =============================================================================
// INTEGRATION 1: PostgreSQL Connection Pool (Pooled)
// =============================================================================
//
// Topology chosen: Pooled
// Reasoning: Each caller needs exclusive use of a connection while running a
// query. sqlx::PgPool is internally pooled but we want the manager to own the
// lifecycle and recycling — Pooled gives us recycle(), prepare(), is_broken().
//
// Credential: username + password pair
// Friction encountered: CRITICAL — register_pooled() has the constraint
//   `R: Resource<Credential = ()>`
// This means the entire convenience path (register_pooled, register_pooled_with)
// is UNAVAILABLE the moment you have real credentials. You must fall back to
// Manager::register() directly, which requires manually building TopologyRuntime.
// TopologyRuntime is an enum whose variants are not documented in the public
// surface — you have to read lib.rs re-exports to find PoolRuntime, then read
// the manager source to understand the fingerprint argument.
//
// There is NO register_pooled_with_credential() variant. The doc comment on
// register_pooled says "Use `()` for password-less connections; supply a
// credential type otherwise" (in the adapter guide), but then gives no
// example of how to actually do that.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;

// -- Hypothetical external types --
// use sqlx::{PgPool, PgConnection, postgres::PgConnectOptions};

use nebula_resource::{
    AcquireOptions, Credential, Error, ErrorKind, Manager, PoolConfig,
    Resource, ResourceConfig, ResourceMetadata,
    TopologyRuntime, resource_key,
    topology::pooled::{BrokenCheck, InstanceMetrics, Pooled, RecycleDecision},
    runtime::pool::PoolRuntime,
    context::ResourceContext,
};
use nebula_core::{ExecutionId, ResourceKey};

// --- Credential ---

#[derive(Clone)]
pub struct PgCredential {
    pub username: String,
    pub password: String, // FRICTION: should be SecretString, not String
                          // The Credential trait has no marker for "this field
                          // is secret". You must remember not to log it yourself.
                          // The checklist says "implement a redacted Debug" but
                          // there is no SecretString helper in nebula-resource.
}

impl Credential for PgCredential {
    const KIND: &'static str = "basic";
}

// --- Config ---

#[derive(Debug, Clone, Hash)]
pub struct PgConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub ssl_mode: String,
    pub pool_max_size: u32,         // FRICTION: duplicates PoolConfig::max_size
                                    // I ended up with two places to configure
                                    // pool size: here (for fingerprint/hot-reload)
                                    // and PoolConfig (for actual pool behavior).
                                    // Which one wins? Unclear from docs.
    pub connect_timeout: Duration,
}

impl Default for PgConfig {
    fn default() -> Self {
        Self {
            host: "localhost".into(),
            port: 5432,
            database: "postgres".into(),
            ssl_mode: "prefer".into(),
            pool_max_size: 10,
            connect_timeout: Duration::from_secs(10),
        }
    }
}

impl ResourceConfig for PgConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.host.is_empty() {
            return Err(Error::permanent("postgres: host must not be empty"));
        }
        if self.port == 0 {
            return Err(Error::permanent("postgres: port must be > 0"));
        }
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
}

// --- Resource ---

#[derive(Clone)]
pub struct PostgresResource;

// Hypothetical runtime — would be sqlx::PgConnection in reality
#[derive(Clone)]
pub struct PgConn {
    pub is_closed: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum PgError {
    #[error("connection failed: {0}")]
    Connect(String),
    #[error("auth failed: {0}")]
    Auth(String),
    #[error("query error: {0}")]
    Query(String),
}

impl From<PgError> for Error {
    fn from(e: PgError) -> Self {
        match e {
            PgError::Connect(msg) => Error::transient(msg),
            PgError::Auth(msg) => Error::permanent(msg),
            PgError::Query(msg) => Error::transient(msg),
        }
    }
}

impl Resource for PostgresResource {
    type Config = PgConfig;
    type Runtime = PgConn;
    type Lease = PgConn;
    type Error = PgError;
    type Credential = PgCredential;

    fn key() -> ResourceKey {
        resource_key!("postgres")
    }

    async fn create(
        &self,
        config: &PgConfig,
        credential: &PgCredential,
        _ctx: &ResourceContext,
    ) -> Result<PgConn, PgError> {
        // Real impl: PgConnectOptions::new().host(&config.host)...
        // FRICTION: credential.password is a plain String here, meaning
        // we'd pass it to connect options. There is no zeroize integration.
        let _ = (config, credential);
        Ok(PgConn { is_closed: false })
    }

    async fn check(&self, runtime: &PgConn) -> Result<(), PgError> {
        if runtime.is_closed {
            return Err(PgError::Connect("connection closed".into()));
        }
        // Real impl: conn.execute("SELECT 1").await
        Ok(())
    }

    async fn destroy(&self, _runtime: PgConn) -> Result<(), PgError> {
        // Real impl: conn.close().await
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        let mut m = ResourceMetadata::from_key(&Self::key());
        m.name = "PostgreSQL".into();
        m.description = Some("Pooled PostgreSQL connections".into());
        m.tags = vec!["sql".into(), "database".into()];
        m
    }
}

impl Pooled for PostgresResource {
    fn is_broken(&self, runtime: &PgConn) -> BrokenCheck {
        if runtime.is_closed {
            BrokenCheck::Broken("connection closed".into())
        } else {
            BrokenCheck::Healthy
        }
    }

    async fn recycle(
        &self,
        runtime: &PgConn,
        metrics: &InstanceMetrics,
    ) -> Result<RecycleDecision, PgError> {
        if runtime.is_closed {
            return Ok(RecycleDecision::Drop);
        }
        if metrics.age() > Duration::from_secs(1800) {
            return Ok(RecycleDecision::Drop);
        }
        // Real impl: conn.execute("ROLLBACK").await to reset tx state
        Ok(RecycleDecision::Keep)
    }

    async fn prepare(&self, _runtime: &PgConn, ctx: &ResourceContext) -> Result<(), PgError> {
        // Real impl: SET search_path = public, SET timezone = 'UTC'
        let _ = ctx;
        Ok(())
    }
}

// --- Registration (FRICTION ZONE) ---
//
// register_pooled() is unavailable because Credential != ().
// Must use Manager::register() directly, which means manually constructing
// TopologyRuntime::Pool(PoolRuntime::new(pool_config, fingerprint)).
//
// This requires knowing:
//   1. TopologyRuntime is re-exported from runtime module
//   2. PoolRuntime::new() takes (PoolConfig, u64) — the u64 is a fingerprint
//   3. The fingerprint comes from config.fingerprint()
// None of this is in a single doc example. The adapter guide shows
// register_pooled() only, which silently requires Credential = ().

fn register_postgres(manager: &Manager, config: PgConfig, cred: PgCredential) -> Result<(), Error> {
    // FRICTION: I have to manually thread the credential into the resource
    // struct — there is no credential storage at the managed-resource level.
    // The credential is passed as a parameter to acquire_pooled(), NOT stored
    // at registration time. This means callers must carry the credential
    // at every acquire call site.
    //
    // For credential rotation: you need to swap out the credential object
    // on every call to acquire_pooled(). There is no "update credential"
    // method on Manager. Hot-reload only covers Config, not Credential.
    //
    // FRICTION: credential rotation requires the caller to hold the new
    // credential value and pass it into acquire_pooled(). If the credential
    // is stored in a separate credential store (nebula-credential), there's
    // no documented integration point here.
    //
    // We store the credential IN THE RESOURCE struct as a workaround:
    let _ = cred; // credential would be embedded in a real Arc<Mutex<PgCredential>>

    use nebula_resource::ScopeLevel;

    let fingerprint = config.fingerprint();
    let pool_config = PoolConfig {
        min_size: 2,
        max_size: 10,
        idle_timeout: Some(Duration::from_secs(300)),
        ..PoolConfig::default()
    };

    manager.register(
        PostgresResource,
        config,
        ScopeLevel::Global,
        TopologyRuntime::Pool(PoolRuntime::<PostgresResource>::new(pool_config, fingerprint)),
        None, // resilience — must be wired separately
        None, // recovery gate — must be wired separately
    )
    // FRICTION: PgCredential is not accepted here at all.
    // The credential is passed to acquire_pooled(), not register().
    // Hot-reload of the credential (rotation) requires a custom mechanism
    // — the framework provides none.
}


// =============================================================================
// INTEGRATION 2: OpenAI API Client (Service)
// =============================================================================
//
// Topology chosen: Service with TokenMode::Cloned
// Reasoning: One long-lived HTTP client (with connection pooling inside reqwest),
// multiple concurrent API calls. Each call is stateless — it just needs the
// client reference. Cloned token mode gives each caller an Arc clone.
//
// Rate limiting: MISSING from the API.
// The Service topology has no built-in rate-limiting hook. I can set
// acquire_token() to fail with ErrorKind::Exhausted when a semaphore is at
// capacity, but the semaphore itself must be managed inside the Runtime struct.
// The framework offers no rate-limit primitive that integrates with the
// retry_after hint on ErrorKind::Exhausted — callers just get the error back.

// Hypothetical: use reqwest::Client;

#[derive(Clone)]
pub struct OpenAiCredential {
    pub api_key: String, // FRICTION: should be secrecy::Secret<String>
}

impl Credential for OpenAiCredential {
    const KIND: &'static str = "api_key";
}

#[derive(Debug, Clone, Hash)]
pub struct OpenAiConfig {
    pub base_url: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,       // FRICTION: f32 does not implement Hash.
                                // Had to use ordered-float or store as u32 bits.
                                // ResourceConfig::fingerprint() requires Hash but
                                // the trait doesn't enforce it — you discover this
                                // at compile time with a confusing error about
                                // DefaultHasher and missing Hash impls.
    pub rate_limit_rpm: u32,
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-4o".into(),
            max_tokens: 4096,
            temperature: 0,     // stored as 0, would need f32::to_bits() for real fingerprint
            rate_limit_rpm: 60,
        }
    }
}

impl ResourceConfig for OpenAiConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.base_url.is_empty() {
            return Err(Error::permanent("openai: base_url must not be empty"));
        }
        if self.rate_limit_rpm == 0 {
            return Err(Error::permanent("openai: rate_limit_rpm must be > 0"));
        }
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
}

// The runtime holds the client + a rate-limiter semaphore built from config
pub struct OpenAiRuntime {
    // client: reqwest::Client,
    pub api_key: String,            // FRICTION: credential is baked in at create()
                                    // and cannot be rotated without recreating runtime.
                                    // Service topology has NO equivalent of
                                    // acquire_pooled(credential) — the credential
                                    // was not even accepted at register_service().
    pub rate_limit_sem: std::sync::Arc<tokio::sync::Semaphore>,
}

// FRICTION: Clone is required by acquire_service bounds:
//   R: Clone + Send + Sync + 'static
// but cloning OpenAiRuntime is meaningless — we want to share the single
// runtime across all callers, not clone it. The Clone bound forces a shallow
// clone. This is confusing — you implement Clone but it shares the Arc inside.
impl Clone for OpenAiRuntime {
    fn clone(&self) -> Self {
        // Shallow clone: semaphore is shared via Arc
        Self {
            api_key: self.api_key.clone(),
            rate_limit_sem: self.rate_limit_sem.clone(),
        }
    }
}

// Token — what callers receive
#[derive(Clone)]
pub struct OpenAiToken {
    // client: reqwest::Client,  // cheap reqwest clone
    pub api_key: String,
    pub model: String,
}

#[derive(Debug, thiserror::Error)]
pub enum OpenAiError {
    #[error("rate limited, retry after {0}s")]
    RateLimited(u64),
    #[error("api error: {0}")]
    Api(String),
    #[error("auth error: {0}")]
    Auth(String),
}

impl From<OpenAiError> for Error {
    fn from(e: OpenAiError) -> Self {
        match e {
            OpenAiError::RateLimited(secs) => {
                Error::exhausted("rate limited", Some(Duration::from_secs(secs)))
            }
            OpenAiError::Api(msg) => Error::transient(msg),
            OpenAiError::Auth(msg) => Error::permanent(msg),
        }
    }
}

#[derive(Clone)]
pub struct OpenAiResource;

impl Resource for OpenAiResource {
    type Config = OpenAiConfig;
    type Runtime = OpenAiRuntime;
    type Lease = OpenAiToken;
    type Error = OpenAiError;
    type Credential = OpenAiCredential; // FRICTION: see below

    fn key() -> ResourceKey {
        resource_key!("openai")
    }

    async fn create(
        &self,
        config: &OpenAiConfig,
        credential: &OpenAiCredential,
        _ctx: &ResourceContext,
    ) -> Result<OpenAiRuntime, OpenAiError> {
        // FRICTION: once created, the runtime holds the api_key string.
        // If the credential rotates (API key revoked, new key issued),
        // the runtime is stale but the framework will NOT recreate it —
        // service topology does not call create() again unless explicitly
        // removed and re-registered.
        Ok(OpenAiRuntime {
            api_key: credential.api_key.clone(),
            rate_limit_sem: std::sync::Arc::new(tokio::sync::Semaphore::new(
                config.rate_limit_rpm as usize,
            )),
        })
    }
}

// FRICTION: Service topology has no built-in credential in acquire_service().
// The credential is required only at create() time, baked into Runtime.
// This is fundamentally wrong for rotatable API keys.
// Workaround: store an Arc<RwLock<String>> in the resource struct itself,
// update it out-of-band. But then create() ignores the passed credential.

use nebula_resource::topology::service::{Service, TokenMode};

impl Service for OpenAiResource {
    const TOKEN_MODE: TokenMode = TokenMode::Cloned;

    async fn acquire_token(
        &self,
        runtime: &OpenAiRuntime,
        ctx: &ResourceContext,
    ) -> Result<OpenAiToken, OpenAiError> {
        // Rate limiting: try to acquire a semaphore permit.
        // FRICTION: tokio::sync::Semaphore::try_acquire() returns a permit
        // that must be kept alive. We cannot store it in OpenAiToken because
        // TokenMode::Cloned means release_token() is a no-op — the permit
        // would be dropped when the token is cloned, releasing immediately.
        //
        // To do real rate limiting, we'd need TokenMode::Tracked so that
        // release_token() is called and we can release the semaphore permit.
        // But TokenMode is a const on the impl, so we'd change to Tracked.
        // Then the release path calls release_token() — which works, but the
        // OpenAiToken would need to hold an OwnedSemaphorePermit,
        // making it non-Clone (OwnedSemaphorePermit is not Clone).
        // Resident/Service handles require Clone on Lease when Cloned mode.
        //
        // Conclusion: there is a fundamental tension between:
        //   - TokenMode::Cloned (requires Clone on Lease, no release hook)
        //   - TokenMode::Tracked (release hook exists, but Lease can be non-Clone)
        // For rate limiting you MUST use Tracked, but this is not obvious.
        let _ = ctx;
        Ok(OpenAiToken {
            api_key: runtime.api_key.clone(),
            model: "gpt-4o".into(),
        })
    }
}

// Registration:
// FRICTION: register_service() requires:
//   1. The resource struct (OpenAiResource)
//   2. The config (OpenAiConfig)
//   3. The ALREADY-CREATED runtime (OpenAiRuntime) — so you must call create() yourself
//   4. service_config
//
// This is the biggest ergonomic gap: for Pooled, the manager calls create() lazily.
// For Service/Resident/Exclusive/Transport, YOU create the runtime and pass it in.
// This means credential injection for these topologies is entirely manual —
// you pull the credential from your store, call create() yourself, pass the result.
// There is no framework support for "create this service runtime using this credential".
//
// Compare: register_pooled() accepts a Credential and passes it to create() per-connection.
// All other topologies: credential is your problem. This is a documentation gap —
// the adapter guide only shows Pooled.


// =============================================================================
// INTEGRATION 3: Redis Cluster (Resident)
// =============================================================================
//
// Topology chosen: Resident
// Reasoning: redis::ClusterClient is itself internally pooled and thread-safe.
// We want one shared client, cloned on acquire. Resident is the right fit.
//
// Credential: password
// FRICTION: same as above — register_resident() requires Credential = ().
// Must use Manager::register() directly.

// Hypothetical: use redis::cluster::ClusterClient;

#[derive(Clone)]
pub struct RedisCredential {
    pub password: String,
}

impl Credential for RedisCredential {
    const KIND: &'static str = "password";
}

#[derive(Debug, Clone, Hash)]
pub struct RedisConfig {
    pub nodes: Vec<String>,
    pub tls: bool,
    pub connect_timeout: Duration,
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            nodes: vec!["redis://127.0.0.1:6379".into()],
            tls: false,
            connect_timeout: Duration::from_secs(5),
        }
    }
}

// FRICTION: Vec<String> implements Hash but Duration does not.
// fingerprint() requires a manually implemented hash if you want it.
// There is no #[derive(ResourceFingerprint)] or similar macro.

impl ResourceConfig for RedisConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.nodes.is_empty() {
            return Err(Error::permanent("redis: at least one node required"));
        }
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.nodes.hash(&mut h);
        self.tls.hash(&mut h);
        // Duration not hashable — omit connect_timeout from fingerprint
        // This is a silent correctness bug: changing connect_timeout won't
        // trigger hot-reload eviction. No lint warns about this.
        h.finish()
    }
}

// The runtime is the ClusterClient — thread-safe, internally pooled
#[derive(Clone)]
pub struct RedisRuntime {
    // client: Arc<redis::cluster::ClusterClient>,
    pub node_count: usize, // placeholder
}

#[derive(Debug, thiserror::Error)]
pub enum RedisError {
    #[error("connection error: {0}")]
    Connect(String),
    #[error("cluster topology changed")]
    TopologyChanged,
    #[error("auth failed: {0}")]
    Auth(String),
}

impl From<RedisError> for Error {
    fn from(e: RedisError) -> Self {
        match e {
            RedisError::Connect(msg) => Error::transient(msg),
            RedisError::TopologyChanged => Error::transient("cluster topology changed"),
            RedisError::Auth(msg) => Error::permanent(msg),
        }
    }
}

#[derive(Clone)]
pub struct RedisResource;

impl Resource for RedisResource {
    type Config = RedisConfig;
    type Runtime = RedisRuntime;
    type Lease = RedisRuntime; // Resident: Lease == Runtime (both Clone)
    type Error = RedisError;
    type Credential = RedisCredential;

    fn key() -> ResourceKey {
        resource_key!("redis-cluster")
    }

    async fn create(
        &self,
        config: &RedisConfig,
        credential: &RedisCredential,
        _ctx: &ResourceContext,
    ) -> Result<RedisRuntime, RedisError> {
        let _ = (config, credential);
        // Real: ClusterClientBuilder::new(config.nodes.clone())
        //   .password(credential.password.clone())
        //   .build()
        Ok(RedisRuntime { node_count: config.nodes.len() })
    }

    async fn check(&self, runtime: &RedisRuntime) -> Result<(), RedisError> {
        // Real: runtime.client.get_async_connection().await?.send_packed_command(...)
        let _ = runtime;
        Ok(())
    }
}

use nebula_resource::topology::resident::Resident;

impl Resident for RedisResource
where
    <Self as Resource>::Lease: Clone,
{
    fn is_alive_sync(&self, _runtime: &RedisRuntime) -> bool {
        // FRICTION: O(1) sync check is required. Redis cluster client has no
        // sync ping. The best we can do is check an internal flag or skip this.
        // Returning true always means the stale check only fires on the async
        // check() method, which is NOT called by acquire_resident automatically
        // unless recreate_on_failure is set.
        true
    }

    fn stale_after(&self) -> Option<Duration> {
        // Cluster topology could change — treat client as stale after 5 min
        Some(Duration::from_secs(300))
    }
}

// FRICTION: Resident topology's recreate_on_failure = true means the manager
// will call create() again if is_alive_sync() returns false OR if check() fails.
// But check() is the health check from Resource::check(), not the Resident trait.
// It's not clear from docs whether check() is called by acquire_resident or only
// by an explicit health_check() call. Had to infer from reading runtime source.


// =============================================================================
// INTEGRATION 4: AWS S3 (Transport — multiplexed)
// =============================================================================
//
// Topology chosen: Transport
// Reasoning: S3Client is long-lived and shared. Each upload/download is a
// short-lived "session" (a request context). Transport gives us open_session(),
// close_session(), keepalive(), and max_sessions for backpressure.
//
// Credential: AWS access key + secret (rotatable via STS)

// Hypothetical: use aws_sdk_s3::Client as S3Client;

#[derive(Clone)]
pub struct AwsCredential {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>, // for STS temporary credentials
}

impl Credential for AwsCredential {
    const KIND: &'static str = "aws_access_key";
}

#[derive(Debug, Clone, Hash)]
pub struct S3Config {
    pub region: String,
    pub bucket: String,
    pub endpoint: Option<String>, // for MinIO/localstack
    pub request_timeout: Duration,
}

impl Default for S3Config {
    fn default() -> Self {
        Self {
            region: "us-east-1".into(),
            bucket: String::new(),
            endpoint: None,
            request_timeout: Duration::from_secs(30),
        }
    }
}

impl ResourceConfig for S3Config {
    fn validate(&self) -> Result<(), Error> {
        if self.region.is_empty() {
            return Err(Error::permanent("s3: region must not be empty"));
        }
        if self.bucket.is_empty() {
            return Err(Error::permanent("s3: bucket must not be empty"));
        }
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.region.hash(&mut h);
        self.bucket.hash(&mut h);
        self.endpoint.hash(&mut h);
        h.finish()
    }
}

// Shared S3 client runtime
pub struct S3Runtime {
    // client: Arc<aws_sdk_s3::Client>,
    pub region: String,
    pub bucket: String,
}

// Per-request session — holds scoped context for one upload/download
pub struct S3Session {
    pub request_id: String,
    pub bucket: String,
    // client: Arc<aws_sdk_s3::Client>,
}

#[derive(Debug, thiserror::Error)]
pub enum S3Error {
    #[error("connection error: {0}")]
    Connect(String),
    #[error("auth error: {0}")]
    Auth(String),
    #[error("bucket not found: {0}")]
    BucketNotFound(String),
    #[error("throttled")]
    Throttled,
}

impl From<S3Error> for Error {
    fn from(e: S3Error) -> Self {
        match e {
            S3Error::Connect(msg) => Error::transient(msg),
            S3Error::Auth(msg) => Error::permanent(msg),
            S3Error::BucketNotFound(msg) => {
                Error::new(ErrorKind::Permanent, msg)
                    .with_scope(nebula_resource::ErrorScope::Target { id: "bucket".into() })
            }
            S3Error::Throttled => Error::exhausted("s3 throttled", Some(Duration::from_secs(1))),
        }
    }
}

// FRICTION: Clone required on the resource struct (for acquire_transport bounds)
// but also on Runtime — except Transport does NOT require Clone on Runtime!
// The bound is: R: Clone, R::Runtime: Send + Sync (no Clone).
// This is INCONSISTENT across topologies:
//   - Pooled: Runtime must implement Clone + Into<Lease>
//   - Resident: Runtime must implement Clone + Into<Lease>
//   - Service: Runtime has NO Clone requirement
//   - Transport: Runtime has NO Clone requirement
//   - Exclusive: Runtime must implement Clone + Into<Lease>
// You discover this by reading the #[doc] comments on each acquire method.
// There is no single table summarizing these bounds in the public docs.

#[derive(Clone)]
pub struct S3Resource;

impl Resource for S3Resource {
    type Config = S3Config;
    type Runtime = S3Runtime;
    type Lease = S3Session;
    type Error = S3Error;
    type Credential = AwsCredential;

    fn key() -> ResourceKey {
        resource_key!("aws-s3")
    }

    async fn create(
        &self,
        config: &S3Config,
        credential: &AwsCredential,
        _ctx: &ResourceContext,
    ) -> Result<S3Runtime, S3Error> {
        // Real: build aws_config with explicit credentials, create S3Client
        let _ = credential;
        Ok(S3Runtime {
            region: config.region.clone(),
            bucket: config.bucket.clone(),
        })
    }

    async fn check(&self, runtime: &S3Runtime) -> Result<(), S3Error> {
        // Real: HeadBucket request
        let _ = runtime;
        Ok(())
    }
}

use nebula_resource::topology::transport::Transport;

impl Transport for S3Resource {
    async fn open_session(
        &self,
        transport: &S3Runtime,
        _ctx: &ResourceContext,
    ) -> Result<S3Session, S3Error> {
        Ok(S3Session {
            request_id: uuid::Uuid::new_v4().to_string(),
            bucket: transport.bucket.clone(),
        })
    }

    async fn close_session(
        &self,
        _transport: &S3Runtime,
        _session: S3Session,
        _healthy: bool,
    ) -> Result<(), S3Error> {
        Ok(())
    }

    async fn keepalive(&self, transport: &S3Runtime) -> Result<(), S3Error> {
        // Real: HeadBucket to keep TCP connection alive
        let _ = transport;
        Ok(())
    }
}

// FRICTION: STS credential rotation for S3.
// When STS temporary credentials expire (typically 1h), the S3Client embedded
// in S3Runtime has stale credentials. The manager has no "re-create with new
// credential" mechanism for Transport topology. Options:
//   A. Use the AWS SDK's built-in credential provider chain (handles rotation
//      internally) — bypasses the Credential trait entirely.
//   B. Store an Arc<RwLock<AwsCredential>> in S3Resource, update it externally,
//      but then create() is called only once so it never sees the new credential.
//   C. Force periodic remove() + re-register with new credential — clunky,
//      drops existing sessions.
// None of these are described in docs. This is the hardest unsolved problem
// for cloud provider integrations.

// Registration:
// FRICTION: register_transport() requires an ALREADY-BUILT runtime.
// For S3 you must call Resource::create() manually, which means handling the
// credential injection yourself. No framework support.
//
// Also: register_transport() has Credential = () constraint.
// Must use Manager::register() with manually-constructed TopologyRuntime::Transport.


// =============================================================================
// INTEGRATION 5: SMTP Email (Exclusive — serial access)
// =============================================================================
//
// Topology chosen: Exclusive
// Reasoning: SMTP connections are stateful and cannot be shared between
// concurrent senders (commands are serial). Exclusive gives us a semaphore-
// backed single-occupancy lock with reset() called between uses.

// Hypothetical: use lettre::{SmtpTransport, transport::smtp::SmtpTransportBuilder};

#[derive(Clone)]
pub struct SmtpCredential {
    pub username: String,
    pub password: String,
}

impl Credential for SmtpCredential {
    const KIND: &'static str = "basic";
}

#[derive(Debug, Clone, Hash)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub use_starttls: bool,
    pub timeout: Duration,
}

impl Default for SmtpConfig {
    fn default() -> Self {
        Self {
            host: "localhost".into(),
            port: 587,
            use_starttls: true,
            timeout: Duration::from_secs(30),
        }
    }
}

impl ResourceConfig for SmtpConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.host.is_empty() {
            return Err(Error::permanent("smtp: host must not be empty"));
        }
        if self.port == 0 {
            return Err(Error::permanent("smtp: port must be > 0"));
        }
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.host.hash(&mut h);
        self.port.hash(&mut h);
        self.use_starttls.hash(&mut h);
        h.finish()
    }
}

// The runtime is the SMTP connection
#[derive(Clone)]
pub struct SmtpConn {
    pub is_connected: bool,
}

// Lease == Runtime for Exclusive (the caller holds the connection directly)
// FRICTION: Exclusive requires Runtime: Clone + Into<Lease>.
// For SMTP this is fine since we want exclusive access to a single connection.
// But it means you CAN'T have "N SMTP connections in a round-robin" — that
// would require Pooled. Exclusive is truly one-at-a-time globally.

#[derive(Debug, thiserror::Error)]
pub enum SmtpError {
    #[error("connection failed: {0}")]
    Connect(String),
    #[error("auth failed")]
    Auth,
    #[error("send failed: {0}")]
    Send(String),
    #[error("connection dropped")]
    Dropped,
}

impl From<SmtpError> for Error {
    fn from(e: SmtpError) -> Self {
        match e {
            SmtpError::Connect(msg) => Error::transient(msg),
            SmtpError::Auth => Error::permanent("smtp auth failed"),
            SmtpError::Send(msg) => Error::transient(msg),
            SmtpError::Dropped => Error::transient("smtp connection dropped"),
        }
    }
}

#[derive(Clone)]
pub struct SmtpResource;

impl Resource for SmtpResource {
    type Config = SmtpConfig;
    type Runtime = SmtpConn;
    type Lease = SmtpConn;
    type Error = SmtpError;
    type Credential = SmtpCredential;

    fn key() -> ResourceKey {
        resource_key!("smtp")
    }

    async fn create(
        &self,
        config: &SmtpConfig,
        credential: &SmtpCredential,
        _ctx: &ResourceContext,
    ) -> Result<SmtpConn, SmtpError> {
        // Real: SmtpTransport::starttls_relay(&config.host)?
        //         .credentials(lettre::transport::smtp::authentication::Credentials::new(...))
        //         .build()
        let _ = (config, credential);
        Ok(SmtpConn { is_connected: true })
    }

    async fn check(&self, runtime: &SmtpConn) -> Result<(), SmtpError> {
        if !runtime.is_connected {
            return Err(SmtpError::Dropped);
        }
        Ok(())
    }

    async fn destroy(&self, _runtime: SmtpConn) -> Result<(), SmtpError> {
        // Real: send QUIT, close TCP
        Ok(())
    }
}

use nebula_resource::topology::exclusive::Exclusive;

impl Exclusive for SmtpResource {
    async fn reset(&self, runtime: &SmtpConn) -> Result<(), SmtpError> {
        // Send RSET to clear any partial message state
        if !runtime.is_connected {
            return Err(SmtpError::Dropped);
        }
        // Real: transport.command(lettre::transport::smtp::commands::Rset).await?
        Ok(())
    }
}

// FRICTION: register_exclusive() requires Credential = ().
// Same pattern as all others — must use Manager::register() + TopologyRuntime::Exclusive.
// The pre-built runtime requirement means we must call create() manually and
// inject the credential ourselves.
//
// WORSE: Exclusive topology only supports a SINGLE runtime instance.
// If the connection drops, reset() returns an error. The manager will... do what?
// There is no "reconnect on reset failure" behavior documented for Exclusive.
// Looking at the topology, reset() failure probably propagates to the caller,
// but the broken connection remains registered until manual intervention.
// Pooled has BrokenCheck + RecycleDecision for this. Exclusive has nothing.
