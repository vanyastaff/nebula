# nebula-resource — Writing Adapter Crates

An adapter crate (e.g. `nebula-resource-postgres`, `nebula-resource-redis`)
wraps a specific driver library and implements the `Resource` trait so that
`nebula-resource` can pool, health-check, and lifecycle-manage it.

This guide walks through the complete implementation of a production-quality
adapter.

---

## Table of Contents

- [Crate Layout](#crate-layout)
- [Step 1: Define Config](#step-1-define-config)
- [Step 2: Define Instance](#step-2-define-instance)
- [Step 3: Implement Resource](#step-3-implement-resource)
- [Step 4: Implement HealthCheckable](#step-4-implement-healthcheckable)
- [Step 5: Declare Metadata](#step-5-declare-metadata)
- [Step 6: Integration Tests](#step-6-integration-tests)
- [Optional: HasResourceComponents](#optional-hasresourcecomponents)
- [Checklist](#checklist)

---

## Crate Layout

```
crates/resource-<driver>/
├── Cargo.toml
├── src/
│   ├── lib.rs          public re-exports
│   ├── config.rs       Config + validation
│   ├── instance.rs     Instance type + HealthCheckable impl
│   └── resource.rs     Resource impl
└── tests/
    └── integration.rs  acquire / recycle / health tests
```

`Cargo.toml` dependencies:

```toml
[dependencies]
nebula-resource = { path = "../../crates/resource" }
nebula-core     = { path = "../../crates/core" }
thiserror       = { workspace = true }
tokio           = { workspace = true, features = ["rt-multi-thread"] }
tracing         = { workspace = true }
# ... your driver crate
```

---

## Step 1: Define Config

```rust
// src/config.rs
use nebula_resource::{Config, Error, FieldViolation, Result};

#[derive(Debug, Clone)]
pub struct PostgresConfig {
    pub dsn: String,
    pub connect_timeout: std::time::Duration,
    pub statement_timeout: std::time::Duration,
    pub ssl_mode: SslMode,
}

#[derive(Debug, Clone, Default)]
pub enum SslMode {
    #[default]
    Prefer,
    Require,
    Disable,
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self {
            dsn: String::new(),
            connect_timeout: std::time::Duration::from_secs(10),
            statement_timeout: std::time::Duration::from_secs(30),
            ssl_mode: SslMode::Prefer,
        }
    }
}

impl Config for PostgresConfig {
    fn validate(&self) -> Result<()> {
        let mut violations = Vec::new();

        if self.dsn.is_empty() {
            violations.push(FieldViolation::new(
                "dsn",
                "must not be empty",
                "<empty>",
            ));
        }

        if self.connect_timeout.is_zero() {
            violations.push(FieldViolation::new(
                "connect_timeout",
                "must be greater than zero",
                "0s",
            ));
        }

        if !violations.is_empty() {
            return Err(Error::validation(violations));
        }

        Ok(())
    }
}
```

**Rules for `Config::validate`:**
- Return `Error::Validation` with all violations at once, not the first one.
- Validate format (non-empty DSN, valid URL scheme) but not connectivity.
  Connectivity is tested by `Resource::create`.
- Never panic. All validation is fail-fast and error-returning.

---

## Step 2: Define Instance

```rust
// src/instance.rs
use std::time::{Duration, Instant};
use nebula_resource::{HealthCheckable, HealthStatus, Result};

pub struct PostgresConnection {
    inner: tokio_postgres::Client,
    /// Prevents leaking sensitive connection strings in logs.
    _dsn: std::marker::PhantomData<()>,
}

impl PostgresConnection {
    pub(crate) fn new(client: tokio_postgres::Client) -> Self {
        Self { inner: client, _dsn: std::marker::PhantomData }
    }

    /// Execute a query via the underlying client.
    pub async fn execute(
        &self,
        statement: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> std::result::Result<u64, tokio_postgres::Error> {
        self.inner.execute(statement, params).await
    }
}

// Do NOT implement Debug to avoid accidentally logging the client state.
// impl Debug is intentionally omitted.
```

### Why no `Debug` on `Instance`?

Debug output of active connections can include internal pointers, buffer state,
or connection metadata that leaks information in logs. Omit it, or implement a
redacted version:

```rust
impl std::fmt::Debug for PostgresConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresConnection")
            .field("status", &"<redacted>")
            .finish()
    }
}
```

---

## Step 3: Implement Resource

```rust
// src/resource.rs
use nebula_core::ResourceKey;
use nebula_resource::{
    Context, Error, Resource, ResourceMetadata, ResourceCategory, Result,
};
use crate::config::PostgresConfig;
use crate::instance::PostgresConnection;

pub struct PostgresResource;

impl Resource for PostgresResource {
    type Config   = PostgresConfig;
    type Instance = PostgresConnection;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::build(
            ResourceKey::try_from("postgres").expect("valid key"),
            "PostgreSQL",
            "Pooled PostgreSQL connection via tokio-postgres",
        )
        .category(ResourceCategory::Database)
        .tag("sql")
        .tag("postgres")
        .build()
    }

    async fn create(
        &self,
        config: &PostgresConfig,
        ctx: &Context,
    ) -> Result<PostgresConnection> {
        // Respect cancellation from the caller's context.
        let connect = async {
            let (client, connection) = tokio_postgres::connect(
                &config.dsn,
                tokio_postgres::NoTls,
            )
            .await
            .map_err(|e| Error::Initialization {
                resource_key: ResourceKey::try_from("postgres").unwrap(),
                reason: e.to_string(),
                source: Some(Box::new(e)),
            })?;

            // Drive the connection in the background.
            tokio::spawn(async move {
                if let Err(e) = connection.await {
                    tracing::error!(error = %e, "postgres connection error");
                }
            });

            Ok(PostgresConnection::new(client))
        };

        if let Some(token) = ctx.cancellation.as_ref() {
            tokio::select! {
                result = connect => result,
                _ = token.cancelled() => Err(Error::Unavailable {
                    resource_key: ResourceKey::try_from("postgres").unwrap(),
                    reason: "acquire cancelled".into(),
                    retryable: true,
                }),
            }
        } else {
            connect.await
        }
    }

    async fn is_reusable(&self, instance: &PostgresConnection) -> Result<bool> {
        // A closed connection is no longer reusable.
        Ok(!instance.inner.is_closed())
    }

    async fn recycle(&self, instance: &mut PostgresConnection) -> Result<()> {
        // Discard any uncommitted transaction state.
        instance
            .execute("ROLLBACK", &[])
            .await
            .map(|_| ())
            .map_err(|e| Error::Internal {
                resource_key: ResourceKey::try_from("postgres").unwrap(),
                message: format!("recycle ROLLBACK failed: {e}"),
                source: Some(Box::new(e)),
            })
    }

    async fn cleanup(&self, instance: PostgresConnection) -> Result<()> {
        // Nothing special needed — tokio_postgres closes the connection on drop.
        drop(instance);
        Ok(())
    }

    fn declare_key() -> ResourceKey {
        ResourceKey::try_from("postgres").expect("valid key")
    }
}
```

**Important rules for `Resource` implementations:**

| Rule | Reason |
|------|--------|
| Always check `ctx.cancellation` in `create` | Prevents orphaned background tasks during shutdown. |
| Use `Error::Initialization` for `create` failures | Signals a startup problem; the pool will not retry immediately. |
| Use `Error::Internal` for `recycle` failures | The pool treats this as a transient problem and discards the instance. |
| Never store config secrets in the `Instance` | Log-safety and memory hygiene. |
| Keep `recycle` idempotent | It may be called multiple times if a circuit breaker is open. |

---

## Step 4: Implement HealthCheckable

```rust
// src/instance.rs (continued)
use nebula_resource::{HealthCheckable, HealthStatus, Result};

impl HealthCheckable for PostgresConnection {
    async fn health_check(&self) -> Result<HealthStatus> {
        let start = std::time::Instant::now();
        match self.execute("SELECT 1", &[]).await {
            Ok(_) => Ok(HealthStatus::healthy().with_latency(start.elapsed())),
            Err(e) if self.inner.is_closed() => {
                Ok(HealthStatus::unhealthy(format!("connection closed: {e}")))
            }
            Err(e) => {
                // Server reachable but query failed — degraded, not dead
                Ok(HealthStatus::degraded(
                    format!("query error: {e}"),
                    0.3, // 30% performance impact
                ))
            }
        }
    }

    fn health_check_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(15)
    }

    fn health_check_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(3)
    }
}
```

---

## Step 5: Declare Metadata

Use `declare_key` to provide a stable key that does not depend on the type name:

```rust
impl Resource for PostgresResource {
    // ...
    fn declare_key() -> ResourceKey {
        ResourceKey::try_from("postgres").expect("static literal is valid")
    }
}
```

Use `ResourceRef::of::<PostgresResource>()` in dependent crates to reference
this resource without hard-coding the key string.

---

## Step 6: Integration Tests

```rust
// tests/integration.rs
#[cfg(test)]
mod tests {
    use nebula_resource::{Context, Manager, PoolConfig, Scope};
    use crate::{PostgresConfig, PostgresResource};

    fn test_config() -> PostgresConfig {
        PostgresConfig {
            dsn: std::env::var("TEST_DATABASE_URL")
                .unwrap_or_else(|_| "postgres://localhost/test".into()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn register_and_acquire() {
        let manager = Manager::new();
        manager
            .register(PostgresResource, test_config(), PoolConfig::default())
            .expect("registration must succeed with valid config");

        let ctx = Context::new(
            Scope::Global,
            nebula_resource::WorkflowId::new(),
            nebula_resource::ExecutionId::new(),
        );

        let guard = manager
            .acquire_typed::<PostgresResource>(&ctx)
            .await
            .expect("acquire must succeed");

        // Use the connection
        let _: &crate::instance::PostgresConnection = &*guard;
    }

    #[tokio::test]
    async fn recycle_after_acquire() {
        let manager = Manager::new();
        manager
            .register(PostgresResource, test_config(), PoolConfig { max_size: 2, ..Default::default() })
            .unwrap();

        let ctx = Context::new(
            Scope::Global,
            nebula_resource::WorkflowId::new(),
            nebula_resource::ExecutionId::new(),
        );

        let guard = manager.acquire_typed::<PostgresResource>(&ctx).await.unwrap();
        drop(guard); // Returns to pool

        // Pool should have 1 idle connection after release
        let pool = manager.pool::<PostgresResource>().unwrap();
        let stats = pool.stats().unwrap();
        assert_eq!(stats.idle, 1);
        assert_eq!(stats.active, 0);
    }

    #[tokio::test]
    async fn taint_skips_recycle() {
        let manager = Manager::new();
        manager
            .register(PostgresResource, test_config(), PoolConfig::default())
            .unwrap();

        let ctx = Context::new(
            Scope::Global,
            nebula_resource::WorkflowId::new(),
            nebula_resource::ExecutionId::new(),
        );

        let mut guard = manager.acquire_typed::<PostgresResource>(&ctx).await.unwrap();
        guard.taint(); // Signal the connection is broken
        drop(guard);   // cleanup called, not recycle

        let pool = manager.pool::<PostgresResource>().unwrap();
        let stats = pool.stats().unwrap();
        assert_eq!(stats.idle, 0);
        assert_eq!(stats.destroyed, 1);
    }
}
```

---

## Optional: HasResourceComponents

If your resource depends on a credential (e.g. a database password stored in
`nebula-credential`), implement `HasResourceComponents`:

```rust
use nebula_resource::{HasResourceComponents, ResourceComponents, ResourceRef};
use nebula_credential::types::DatabaseCredential;  // example type

impl HasResourceComponents for PostgresResource {
    fn components() -> ResourceComponents {
        ResourceComponents::new()
            .credential::<DatabaseCredential>("postgres-db-cred")
    }
}
```

The engine uses `components()` to pre-validate credentials exist before the
pool is created, and to wire up credential rotation events to pool recycling.

---

## Checklist

Before publishing an adapter crate:

- [ ] `Config::validate` rejects all invalid inputs with `Error::Validation`.
- [ ] `Resource::create` checks `ctx.cancellation` via `tokio::select!`.
- [ ] `Resource::create` returns `Error::Initialization` on connection failure.
- [ ] `Resource::recycle` is idempotent and returns `Error::Internal` on failure.
- [ ] `Resource::cleanup` is infallible (swallow or log errors, never panic).
- [ ] `Resource::metadata` returns a canonical `ResourceKey` matching `declare_key`.
- [ ] `Resource::declare_key` uses a static string literal, not a derived name.
- [ ] `Instance` does not implement `Debug` or implements a redacted version.
- [ ] `HealthCheckable` is implemented on `Instance` with appropriate interval/timeout.
- [ ] Integration tests cover: register, acquire, recycle, taint/cleanup.
- [ ] `#![forbid(unsafe_code)]` is set in `lib.rs`.
- [ ] No credentials or secrets appear in log output.
