# nebula-resource — Writing Adapter Crates

An adapter crate (e.g. `nebula-resource-postgres`, `nebula-resource-redis`) wraps a
specific driver library and implements the v2 `Resource` trait so that `nebula-resource`
can pool, health-check, and lifecycle-manage it.

This guide walks through a complete pseudo-Postgres example. Because a real Postgres driver
is not a workspace dependency, all code blocks use `ignore` to keep them out of `cargo test
--doc`. Replace `ignore` with `no_run` once you add the driver crate.

---

## Overview

An adapter crate owns three things:

1. A **config struct** — operational parameters (host, port, timeouts). No secrets.
2. A **resource struct** — implements `Resource` with 5 associated types and 4 lifecycle
   methods. This is the factory that creates and tears down connections.
3. A **topology impl** — `Pooled`, `Resident`, or another topology trait that tells the
   manager how to hand out leases. Most database adapters use `Pooled`.

The manager calls `Resource::create` when it needs a new instance, `Pooled::recycle` when
an instance is returned, and `Resource::destroy` when it is discarded.

---

## Cargo.toml

```toml
[package]
name = "nebula-resource-postgres"
version = "0.1.0"
edition = "2024"

[dependencies]
nebula-resource = { path = "../../crates/resource" }
nebula-core     = { path = "../../crates/core" }
thiserror       = { workspace = true }
tokio           = { workspace = true, features = ["rt-multi-thread"] }
tracing         = { workspace = true }
# your driver:
# tokio-postgres = "0.7"
```

---

## Step 1: Define Config

`ResourceConfig` holds operational parameters. Implement `validate()` to reject bad inputs
before the manager tries to create any connections, and `fingerprint()` to enable config
hot-reload without recreating healthy instances.

```rust,ignore
// src/config.rs
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use nebula_resource::{Error, ResourceConfig};

#[derive(Debug, Clone, Hash)]
pub struct PostgresConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub connect_timeout_secs: u64,
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self {
            host: "localhost".into(),
            port: 5432,
            database: "postgres".into(),
            connect_timeout_secs: 10,
        }
    }
}

impl ResourceConfig for PostgresConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.host.is_empty() {
            return Err(Error::permanent("postgres: host must not be empty"));
        }
        if self.connect_timeout_secs == 0 {
            return Err(Error::permanent("postgres: connect_timeout_secs must be > 0"));
        }
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
}
```

Two rules for `validate`:
- Reject every bad field, not just the first. Build up errors and return a combined message.
- Validate format (non-empty host, valid port range), not connectivity. Connectivity is the
  job of `Resource::create`.

`fingerprint` must be deterministic: same config fields → same fingerprint. When the
manager detects a fingerprint change after `reload_config`, it evicts idle pool instances
that were created with the old config.

---

## Step 2: Define Error

Your resource's `Error` type must implement `Into<nebula_resource::Error>`. The framework
uses `ErrorKind` on the converted error to decide whether to retry (`Transient`), give up
(`Permanent`), or backoff (`Exhausted`).

### Option A: `ClassifyError` derive (recommended)

`ClassifyError` generates `From<YourError> for nebula_resource::Error` from per-variant
`#[classify(...)]` annotations.

```rust,ignore
// src/error.rs
use nebula_resource::ClassifyError;

#[derive(Debug, thiserror::Error, ClassifyError)]
pub enum PostgresError {
    #[error("connection failed: {0}")]
    #[classify(transient)]
    Connect(String),

    #[error("authentication failed: {0}")]
    #[classify(permanent)]
    Auth(String),

    #[error("rate limited, retry after {retry_after_secs}s")]
    #[classify(exhausted(retry_after_secs))]
    RateLimited { retry_after_secs: u64 },
}
```

### Option B: Manual `From` impl

Use this when you do not control the error type (e.g., wrapping a third-party driver error).

```rust,ignore
// src/error.rs
use nebula_resource::{Error, ErrorKind};

#[derive(Debug, thiserror::Error)]
pub enum PostgresError {
    #[error("connection failed: {0}")]
    Connect(String),

    #[error("authentication failed: {0}")]
    Auth(String),
}

impl From<PostgresError> for Error {
    fn from(e: PostgresError) -> Self {
        match e {
            PostgresError::Connect(msg) => Error::transient(msg),
            PostgresError::Auth(msg) => Error::permanent(msg),
        }
    }
}
```

