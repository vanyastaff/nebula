# nebula-resource v2 — Design Spec

## Goal

Integrate nebula-resource with credential v3, add optional resource-level Parameters for connection config, typed credential rotation, error classification with sanitization for test-connection, and restore the credential rotation system removed on 2026-03-15.

## Philosophy

- **Resource = connection + topology + credential integration.** Resource owns connection config (host/port/db), credential provides auth material. Together they form a working connection.
- **No dependency on nebula-credential.** Integration via nebula-core types (`CredentialId`, `CredentialKey`, `CredentialEvent`) + EventBus. Rotation callback receives typed `R::Auth`, not raw JSON.
- **Parameters are optional.** Resources CAN use `#[derive(Parameters)]` for UI config forms. Resources without UI (programmatic, test fixtures) don't need it.
- **Three rotation strategies.** HotSwap (re-authorize idle), DrainAndRecreate (evict + rebuild), Reconnect (session-bound protocols).
- **Security by default.** Rotation events validated, error messages sanitized, credential material never in logs.

## Post-Review Amendments

1. **`HasParameters` NOT a bound on Resource trait.** Optional via separate `UiConfigurable` trait.
2. **AuthorizeCallback receives `&R::Auth` (typed)**, not `&serde_json::Value`. Framework deserializes before calling callback.
3. **`credential_keys()` returns `Vec<CredentialKey>`**, not `Option<CredentialKey>`. Resources can accept multiple credential types.
4. **`test_resource()` accepts `TestOptions`** with `max_connections: 1, timeout`.
5. **Rotation event validation**: reject unknown CredentialId, size/depth limits on payload, authorize must succeed before pool update.
6. **Error sanitization**: generic messages externally, full details in structured log only.
7. **VISION.md updated**: `type Auth` is canonical approach. `ctx.credentials().get()` pattern retired.
8. **Missing F-09 error variants restored**: `CredentialNotConfigured`, `MissingCredential`.
9. **Partial HotSwap failure documented**: if any idle instance fails re-auth, drain entire pool (fail-closed).

---

## 1. Resource Trait — Optional Parameters

Resource Config does NOT require `HasParameters`. Resources CAN opt in for UI form generation:

```rust
// Minimal resource — no Parameters, no UI form
impl Resource for InternalCache {
    type Config = CacheConfig;        // just implements ResourceConfig
    type Auth = ();                   // no credentials needed
    type Runtime = DashMap<String, Value>;
    // ...
}

// Full resource — with Parameters for UI config form
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

    #[param(label = "Connection URL")]
    #[param(description = "Alternative: provide full connection string")]
    connection_url: Option<String>,  // supports DATABASE_URL pattern

    #[param(label = "SSL Mode", no_expression, default = "prefer")]
    ssl_mode: SslMode,

    #[param(label = "Max Connections", default = 10)]
    #[validate(range(1..=1000))]
    max_connections: u16,
}

impl Resource for PostgresResource {
    type Config = PostgresConfig;
    type Auth = IdentityPassword;
    type Runtime = PgPool;
    type Lease = PgConnection;
    type Error = PgError;
    // ...
}
```

**Optional UI trait** (not on Resource itself):
```rust
/// Marker trait for resources whose Config can render UI forms.
pub trait UiConfigurable: Resource
where
    Self::Config: HasParameters,
{
    fn config_parameters() -> ParameterCollection {
        Self::Config::parameters()
    }
}

/// Blanket impl: any Resource whose Config has Parameters is UiConfigurable.
impl<R: Resource> UiConfigurable for R where R::Config: HasParameters {}
```

---

## 2. Error Classification with Sanitization

`ResourceError` gains auth/connection classification. Error messages are SANITIZED — no credential material in external messages.

```rust
#[derive(Debug, thiserror::Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum ResourceError {
    /// Authentication failed — credential is invalid, expired, or revoked.
    #[classify(category = "auth", code = "RESOURCE:AUTH_FAILED")]
    #[error("authentication failed for resource `{resource_key}`")]
    AuthFailed {
        resource_key: String,
        /// Internal details for structured logging (NEVER exposed to user).
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Connection failed — host unreachable, port closed, DNS failure.
    #[classify(category = "connection", code = "RESOURCE:CONNECTION_FAILED")]
    #[error("connection failed for resource `{resource_key}`")]
    ConnectionFailed {
        resource_key: String,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Configuration error — invalid config values.
    #[classify(category = "validation", code = "RESOURCE:CONFIG_INVALID")]
    #[error("invalid configuration for resource `{resource_key}`: {message}")]
    ConfigInvalid {
        resource_key: String,
        message: String,
    },

    /// Credential not configured — rotation callback missing.
    #[classify(category = "configuration", code = "RESOURCE:CREDENTIAL_NOT_CONFIGURED")]
    #[error("credential not configured for resource `{resource_key}`")]
    CredentialNotConfigured {
        resource_key: String,
    },

    /// Expected credential not found in store/registry.
    #[classify(category = "not_found", code = "RESOURCE:MISSING_CREDENTIAL")]
    #[error("credential `{credential_id}` not found for resource `{resource_key}`")]
    MissingCredential {
        credential_id: String,
        resource_key: String,
    },

    // ... existing variants preserved
}
```

