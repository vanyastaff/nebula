# nebula-resource v2 — Design Spec

## Goal

Integrate nebula-resource with credential v3, add resource-level Parameters for connection config, error classification for test-connection, and restore the credential rotation system that was removed on 2026-03-15 pending credential API stabilization.

## Philosophy

- **Resource = connection + topology + credential integration.** Resource owns connection config (host/port/db), credential provides auth material. Together they form a working connection.
- **No dependency on nebula-credential.** Integration via nebula-core types (`CredentialId`, `CredentialKey`, `CredentialEvent`) + EventBus + opaque callbacks.
- **Resource authoring via Parameters.** Resource connection config uses the same `#[derive(Parameters)]` as credentials — one form system for everything.
- **Three rotation strategies.** HotSwap (re-authorize idle), DrainAndRecreate (evict + rebuild), Reconnect (same as Drain, for session-bound protocols).

## Context

The credential integration was fully designed (F-01 through F-10 in `crates/resource/design/CREDENTIAL_INTEGRATION.md`) and partially implemented, then removed on 2026-03-15. The design is sound and should be restored with credential v3 compatibility.

---

## 1. Resource Trait — Add Parameters

Current Resource trait has `type Config`. Config is operational (no secrets). With credential v3, connection config (host/port/db) lives on the resource, not the credential.

Resource authors should be able to define connection config with `#[derive(Parameters)]`:

```rust
#[derive(Parameters, Deserialize, Clone)]
pub struct PostgresConfig {
    #[param(label = "Host")]
    #[validate(required)]
    host: String,

    #[param(label = "Port", default = 5432)]
    #[validate(range(1..=65535))]
    port: u16,

    #[param(label = "Database")]
    #[validate(required)]
    database: String,

    #[param(label = "SSL Mode", no_expression)]
    #[param(default = "prefer")]
    ssl_mode: SslMode,

    #[param(label = "Max Connections", default = 10)]
    #[validate(range(1..=1000))]
    max_connections: u16,
}

impl Resource for PostgresResource {
    type Config = PostgresConfig;  // has Parameters!
    type Auth = IdentityPassword;   // from credential v3
    type Runtime = PgPool;
    type Lease = PgConnection;
    type Error = PgError;

    async fn create(config: &PostgresConfig, auth: IdentityPassword, ctx: &Ctx) -> Result<PgPool> {
        // config has host/port/db, auth has username/password
        let dsn = format!(
            "postgres://{}:{}@{}:{}/{}",
            auth.identity, auth.password.expose(), config.host, config.port, config.database
        );
        PgPool::connect(&dsn).await
    }

    async fn check(pool: &PgPool) -> Result<()> {
        pool.execute("SELECT 1").await?;
        Ok(())
    }
}
```

**Key:** Resource Config uses `#[derive(Parameters)]` for UI form generation. Auth material comes from credential, connection params come from resource config. `create()` merges both.

---

## 2. Error Classification for Test-Connection

`ResourceError` gains auth vs connection classification:

```rust
#[derive(Debug, thiserror::Error, nebula_error::Classify)]
pub enum ResourceError {
    /// Authentication failed — credential is invalid, expired, or revoked.
    #[classify(category = "auth", code = "RESOURCE:AUTH_FAILED")]
    #[error("authentication failed for {resource_key}: {message}")]
    AuthFailed {
        resource_key: String,
        message: String,
    },

    /// Connection failed — host unreachable, port closed, DNS failure.
    #[classify(category = "connection", code = "RESOURCE:CONNECTION_FAILED")]
    #[error("connection failed for {resource_key}: {message}")]
    ConnectionFailed {
        resource_key: String,
        message: String,
    },

    /// Configuration error — invalid config values.
    #[classify(category = "validation", code = "RESOURCE:CONFIG_INVALID")]
    #[error("invalid configuration for {resource_key}: {message}")]
    ConfigInvalid {
        resource_key: String,
        message: String,
    },

    // ... existing variants
}
```

Resource impls classify errors in `create()` and `check()`:
```rust
async fn create(config: &PgConfig, auth: IdentityPassword, ctx: &Ctx) -> Result<PgPool> {
    PgPool::connect(&dsn).await.map_err(|e| {
        if e.to_string().contains("password authentication failed") {
            ResourceError::AuthFailed { resource_key: "postgres".into(), message: e.to_string() }
        } else {
            ResourceError::ConnectionFailed { resource_key: "postgres".into(), message: e.to_string() }
        }
    })
}
```

Framework maps to user-friendly messages:
- `AuthFailed` → "Credential problem: check your username/password"
- `ConnectionFailed` → "Connection problem: check host, port, firewall"
- `ConfigInvalid` → "Configuration problem: check your settings"

