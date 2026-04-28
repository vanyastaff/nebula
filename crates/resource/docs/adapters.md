# nebula-resource — Writing Adapter Crates

An adapter crate (e.g. `nebula-resource-postgres`) wraps a specific driver
library and implements the v2 `Resource` trait so that `nebula-resource`
can pool, health-check, and lifecycle-manage it. The walkthrough uses
fictional `PgConnection` / `tokio_postgres` types — substitute as you go.
Signatures for `nebula_resource::*` are real (cross-check against
[`api-reference.md`](api-reference.md)); all code blocks are `rust,ignore`.

---

## Overview

An adapter crate owns three things:

1. A **config struct** — operational parameters (host, port, timeouts).
   No secrets. Implements `ResourceConfig` (super-bound:
   `nebula_schema::HasSchema`).
2. A **resource struct** — implements `Resource` with five associated
   types (`Config`, `Runtime`, `Lease`, `Error`, `Credential`) plus the
   lifecycle methods. The factory.
3. A **topology impl** — `Pooled`, `Resident`, `Service`, `Transport`,
   or `Exclusive`. Most database adapters use `Pooled`.

**Credential binding (ADR-0036).** If your resource binds to a credential,
declare a `Credential` type and override `on_credential_refresh` /
`on_credential_revoke`. If not, use `type Credential = NoCredential` and
let the defaults stand. The reverse-index is populated at registration via
`RegisterOptions { credential_id: Some(..), .. }`; every no-credential
register helper (`register_pooled`, `register_pooled_with`, etc.) is
bounded `where R: Resource<Credential = NoCredential>`.

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
nebula-schema   = { path = "../../crates/schema" }
thiserror = { workspace = true }
tokio = { workspace = true, features = ["rt-multi-thread"] }
# tokio-postgres = "0.7"   # your driver
```

---

## Step 1: Define Config

`ResourceConfig` extends `nebula_schema::HasSchema + Send + Sync + Clone +
'static`. The `HasSchema` super-bound exists so resource configs can be
introspected by the catalog and workflow editor — derive it from the
struct shape (recommended) or stub it out for in-process / test fixtures.

### Option A: `#[derive(Schema)]` — recommended

`nebula_schema::Schema` is a derive macro that generates a real schema
from struct fields. Field-level `#[param(...)]` attributes set labels,
descriptions, and other catalog metadata. Use this whenever the config
is meant to be visible in a UI:

```rust,ignore
// src/config.rs
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use nebula_resource::{Error, ResourceConfig};
use nebula_schema::Schema;
use serde::Deserialize;

#[derive(Debug, Clone, Hash, Schema, Deserialize)]
pub struct PostgresConfig {
    #[param(label = "Host")]
    pub host: String,
    #[param(label = "Port", default = 5432)]
    pub port: u16,
    #[param(label = "Database")]
    pub database: String,
    #[param(label = "Connect timeout (s)", default = 10)]
    pub connect_timeout_secs: u64,
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

### Option B: explicit empty schema — for in-process / test fixtures

If the config is never exposed to a catalog (e.g., test scaffolding,
internal-only adapters), implement `HasSchema` directly with an empty
schema. No macros, no `serde` requirement:

```rust,ignore
use nebula_schema::{HasSchema, ValidSchema};

#[derive(Debug, Clone, Hash)]
pub struct PostgresConfig { /* ... */ }

impl HasSchema for PostgresConfig {
    fn schema() -> ValidSchema {
        ValidSchema::empty()
    }
}
```

(`nebula-schema` also exposes a `nebula_schema::impl_empty_has_schema!`
macro that emits the same boilerplate; either form is acceptable.)

### `validate` and `fingerprint` (apply to both options)

`validate` checks format and bounds — connectivity belongs in
`Resource::create`. `fingerprint` must be deterministic; on a
`reload_config` fingerprint change, the manager evicts idle pool instances
created with the old config.

---

## Step 2: Define Error

Your resource's `Error` type must implement
`Into<nebula_resource::Error>`. The framework reads `ErrorKind` on the
converted error to decide retry (`Transient`) / give-up (`Permanent`) /
back-off (`Exhausted`).

### Option A: `ClassifyError` derive (recommended)

```rust,ignore
// src/error.rs
use nebula_resource::ClassifyError;

#[derive(Debug, thiserror::Error, ClassifyError)]
pub enum PostgresError {
    #[error("connection failed: {0}")] #[classify(transient)] Connect(String),
    #[error("authentication failed: {0}")] #[classify(permanent)] Auth(String),
    #[error("rate limited")] #[classify(exhausted, retry_after = "30s")] RateLimited,
}
```

Supported kinds: `transient`, `permanent`, `exhausted` (with optional
`retry_after = "30s" / "5m" / "1h" / "500ms"`), `backpressure`,
`cancelled`.

### Option B: Manual `From` impl

Use this when you do not control the error type (e.g., wrapping a
third-party driver error).

```rust,ignore
use nebula_resource::Error;