**Error classification helpers** for resource authors (optional, not required):
```rust
/// Helpers for classifying common driver errors.
pub mod classify_helpers {
    /// Classify a sqlx error into auth or connection.
    pub fn classify_sqlx(e: &sqlx::Error) -> ResourceErrorKind {
        match e {
            sqlx::Error::Database(db_err) => {
                let msg = db_err.message().to_lowercase();
                if msg.contains("password") || msg.contains("authentication")
                    || msg.contains("denied") || msg.contains("unauthorized")
                {
                    ResourceErrorKind::Auth
                } else {
                    ResourceErrorKind::Connection
                }
            }
            sqlx::Error::Io(_) | sqlx::Error::Tls(_) => ResourceErrorKind::Connection,
            _ => ResourceErrorKind::Unknown,
        }
    }

    // Future: classify_redis, classify_mongodb, etc.
}
```

Resource authors CAN use helpers or classify manually. Not forced.

---

## 3. Credential Rotation — Typed Callbacks

### 3.1 RotationStrategy (in nebula-resource)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum RotationStrategy {
    /// Re-authorize idle instances. In-flight finish with old creds.
    HotSwap,
    /// Evict all idle instances. New ones built with new creds.
    DrainAndRecreate,
    /// Same as Drain for session-bound protocols (TCP, WebSocket).
    Reconnect,
}
```

### 3.2 Typed AuthorizeCallback

Framework deserializes `serde_json::Value` into `R::Auth` BEFORE calling callback. Plugin receives typed auth, not raw JSON:

```rust
/// Callback that applies new credentials to an existing resource instance.
/// Receives typed auth material, not raw JSON.
pub type AuthorizeCallback<R: Resource> =
    Arc<dyn Fn(&mut R::Runtime, &R::Auth) -> Result<(), Error> + Send + Sync>;
```

Framework-internal flow:
```
Rotation event: (CredentialId, serde_json::Value)
    ↓ framework validates: known CredentialId? size < limit? depth < limit?
    ↓ framework deserializes: serde_json::from_value::<R::Auth>(value)?
    ↓ on deser failure: log error, skip rotation (fail-closed)
    ↓ on success: call authorize_callback(&mut runtime, &typed_auth)
    ↓ on callback failure: quarantine instance, drain pool (fail-closed)
```

### 3.3 Partial HotSwap Failure

If ANY idle instance fails re-authorization during HotSwap:
1. Mark failed instance as broken (remove from pool)
2. If >50% of idle instances fail → drain entire pool (escalate to DrainAndRecreate)
3. Log with correlation ID for debugging
4. Emit `ResourceEvent::RotationPartialFailure`

This is fail-closed: never leave a pool with mixed old/new credentials.

### 3.4 Manager credential integration

```rust
impl Manager {
    /// Register a resource with credential rotation support.
    pub fn register_with_credential<R: Resource>(
        &self,
        resource: R,
        config: R::Config,
        credential_keys: Vec<CredentialKey>,
        strategy: RotationStrategy,
        authorize: AuthorizeCallback<R>,
    ) -> Result<()>;

    /// Start listening for credential rotation events.
    /// Events are validated before dispatch (known CredentialId, size limits).
    pub fn spawn_rotation_listener(
        &self,
        events: impl Stream<Item = (CredentialId, serde_json::Value)> + Send + 'static,
    );
}
```

**Event validation in listener:**
```rust
// Inside spawn_rotation_listener:
while let Some((cred_id, value)) = events.next().await {
    // 1. Reject unknown CredentialId
    if !self.credential_pool_map.contains_key(&cred_id) {
        tracing::warn!(%cred_id, "rotation event for unknown credential, ignoring");
        continue;
    }
    // 2. Size limit (prevent OOM from malicious events)
    let size = serde_json::to_vec(&value).map(|v| v.len()).unwrap_or(0);
    if size > MAX_ROTATION_PAYLOAD_BYTES {
        tracing::error!(%cred_id, size, "rotation payload too large, ignoring");
        continue;
    }
    // 3. Dispatch to registered pools
    self.dispatch_rotation(cred_id, value).await;
}
```

---

## 4. Resource Dependencies — Multiple Credential Types

Resources declare which credential types they accept:

```rust
pub trait ResourceDependencies {
    /// Credential types this resource can authenticate with.
    /// Empty = no credentials needed.
    fn credential_keys() -> Vec<CredentialKey>
    where
        Self: Sized,
    {
        vec![]  // default: no credentials
    }

    /// How to handle credential rotation.
    fn rotation_strategy() -> RotationStrategy
    where
        Self: Sized,
    {
        RotationStrategy::HotSwap
    }
}

// Postgres: accepts password OR certificate auth
impl ResourceDependencies for PostgresResource {
    fn credential_keys() -> Vec<CredentialKey> {
        vec![
            credential_key!("identity_password"),
            credential_key!("certificate"),
        ]
    }
}
```

---

## 5. Test-Connection with Options

```rust
/// Options for testing a resource connection.
pub struct TestOptions {
    /// Override max connections for testing (default: 1).
    pub max_connections: Option<u16>,
    /// Timeout for the entire test (default: 10s).
    pub timeout: Duration,
}