---

## 3. Credential Rotation — Restore F-01 through F-10

Restore the credential rotation system per `CREDENTIAL_INTEGRATION.md`. Key components:

### 3.1 RotationStrategy (lives in nebula-resource)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationStrategy {
    /// Re-authorize idle instances immediately. In-flight finish with old creds.
    HotSwap,
    /// Evict all idle instances. New ones created with new creds.
    DrainAndRecreate,
    /// Same as DrainAndRecreate for session-bound protocols (TCP, WebSocket).
    Reconnect,
}
```

### 3.2 AuthorizeCallback (decoupled from nebula-credential)

```rust
/// Opaque callback that applies credential state to a resource instance.
/// Typed by the plugin, erased by the framework.
pub type AuthorizeCallback<I> =
    Arc<dyn Fn(&mut I, &serde_json::Value) -> Result<(), Error> + Send + Sync>;
```

### 3.3 Manager credential integration

```rust
impl Manager {
    /// Register a resource with credential rotation support.
    pub fn register_with_credential<R: Resource>(
        &self,
        resource: R,
        config: R::Config,
        credential_key: CredentialKey,
        strategy: RotationStrategy,
        authorize: AuthorizeCallback<R::Runtime>,
    ) -> Result<()>;

    /// Start listening for credential rotation events.
    /// Accepts a stream to stay decoupled from nebula-credential.
    pub fn spawn_rotation_listener(
        &self,
        events: impl Stream<Item = (CredentialId, serde_json::Value)> + Send + 'static,
    );
}
```

### 3.4 Invariants (from CREDENTIAL_INTEGRATION.md)

1. `nebula-resource` does NOT depend on `nebula-credential`
2. Credential state stored as `serde_json::Value` — typing done in plugin callback
3. Rotation is atomic per pool — all idle updated (HotSwap) or all evicted (Drain)
4. In-flight instances never interrupted — finish with old credentials
5. `credential_pool_map` uses `Weak<dyn RotatablePool>` — doesn't prevent shutdown
6. Authorization error in create = create failure — bad credential never enters pool

---

## 4. Resource Dependencies Declaration

Resources declare what credentials they need:

```rust
pub trait ResourceDependencies {
    /// Which credential type this resource requires.
    fn credential_key() -> Option<CredentialKey>
    where
        Self: Sized,
    {
        None  // default: no credential needed
    }

    /// Rotation strategy when credential changes.
    fn rotation_strategy() -> RotationStrategy
    where
        Self: Sized,
    {
        RotationStrategy::HotSwap  // default
    }
}
```

---

## 5. Test-Connection Flow

Framework-level function:

```rust
/// Test a resource by creating a temporary instance and running check().
/// Returns classified result: AuthFailed, ConnectionFailed, or Ok.
pub async fn test_resource<R: Resource>(
    config: &R::Config,
    auth: R::Auth,
) -> TestResult {
    match R::create(config, auth, &test_ctx()).await {
        Ok(runtime) => match R::check(&runtime).await {
            Ok(()) => {
                let _ = R::destroy(runtime).await;
                TestResult::Ok
            }
            Err(e) => TestResult::from_error(e),
        },
        Err(e) => TestResult::from_error(e),
    }
}

impl TestResult {
    fn from_error(e: impl Classify) -> Self {
        match e.category().as_str() {
            "auth" => TestResult::AuthFailed { message: e.to_string() },
            "connection" => TestResult::ConnectionFailed { message: e.to_string() },
            _ => TestResult::Unknown { message: e.to_string() },
        }
    }
}
```

---

## 6. What Changes vs Current

| Area | Current | New |
|------|---------|-----|
| Resource Config | `type Config` (manual) | `type Config: HasParameters` (derive for UI forms) |
| Error types | `ErrorKind` (generic) | Add `AuthFailed`, `ConnectionFailed` variants with Classify |
| Credential integration | Removed (2026-03-15) | Restored per F-01–F-10 with credential v3 types |
| RotationStrategy | Was in nebula-credential | Moved to nebula-resource |
| ResourceDependencies | No credential declaration | `credential_key()` + `rotation_strategy()` |
| Test-connection | No framework support | `test_resource()` with error classification |
| Parameters integration | None | Config derives `Parameters` for UI |

---

## 7. Not In Scope

- New topology patterns (7 existing are sufficient)
- Manager API changes (register/acquire/shutdown unchanged)
- Recovery gate changes (working correctly)
- Metrics redesign (existing works)
- PendingStateStore backends (credential concern, not resource)
- Resource-level localization (uses parameter localization system)