#[derive(Debug, thiserror::Error)]
pub enum PostgresError {
    #[error("connection failed: {0}")] Connect(String),
    #[error("authentication failed: {0}")] Auth(String),
}

impl From<PostgresError> for Error {
    fn from(e: PostgresError) -> Self {
        match e {
            PostgresError::Connect(msg) => Error::transient(msg),
            PostgresError::Auth(msg)    => Error::permanent(msg),
        }
    }
}
```

---

## Step 3: Implement Resource

Only `create` is required; `check`, `shutdown`, `destroy`,
`on_credential_refresh`, and `on_credential_revoke` have sensible defaults.

```rust,ignore
// src/resource.rs
use std::future::Future;
use nebula_core::ResourceKey;
use nebula_resource::{
    resource_key, Credential, NoCredential, Resource, ResourceMetadata,
};
use nebula_resource::context::ResourceContext;

use crate::config::PostgresConfig;
use crate::error::PostgresError;

#[derive(Clone)]
pub struct PgConnection { pub closed: bool }

#[derive(Clone)]
pub struct PostgresResource;

impl Resource for PostgresResource {
    type Config = PostgresConfig;
    type Runtime = PgConnection;
    type Lease = PgConnection;
    type Error = PostgresError;
    type Credential = NoCredential;  // unauthenticated; see sidebar for credential-bound

    fn key() -> ResourceKey { resource_key!("postgres") }

    fn create(
        &self,
        config: &PostgresConfig,
        _scheme: &<Self::Credential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<PgConnection, PostgresError>> + Send {
        // Replace with: tokio_postgres::connect(&dsn, NoTls).await
        let _ = config;
        async { Ok(PgConnection { closed: false }) }
    }

    async fn check(&self, runtime: &PgConnection) -> Result<(), PostgresError> {
        if runtime.closed {
            return Err(PostgresError::Connect("connection is closed".into()));
        }
        Ok(())
    }

    fn metadata() -> ResourceMetadata { ResourceMetadata::from_key(&Self::key()) }

    // `shutdown` and `destroy` defaults (no-op + drop) are correct here.
}
```

**Credential-bound sidebar.** For an authenticated resource that binds to
a rotating credential, set `type Credential = MyDbCredential` and override
`on_credential_refresh` to perform the blue-green pool swap from
credential Tech Spec §15.7. The default no-op is correct for
unauthenticated resources.

```rust,ignore
use nebula_resource::{CredentialContext, SchemeGuard};

// inside `impl Resource for PostgresResource` with `type Credential = MyDbCredential`:
fn on_credential_refresh<'a>(
    &self,
    new_scheme: SchemeGuard<'a, Self::Credential>,
    ctx: &'a CredentialContext,
) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a {
    // Build a fresh pool from `new_scheme`, atomically swap into your
    // `Arc<RwLock<Pool>>`, let RAII drain old handles. `new_scheme` MUST
    // NOT be retained past this future per PRODUCT_CANON.md §12.5 — the
    // shared `'a` lifetime is the compile-time barrier.
    async move { let _ = (new_scheme, ctx); Ok(()) }
}
```

---

## Step 4: Pick and Implement Topology

| Topology    | Use when                                                                              |
|-------------|---------------------------------------------------------------------------------------|
| `Pooled`    | Stateful connections (DBs, gRPC channels). N instances, one-at-a-time checkout.       |
| `Resident`  | Stateless or internally-pooled clients (`reqwest::Client`, AWS SDK). Cloned on acquire. |
| `Service`   | Token-bounded shared clients with `TokenMode` admission. Rate-limited service clients. |
| `Transport` | Connection-oriented — hands out a frame channel per acquire (e.g., long-lived TCP).    |
| `Exclusive` | Mutex-style — one acquirer at a time, no internal pooling.                            |

### Pooled implementation

`Pooled` adds three hooks on top of `Resource`:

- `is_broken` — sync, O(1), runs in `Drop`. Check a cheap flag, **no I/O.**
- `recycle` — async, runs on return. Returns `Keep` / `Drop` based on
  `InstanceMetrics` (age, error count, checkout count).
- `prepare` — async, runs after checkout, before the lease reaches the
  caller. Use for per-checkout setup (`SET search_path`); default no-op.

```rust,ignore
// src/resource.rs (continued)
use nebula_resource::topology::pooled::{
    BrokenCheck, InstanceMetrics, Pooled, RecycleDecision,
};

impl Pooled for PostgresResource {
    fn is_broken(&self, runtime: &PgConnection) -> BrokenCheck {
        if runtime.closed { BrokenCheck::Broken("connection closed".into()) }
        else { BrokenCheck::Healthy }
    }