---

## Step 3: Implement Resource

`Resource` is the factory. It describes all five associated types and implements the four
lifecycle methods. Only `create` is required; `check`, `shutdown`, and `destroy` have
no-op defaults.

```rust,ignore
// src/resource.rs
use nebula_core::ResourceKey;
use nebula_credential::NoCredential;
use nebula_resource::{resource_key, Resource, ResourceConfig, ResourceMetadata};
use nebula_resource::context::ResourceContext;

use crate::config::PostgresConfig;
use crate::error::PostgresError;

/// A fake connection handle — replace with your driver's client type.
#[derive(Clone)]
pub struct PgConnection {
    pub closed: bool,
}

/// The resource factory. Stateless — put shared config in `PostgresConfig`.
#[derive(Clone)]
pub struct PostgresResource;

impl Resource for PostgresResource {
    /// Operational config — no secrets.
    type Config = PostgresConfig;
    /// The live connection handle.
    type Runtime = PgConnection;
    /// What callers receive from `acquire_pooled`. Often identical to Runtime.
    type Lease = PgConnection;
    /// Resource-specific error, convertible to `nebula_resource::Error`.
    type Error = PostgresError;
    /// Use `NoCredential` for unauthenticated resources; supply a credential type otherwise.
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        resource_key!("postgres")
    }

    /// Opens a new connection. Called by the pool when it needs more capacity.
    ///
    /// Map your driver's connection error to `PostgresError::Connect` (transient)
    /// or `PostgresError::Auth` (permanent) so the framework knows whether to retry.
    async fn create(
        &self,
        config: &PostgresConfig,
        _scheme: &(),
        _ctx: &ResourceContext,
    ) -> Result<PgConnection, PostgresError> {
        // Replace with: tokio_postgres::connect(&dsn, NoTls).await
        let _ = config;
        Ok(PgConnection { closed: false })
    }

    /// Pings the connection. Called when `test_on_checkout = true` in PoolConfig.
    async fn check(&self, runtime: &PgConnection) -> Result<(), PostgresError> {
        if runtime.closed {
            return Err(PostgresError::Connect("connection is closed".into()));
        }
        // Replace with: client.simple_query("").await
        Ok(())
    }

    /// Drops the connection cleanly. The default (just drops it) is fine for
    /// drivers that close on drop, but implement this if you need to send a
    /// disconnect message.
    async fn destroy(&self, _runtime: PgConnection) -> Result<(), PostgresError> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        let mut m = ResourceMetadata::from_key(&Self::key());
        m.name = "PostgreSQL".into();
        m.description = Some("Pooled PostgreSQL connection".into());
        m.tags = vec!["sql".into(), "postgres".into()];
        m
    }
}
```

---

## Step 4: Pick and Implement Topology

### Which topology?

| Topology | Use when |
|----------|----------|
| `Pooled` | Stateful connections (databases, gRPC channels). The manager maintains N instances and checks them out one-at-a-time. |
| `Resident` | Stateless or internally-pooled clients (`reqwest::Client`, AWS SDK). One instance, cloned on acquire. |

Most database adapters use `Pooled`. HTTP client adapters use `Resident`.

### Pooled implementation

`Pooled` adds three hooks on top of `Resource`:

- `is_broken` — called synchronously in the `Drop` path to decide whether to return an
  instance to the idle queue or destroy it immediately. Must be O(1), no I/O.
- `recycle` — called asynchronously when an instance is returned. Use `InstanceMetrics`
  to retire long-lived or error-prone instances.
- `prepare` — called after checkout, before the caller receives the lease. Use this for
  session setup (e.g., `SET search_path`).

