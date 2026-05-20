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
   `HasSchema`, re-exported as `nebula_resource::HasSchema`).
2. A **resource struct** — implements `Resource` with four associated
   types (`Config`, `Runtime`, `Lease`, `Error`) plus the lifecycle
   methods. The factory.
3. A **topology impl** — [`Pooled`], [`Resident`], or [`Bounded`] (with
   a sealed `Cap` typestate: `Unbounded` / `Capped<N>` / `Exclusive`).
   Most database adapters use `Pooled`; HTTP clients use `Resident`;
   OAuth / gRPC / file-lock patterns use `Bounded` with the matching cap.

**Credential binding.** If your resource binds to a credential, declare a slot field
`#[credential(key = "...")] auth: SlotCell<CredentialGuard<C>>` and read the
resolved guard via the derive-emitted `self.auth_slot()` accessor inside
`create` (and the rotation hooks). Connection-bound resources override
`on_credential_refresh` / `on_credential_revoke` to act on the runtime's
interior mutability. If your resource needs no credential, simply declare
**no** `#[credential]` field — there is no `Credential` associated type and
no `NoCredential` opt-out anymore. Credential bindings are no longer threaded
through registration: the engine resolves each declared slot and binds it on
`&self` before `Resource::create`.

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
thiserror = { workspace = true }
tokio = { workspace = true, features = ["rt-multi-thread"] }
# tokio-postgres = "0.7"   # your driver
```

`nebula-resource` re-exports `HasSchema`, `Schema`, `ValidSchema`, and
the `impl_empty_has_schema!` convenience macro from `nebula-schema`, so
adapter `Cargo.toml`s do not need a direct `nebula-schema` dependency
just to satisfy `ResourceConfig`'s super-bound. Add `nebula-schema`
explicitly only if you reach for schema APIs the resource crate does not
re-export (custom builders, validators, etc.).

---

## Step 1: Define Config

`ResourceConfig` extends `HasSchema + Send + Sync + Clone + 'static`
(canonical path: `nebula_schema::HasSchema`; re-exported as
`nebula_resource::HasSchema`). The `HasSchema` super-bound exists so
resource configs can be introspected by the catalog and workflow editor
— derive it from the struct shape (recommended) or stub it out for
in-process / test fixtures.

### Option A: `#[derive(Schema)]` — recommended

`nebula_resource::Schema` (re-exported from `nebula-schema`) is a derive
macro that generates a real schema from struct fields. Field-level
`#[field(...)]` attributes set labels, descriptions, and other catalog
metadata. Use this whenever the config is meant to be visible in a UI:

```rust,ignore
// src/config.rs
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use nebula_resource::{Error, ResourceConfig, Schema};
use serde::Deserialize;

#[derive(Debug, Clone, Hash, Schema, Deserialize)]
pub struct PostgresConfig {
    #[field(label = "Host")]
    pub host: String,
    #[field(label = "Port", default = 5432)]
    pub port: u16,
    #[field(label = "Database")]
    pub database: String,
    #[field(label = "Connect timeout (s)", default = 10)]
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
use nebula_resource::{HasSchema, ValidSchema};

#[derive(Debug, Clone, Hash)]
pub struct PostgresConfig { /* ... */ }

impl HasSchema for PostgresConfig {
    fn schema() -> ValidSchema {
        ValidSchema::empty()
    }
}
```

(`nebula-resource` also re-exports `impl_empty_has_schema!`, so
`nebula_resource::impl_empty_has_schema!(PostgresConfig);` emits the
same boilerplate; either form is acceptable.)

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
use nebula_resource::{resource_key, Resource, ResourceMetadata};
use nebula_resource::context::ResourceContext;

use crate::config::PostgresConfig;
use crate::error::PostgresError;

#[derive(Clone)]
pub struct PgConnection { pub closed: bool }

// Unauthenticated adapter: no `#[credential]` field. See the sidebar for
// the credential-bound shape.
#[derive(Clone)]
pub struct PostgresResource;