impl Default for TestOptions {
    fn default() -> Self {
        Self {
            max_connections: Some(1),
            timeout: Duration::from_secs(10),
        }
    }
}

/// Test a resource by creating a minimal instance and running check().
/// Error messages are sanitized — no credentials in output.
pub async fn test_resource<R: Resource>(
    config: &R::Config,
    auth: R::Auth,
    options: TestOptions,
) -> TestResult {
    let result = tokio::time::timeout(options.timeout, async {
        let runtime = R::create(config, auth, &test_ctx()).await?;
        let check = R::check(&runtime).await;
        let _ = R::shutdown(&runtime).await;
        check
    }).await;

    match result {
        Ok(Ok(())) => TestResult::Ok,
        Ok(Err(e)) => TestResult::from_classified_error(&e),
        Err(_) => TestResult::ConnectionFailed {
            message: "connection test timed out".into(),
        },
    }
}

impl TestResult {
    /// Classify error and SANITIZE the message.
    fn from_classified_error(e: &(impl Classify + std::fmt::Display)) -> Self {
        // Log full details internally
        tracing::debug!(error = %e, "test_resource failed");
        // Return sanitized message externally
        match e.category().as_str() {
            "auth" => TestResult::AuthFailed {
                message: "authentication failed — check your credentials".into(),
            },
            "connection" => TestResult::ConnectionFailed {
                message: "connection failed — check host, port, and firewall".into(),
            },
            _ => TestResult::Unknown {
                message: "connection test failed".into(),
            },
        }
    }
}
```

---

## 6. Events and Metrics (F-10)

```rust
/// Resource lifecycle events (extends existing ResourceEvent).
#[non_exhaustive]
pub enum ResourceEvent {
    // ... existing variants ...

    /// Credential rotation completed for a pool.
    CredentialRotated {
        resource_key: ResourceKey,
        credential_key: CredentialKey,
        strategy: RotationStrategy,
    },
    /// Credential rotation partially failed — some instances re-authorized, some not.
    RotationPartialFailure {
        resource_key: ResourceKey,
        credential_key: CredentialKey,
        succeeded: usize,
        failed: usize,
    },
}

/// Reason an instance was cleaned up.
#[non_exhaustive]
pub enum CleanupReason {
    // ... existing variants ...
    /// Evicted due to credential rotation.
    CredentialRotated,
}
```

Metric: `NEBULA_RESOURCE_CREDENTIAL_ROTATED_TOTAL` counter, tagged by resource_key + strategy.

---

## 7. What Changes vs Current

| Area | Current | New |
|------|---------|-----|
| Resource Config | `type Config: ResourceConfig` | Unchanged. Optional `HasParameters` via `UiConfigurable` trait |
| Error types | `ErrorKind` (generic) | Add `AuthFailed`, `ConnectionFailed`, `CredentialNotConfigured`, `MissingCredential` with sanitized messages |
| Credential integration | Removed (2026-03-15) | Restored with typed `AuthorizeCallback<R>` (receives `&R::Auth`, not `&Value`) |
| RotationStrategy | Was in nebula-credential | Moved to nebula-resource |
| ResourceDependencies | No credential declaration | `credential_keys() -> Vec<CredentialKey>` + `rotation_strategy()` |
| Test-connection | No framework support | `test_resource()` with `TestOptions`, error classification + sanitization |
| Rotation validation | None | CredentialId whitelist, payload size limit, fail-closed on partial failure |
| Events/metrics | No rotation events | `CredentialRotated`, `RotationPartialFailure`, cleanup reason, counter |

---

## 8. Invariants (from CREDENTIAL_INTEGRATION.md, updated)

1. **`nebula-resource` does NOT depend on `nebula-credential`** — only nebula-core types + EventBus
2. **Typed auth in callbacks** — framework deserializes `Value` → `R::Auth` before plugin sees it
3. **Rotation is fail-closed** — partial HotSwap failure → drain pool. Never mixed old/new state
4. **In-flight instances never interrupted** — finish with old credentials
5. **Weak pool references** — `credential_pool_map` uses `Weak<dyn RotatablePool>`
6. **Auth error = create failure** — bad credential never enters pool
7. **Error messages sanitized** — credential material never in TestResult or external error messages
8. **Rotation events validated** — unknown CredentialId rejected, payload size limited

---

## 9. Not In Scope

- New topology patterns (7 existing sufficient)
- Manager API changes (register/acquire/shutdown unchanged)
- Recovery gate changes (working correctly)
- Connection string parsing (plugin concern, Config can have `connection_url: Option<String>`)
- Pool warm-up hooks (Phase 2)
- Create retry with backoff (handled by resilience pipeline at acquire time)
- Resource-level localization (uses parameter localization system)

---

## Serialization Strategy

See `2026-04-06-serialization-strategy-design.md` for cross-cutting serialization decisions affecting this crate.