    async fn recycle(
        &self,
        runtime: &PgConnection,
        metrics: &InstanceMetrics,
    ) -> Result<RecycleDecision, PostgresError> {
        if runtime.closed { return Ok(RecycleDecision::Drop); }
        // Retire instances older than 30 minutes.
        if metrics.age() > std::time::Duration::from_secs(1800) {
            return Ok(RecycleDecision::Drop);
        }
        // Replace with: client.simple_query("ROLLBACK").await?;
        Ok(RecycleDecision::Keep)
    }

    // prepare(): default no-op; override for per-checkout session setup
    // (e.g., `SET search_path`, `SET application_name`).
}
```

---

## Step 5: Register and Acquire

Three paths — most adapters need only the first.

1. **`register_pooled` + `acquire_pooled_default`** — zero-boilerplate,
   bounded `where R: Resource<Credential = NoCredential>`.
   `acquire_pooled_default` passes `&()` as the scheme internally.
2. **`register_pooled_with` + `acquire_pooled_default`** — pass
   `RegisterOptions` for a non-default `scope`, an `AcquireResilience`
   profile, or a `RecoveryGate`. Still `NoCredential`-bound.
3. **`Manager::register` + `acquire_pooled`** — full positional path for
   credential-bearing resources. Build a
   `TopologyRuntime::Pool(PoolRuntime::<R>::new(...))`, supply
   `credential_id` directly, and call `acquire_pooled` with a borrowed
   scheme produced by the credential subsystem.

```rust,ignore
// path 1: simplest
use nebula_resource::{Manager, PoolConfig};
use nebula_resource_postgres::{PostgresConfig, PostgresResource};

let manager = Manager::new();
manager.register_pooled(
    PostgresResource,
    PostgresConfig { host: "db.example.com".into(), port: 5432,
                     database: "myapp".into(), connect_timeout_secs: 10 },
    PoolConfig { max_size: 10, ..PoolConfig::default() },
)?;
```

```rust,ignore
// path 2: with options (still NoCredential).
use nebula_resource::{
    AcquireResilience, AcquireRetryConfig, PoolConfig, RegisterOptions,
};

let mut opts = RegisterOptions::default();  // scope = Global
opts.resilience = Some(AcquireResilience {
    timeout: Some(std::time::Duration::from_secs(5)),
    retry: Some(AcquireRetryConfig::default()),
});

manager.register_pooled_with(
    PostgresResource, PostgresConfig::default(), PoolConfig::default(), opts,
)?;
```

To acquire:

```rust,ignore
use nebula_core::scope::Scope;
use nebula_resource::AcquireOptions;
use nebula_resource::context::ResourceContext;
use tokio_util::sync::CancellationToken;

let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
let handle = manager
    .acquire_pooled_default::<PostgresResource>(&ctx, &AcquireOptions::default())
    .await?;
let conn: &PgConnection = &*handle;  // RAII — returns to the pool on drop.
```

---

## Step 6: Integration Tests

Tests should not require a real database — use a mock `PgConnection`.

```rust,ignore
// tests/integration.rs
use nebula_core::scope::Scope;
use nebula_resource::{AcquireOptions, Manager, PoolConfig, TopologyTag};
use nebula_resource::context::ResourceContext;
use nebula_resource_postgres::{PostgresConfig, PostgresResource};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn register_and_acquire() {
    let manager = Manager::new();
    manager.register_pooled(
        PostgresResource,
        PostgresConfig::default(),
        PoolConfig::default(),
    ).expect("valid config must register");

    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let handle = manager
        .acquire_pooled_default::<PostgresResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire must succeed");

    assert_eq!(handle.topology_tag(), TopologyTag::Pool);
    drop(handle); // returns to pool
}
```

---

## Checklist

- [ ] `ResourceConfig::validate` rejects invalid inputs without panic; `fingerprint` is deterministic.
- [ ] `Resource::create` maps driver errors to `Transient` vs `Permanent` `ErrorKind`.
- [ ] `Resource::key` returns a stable string literal (use `resource_key!`).
- [ ] `Pooled::is_broken` is O(1) and performs no I/O — runs in `Drop`.
- [ ] `Pooled::recycle` rolls back any open transaction before returning `Keep`.
- [ ] `Resource::Credential = NoCredential` unless the adapter binds to a credential.
- [ ] If credential-bound: `on_credential_refresh` performs the blue-green
      swap (credential Tech Spec §15.7); `on_credential_revoke` ensures
      no further authenticated traffic on the revoked credential.
- [ ] `Runtime` does not implement `Debug`, or implements a redacted variant
      that omits connection strings and internal buffer state.
- [ ] Integration tests use a mock runtime, not a live network service.
- [ ] `#![forbid(unsafe_code)]` is set in `lib.rs`.