impl Resource for PostgresResource {
    type Config = PostgresConfig;
    type Runtime = PgConnection;
    type Lease = PgConnection;
    type Error = PostgresError;

    fn key() -> ResourceKey { resource_key!("postgres") }

    fn create(
        &self,
        config: &PostgresConfig,
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

**Credential-bound sidebar.** For an authenticated resource that binds to a
rotating credential, declare a `#[credential(key = "...")]` slot field and
read the resolved guard via the derive-emitted `self.<field>_slot()`
accessor. Override `on_credential_refresh` to perform the blue-green pool
swap acting on the runtime's own interior mutability. The hooks take
`&self`, the rotated `slot_name`, and `&Self::Runtime` — no scheme is ever
handed to the impl; the engine binds the rotated guard into the slot cell
before calling the hook.

```rust,ignore
use std::sync::Arc;
use arc_swap::ArcSwap;
use nebula_credential::CredentialGuard;
use nebula_resource::SlotCell;

// MyDbCredential: C; PgPool: built from a resolved guard.
pub struct PostgresResource {
    #[credential(key = "db")]
    auth: SlotCell<CredentialGuard<MyDbCredential>>,
}

// Runtime owns the swappable pool.
pub struct PgRuntime {
    pool: ArcSwap<PgPool>,
}

// inside `impl Resource for PostgresResource`:
fn create(
    &self,
    config: &PostgresConfig,
    _ctx: &ResourceContext,
) -> impl Future<Output = Result<PgRuntime, PostgresError>> + Send {
    // Read the resolved guard the engine bound before `create`.
    let guard = self.auth_slot(); // Option<Arc<CredentialGuard<MyDbCredential>>>
    async move {
        let guard = guard.ok_or_else(|| PostgresError::Auth("db slot unbound".into()))?;
        let pool = build_pool(config, &guard).await?;
        Ok(PgRuntime { pool: ArcSwap::from_pointee(pool) })
    }
}

fn on_credential_refresh(
    &self,
    slot_name: &str,
    runtime: &PgRuntime,
) -> impl Future<Output = Result<(), Self::Error>> + Send {
    // The engine has already swapped the rotated guard into `self`'s slot
    // cell; rebuild from it and atomically swap, letting RAII drain old
    // handles. Multi-slot resources branch on `slot_name`.
    let guard = self.auth_slot();
    async move {
        if slot_name == "db" {
            let guard = guard.ok_or_else(|| PostgresError::Auth("db slot unbound".into()))?;
            let fresh = build_pool_from_guard(&guard).await?;
            runtime.pool.store(Arc::new(fresh));
        }
        Ok(())
    }
}
```

---

## Step 4: Pick and Implement Topology

| Topology               | Use when                                                                                                  |
|------------------------|-----------------------------------------------------------------------------------------------------------|
| `Pooled`               | Stateful connections (DBs, gRPC channels). N instances, one-at-a-time checkout.                           |
| `Resident`             | Stateless or internally-pooled clients (`reqwest::Client`, AWS SDK). `Arc::clone` on acquire.             |
| `Bounded<Unbounded>`   | Token-gated services (OAuth, fan-out APIs). Long-lived runtime, no per-acquire release.                   |
| `Bounded<Capped<N>>`   | Multiplexed sessions over one shared connection (SSH, gRPC channel). Tracked release via `BoundedRelease`. |
| `Bounded<Exclusive>`   | Mutex-style — one acquirer at a time, reset-on-release ordering.                                          |

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

Three paths — most adapters need only the first. None take a `scheme`
argument; a credential-bound resource carries its resolved guard in its
`#[credential]` slot field (bound by the engine before `create`), so the
register/acquire surface is identical whether or not the resource binds a
credential.

Registration goes through **one funnel**:
`Manager::register::<R>(spec: RegistrationSpec<R>)`. The per-topology
`register_<topo>[_with]` shorthands and the multi-step delegation chain
were removed (the manager-side `AcquireResilience` wrapper was
dropped; the `register_*` shorthand family followed).
`RegistrationSpec<R>` is a plain struct with public fields and no
builder.

For the multi-tenant case, pin distinct resolved credentials to
distinct registry rows by building a `SlotIdentity::Structural` (via
`SlotIdentity::from_bindings`) and acquire through
`acquire_pooled_for_identity`.

```rust,ignore
use nebula_resource::{
    AcquireOptions, Manager, PoolRuntime, RegistrationSpec, ScopeLevel,
    TopologyRuntime, dedup::SlotIdentity,
    topology::pooled::config::Config as PoolConfig,
};
use nebula_resource_postgres::{PostgresConfig, PostgresResource};

let manager = Manager::new();
let pg_config = PostgresConfig {
    host: "db.example.com".into(),
    port: 5432,
    database: "myapp".into(),
    connect_timeout_secs: 10,
};
let pool_rt = PoolRuntime::<PostgresResource>::try_new(
    PoolConfig { max_size: 10, ..PoolConfig::default() },
    pg_config.fingerprint(),
)?;

manager.register(RegistrationSpec {
    resource: PostgresResource,
    config: pg_config,
    scope: ScopeLevel::Global,
    slot_identity: SlotIdentity::Unbound,
    topology: TopologyRuntime::Pool(pool_rt),
    acquire: Manager::erased_acquire_pooled_for::<PostgresResource>(),
    recovery_gate: None,  // attach an Arc<RecoveryGate> for thundering-herd protection
})?;
```

To acquire:

```rust,ignore
use nebula_core::scope::Scope;
use nebula_resource::{AcquireOptions, context::ResourceContext};
use tokio_util::sync::CancellationToken;

let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
let handle = manager
    .acquire_pooled::<PostgresResource>(&ctx, &AcquireOptions::default())
    .await?;
let conn: &PgConnection = &*handle;  // RAII — returns to the pool on drop.
```

Retry / timeout composition lives one layer up (action handler / engine
activity) — peer Rust pools (sqlx, deadpool, bb8) all ship
acquire-timeout only, and per-topology configs carry their own
`create_timeout`. Add a `RecoveryGate` only when you need thundering-
herd protection on a flapping backend.

---

## Step 6: Integration Tests

Tests should not require a real database — use a mock `PgConnection`.

```rust,ignore
// tests/integration.rs
use nebula_core::scope::Scope;
use nebula_resource::{
    AcquireOptions, Manager, PoolRuntime, RegistrationSpec, ScopeLevel,
    TopologyRuntime, TopologyTag, context::ResourceContext, dedup::SlotIdentity,
    topology::pooled::config::Config as PoolConfig,
};
use nebula_resource_postgres::{PostgresConfig, PostgresResource};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn register_and_acquire() {
    let manager = Manager::new();
    let pg_config = PostgresConfig::default();
    let pool_rt = PoolRuntime::<PostgresResource>::try_new(
        PoolConfig::default(),
        pg_config.fingerprint(),
    ).expect("valid pool config");

    manager.register(RegistrationSpec {
        resource: PostgresResource,
        config: pg_config,
        scope: ScopeLevel::Global,
        slot_identity: SlotIdentity::Unbound,
        topology: TopologyRuntime::Pool(pool_rt),
        acquire: Manager::erased_acquire_pooled_for::<PostgresResource>(),
        recovery_gate: None,
    }).expect("valid config must register");

    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let handle = manager
        .acquire_pooled::<PostgresResource>(&ctx, &AcquireOptions::default())
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
- [ ] No `#[credential]` field unless the adapter binds to a credential; if it
      does, the field is `SlotCell<CredentialGuard<C>>` and `create` reads it
      via `self.<field>_slot()` (handling the unbound `None` case).
- [ ] If credential-bound: `on_credential_refresh` performs the blue-green
      swap on the runtime's interior mutability; `on_credential_revoke`
      ensures no further authenticated traffic on the revoked credential.
- [ ] `Runtime` does not implement `Debug`, or implements a redacted variant
      that omits connection strings and internal buffer state.
- [ ] Integration tests use a mock runtime, not a live network service.
- [ ] `#![forbid(unsafe_code)]` is set in `lib.rs`.