```rust,ignore
// src/resource.rs (continued)
use nebula_resource::topology::pooled::{BrokenCheck, InstanceMetrics, Pooled, RecycleDecision};

impl Pooled for PostgresResource {
    /// Sync broken check — runs in the Drop path. Check a cheap flag, not the network.
    fn is_broken(&self, runtime: &PgConnection) -> BrokenCheck {
        if runtime.closed {
            BrokenCheck::Broken("connection closed".into())
        } else {
            BrokenCheck::Healthy
        }
    }

    /// Decide whether to keep or destroy an instance being returned to the pool.
    ///
    /// Drop instances that have lived too long or seen too many errors.
    async fn recycle(
        &self,
        runtime: &PgConnection,
        metrics: &InstanceMetrics,
    ) -> Result<RecycleDecision, PostgresError> {
        if runtime.closed {
            return Ok(RecycleDecision::Drop);
        }
        // Retire instances older than 30 minutes.
        if metrics.age() > std::time::Duration::from_secs(1800) {
            return Ok(RecycleDecision::Drop);
        }
        // Replace with: client.simple_query("ROLLBACK").await?;
        Ok(RecycleDecision::Keep)
    }
}
```

---

## Step 5: Register and Acquire

`Manager::register_pooled` is the zero-boilerplate path for unauthenticated pooled
resources (no credentials). It sets `scope = Global`, no resilience, no recovery gate.

```rust,ignore
// in your application bootstrap
use nebula_resource::{Manager, PoolConfig};
use nebula_resource_postgres::{PostgresConfig, PostgresResource};

let manager = Manager::new();

manager.register_pooled(
    PostgresResource,
    PostgresConfig {
        host: "db.example.com".into(),
        port: 5432,
        database: "myapp".into(),
        connect_timeout_secs: 10,
    },
    PoolConfig {
        max_size: 10,
        ..PoolConfig::default()
    },
)?;
```

To acquire a connection in an action or test:

```rust,ignore
use nebula_resource::{AcquireOptions, Manager, ResourceContext};
use nebula_core::ExecutionId;

let ctx = ResourceContext::new(ExecutionId::new());
let handle = manager
    .acquire_pooled::<PostgresResource>(&(), &ctx, &AcquireOptions::default())
    .await?;

// Use the connection via the RAII handle — it returns to the pool on drop.
let conn: &PgConnection = &*handle;
```

If you need credentials or a recovery gate, use `Manager::register` directly and pass the
full arguments.

---

## Step 6: Integration Tests

Tests should not require a real database. Use a mock `PgConnection` with atomic flags to
exercise the lifecycle without network I/O.

```rust,ignore
// tests/integration.rs
use std::sync::Arc;
use nebula_core::ExecutionId;
use nebula_resource::{AcquireOptions, Manager, PoolConfig, ResourceContext};
use nebula_resource_postgres::{PostgresConfig, PostgresResource};

#[tokio::test]
async fn register_and_acquire() {
    let manager = Manager::new();
    manager
        .register_pooled(
            PostgresResource,
            PostgresConfig::default(),
            PoolConfig::default(),
        )
        .expect("valid config must register without error");

    let ctx = ResourceContext::new(ExecutionId::new());
    let handle = manager
        .acquire_pooled::<PostgresResource>(&(), &ctx, &AcquireOptions::default())
        .await
        .expect("acquire must succeed after registration");

    // Verify the topology tag to ensure the correct path was taken.
    assert_eq!(handle.topology_tag(), nebula_resource::TopologyTag::Pool);

    // Drop returns the instance to the pool automatically.
    drop(handle);
}
```

---

## Checklist

Before publishing an adapter crate:

- [ ] `ResourceConfig::validate` rejects all invalid inputs without panicking.
- [ ] `ResourceConfig::fingerprint` is deterministic: equal configs produce equal fingerprints.
- [ ] `Resource::create` maps driver errors to `Transient` vs `Permanent` `ErrorKind`
      so the framework knows whether to retry.
- [ ] `Resource::key()` returns a stable string literal, not a derived type name.
- [ ] `Pooled::is_broken` is O(1) and performs no I/O — it runs in `Drop`.
- [ ] `Pooled::recycle` rolls back any open transaction before returning `Keep`.
- [ ] `Resource::Credential = NoCredential` unless the adapter genuinely binds to a credential.
- [ ] `Runtime` does not implement `Debug`, or implements a redacted version that omits
      connection strings and internal buffer state.
- [ ] Integration tests use a mock/in-memory runtime, not a live network service.
- [ ] `#![forbid(unsafe_code)]` is set in `lib.rs`.
