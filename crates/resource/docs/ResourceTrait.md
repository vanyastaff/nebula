# Implementing the Resource Trait

This guide walks through implementing the `Resource` trait for the nebula-resource crate. A resource represents any external dependency your workflow needs: a database connection, an HTTP client, a cache handle, etc.

## Overview

The `Resource` trait has two associated types and five methods (three with defaults):

```rust
pub trait Resource: Send + Sync + 'static {
    type Config: Config;
    type Instance: Send + Sync + 'static;

    fn id(&self) -> &str;
    fn create(&self, config: &Self::Config, ctx: &Context) -> impl Future<Output = Result<Self::Instance>> + Send;
    fn is_valid(&self, instance: &Self::Instance) -> impl Future<Output = Result<bool>> + Send;     // default: Ok(true)
    fn recycle(&self, instance: &mut Self::Instance) -> impl Future<Output = Result<()>> + Send;    // default: Ok(())
    fn cleanup(&self, instance: Self::Instance) -> impl Future<Output = Result<()>> + Send;         // default: drop
    fn dependencies(&self) -> Vec<&str>;                                                             // default: empty
}
```

## Step 1: Define Your Config

Every resource needs a configuration type that implements `Config`. The `validate()` method is called during pool creation.

```rust
use nebula_resource::error::{Error, FieldViolation, Result};
use nebula_resource::resource::Config;

#[derive(Debug, Clone)]
struct PostgresConfig {
    host: String,
    port: u16,
    database: String,
    max_query_duration_secs: u64,
}

impl Config for PostgresConfig {
    fn validate(&self) -> Result<()> {
        let mut violations = Vec::new();

        if self.host.is_empty() {
            violations.push(FieldViolation::new("host", "must not be empty", ""));
        }
        if self.port == 0 {
            violations.push(FieldViolation::new("port", "must be > 0", "0"));
        }

        if violations.is_empty() {
            Ok(())
        } else {
            Err(Error::validation(violations))
        }
    }
}
```

## Step 2: Define Your Instance

The instance is what gets pooled. It must be `Send + Sync + 'static`.

```rust
struct PgConnection {
    // Your connection handle, query count, etc.
    conn: tokio_postgres::Client,
    created_at: std::time::Instant,
}
```

## Step 3: Implement the Resource Trait

### `id()` -- Unique Identifier

Return a static string identifying this resource type. Used as the key in the Manager registry.

```rust
fn id(&self) -> &str {
    "postgres"
}
```

### `create()` -- Instance Factory

Called when the pool needs a new instance (no idle instance available, or pool is warming up). Use the config and context to build the instance.

```rust
async fn create(
    &self,
    config: &PostgresConfig,
    ctx: &Context,
) -> Result<PgConnection> {
    let (client, connection) = tokio_postgres::connect(
        &format!("host={} port={} dbname={}", config.host, config.port, config.database),
        tokio_postgres::NoTls,
    )
    .await
    .map_err(|e| Error::Initialization {
        resource_id: self.id().to_string(),
        reason: e.to_string(),
        source: Some(Box::new(e)),
    })?;

    // Spawn the connection task
    tokio::spawn(connection);

    Ok(PgConnection {
        conn: client,
        created_at: std::time::Instant::now(),
    })
}
```

### `is_valid()` -- Health Check (optional)

Called before returning an idle instance to a caller. Return `Ok(false)` to discard the instance and try the next one (or create a new one).

```rust
async fn is_valid(&self, instance: &PgConnection) -> Result<bool> {
    // Simple "ping" query
    match instance.conn.simple_query("SELECT 1").await {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}
```

### `recycle()` -- State Reset (optional)

Called when an instance is returned to the pool after use. Reset any per-checkout state.

```rust
async fn recycle(&self, instance: &mut PgConnection) -> Result<()> {
    // Reset session state (e.g., cancel any pending queries, reset variables)
    instance.conn.simple_query("DISCARD ALL").await.map_err(|e| {
        Error::Internal {
            resource_id: self.id().to_string(),
            message: format!("recycle failed: {e}"),
            source: Some(Box::new(e)),
        }
    })?;
    Ok(())
}
```

If `recycle` returns an error, the instance is destroyed via `cleanup` instead of being returned to the idle queue.

### `cleanup()` -- Teardown (optional)

Called when an instance is permanently removed from the pool (expired, failed validation, pool shutdown, recycle failure). Close connections, flush buffers, release system resources.

```rust
async fn cleanup(&self, instance: PgConnection) -> Result<()> {
    // The Drop impl on tokio_postgres::Client handles disconnection,
    // but you might want to run cleanup queries first.
    drop(instance);
    Ok(())
}
```

### `dependencies()` -- Ordering (optional)

Return resource IDs that must be initialized before this one. Used by the Manager for startup ordering and reverse-order shutdown.

```rust
fn dependencies(&self) -> Vec<&str> {
    vec!["config-store", "credential-vault"]
}
```

## Step 4: Use with a Pool

```rust
use nebula_resource::pool::{Pool, PoolConfig};
use nebula_resource::context::Context;
use nebula_resource::scope::Scope;

let pool = Pool::new(
    MyPostgresResource,
    PostgresConfig { host: "localhost".into(), port: 5432, database: "app".into(), max_query_duration_secs: 30 },
    PoolConfig {
        min_size: 2,
        max_size: 20,
        acquire_timeout: std::time::Duration::from_secs(5),
        ..Default::default()
    },
)?;

let ctx = Context::new(Scope::Global, "my-workflow", "exec-001");
let conn = pool.acquire(&ctx).await?;
// Use conn via Deref...
// conn is returned to pool when guard is dropped.
```

## Common Patterns

### Validation with FieldViolation

Use `Error::validation()` with `FieldViolation` for structured config errors:

```rust
let mut violations = vec![];
if config.timeout_secs == 0 {
    violations.push(FieldViolation::new("timeout_secs", "must be > 0", "0"));
}
if !violations.is_empty() {
    return Err(Error::validation(violations));
}
```

### Error Mapping

Map external errors into resource errors:

```rust
some_io_call().await.map_err(|e| Error::Initialization {
    resource_id: self.id().to_string(),
    reason: e.to_string(),
    source: Some(Box::new(e)),
})?;
```

### Cancellation Awareness

Use the context's cancellation token in long-running create operations:

```rust
async fn create(&self, config: &Self::Config, ctx: &Context) -> Result<Self::Instance> {
    tokio::select! {
        result = connect(config) => result,
        _ = ctx.cancellation.cancelled() => Err(Error::Unavailable {
            resource_id: self.id().to_string(),
            reason: "cancelled".to_string(),
            retryable: false,
        }),
    }
}
```

## Checklist

- [ ] Config implements `Config` with meaningful `validate()`
- [ ] `id()` returns a unique, descriptive string
- [ ] `create()` maps errors to `Error::Initialization`
- [ ] `is_valid()` performs a cheap liveness check (if applicable)
- [ ] `recycle()` resets per-checkout state (if applicable)
- [ ] `cleanup()` releases external resources (if applicable)
- [ ] `dependencies()` lists all required resources (if any)
