//! Postgres Pool topology example.
//!
//! Demonstrates the headline patterns for a database-style resource:
//!
//! - **Pool topology** ([`Resource`] + [`Pooled`]) — N interchangeable instances with `is_broken` /
//!   `recycle` lifecycle hooks.
//! - **`ResourceAction`** for per-execution scoped configuration: `ScopedTestSchema` creates a
//!   temporary schema before downstream nodes run and cleans it up on branch exit.
//! - **Slot-binding mental model** — `QueryUsers` declares a resource slot (the connection pool)
//!   and a credential slot (the database creds). The example wires the slots manually because the
//!   runtime resolution pipeline (engine + manager) runs in production binaries; here we focus on
//!   the lifecycle and topology shape.
//!
//! No real database is required — `MockPgConnection` simulates the network
//! boundary so the example is self-contained.
//!
//! ## Run
//!
//! ```shell
//! cargo run -p nebula-examples --example resource_postgres_pool
//! ```
//!
//! ## What it prints
//!
//! - Pool warmup output (creating connections lazily)
//! - 5 simulated `QueryUsers` calls, each acquiring a connection, running a query, and releasing it
//!   back to the pool
//! - The `ScopedTestSchema` configure / cleanup pair — only one schema is created and torn down per
//!   branch
//! - Final pool stats (idle, capacity, in-use)

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use nebula_core::{ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_resource::Pooled;
use nebula_resource::topology::pooled::PoolProvider;
use nebula_resource::{
    AcquireOptions, Manager, RegistrationSpec, ResourceContext,
    dedup::SlotIdentity,
    error::Error as ResourceError,
    resource::{Provider, ResourceConfig, ResourceMetadata},
    topology::pooled::{BrokenCheck, RecycleDecision, config::Config as PoolConfig},
};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

// ─── Database credential (mock) ────────────────────────────────────────────

/// Stand-in for a real `DatabaseCredential` carried by a credential slot.
///
/// In production this would derive `Credential` and live in a credential
/// store; here we only model the shape so `QueryUsers` can show the slot
/// pattern without requiring full credential plumbing.
#[derive(Clone)]
#[allow(
    dead_code,
    reason = "password field is held to model SecretString-shaped slot — never logged"
)]
struct DatabaseCredential {
    username: String,
    // The password would be a `SecretString` in production. Plain `String`
    // keeps the example dependency-free.
    password: String,
    database: String,
}

impl DatabaseCredential {
    fn new(
        username: impl Into<String>,
        password: impl Into<String>,
        database: impl Into<String>,
    ) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
            database: database.into(),
        }
    }
}

// ─── Pool resource ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
struct PostgresConfig {
    application_name: String,
    statement_timeout_ms: u64,
}

nebula_schema::impl_empty_has_schema!(PostgresConfig);

impl ResourceConfig for PostgresConfig {
    fn validate(&self) -> Result<(), ResourceError> {
        if self.application_name.is_empty() {
            Err(ResourceError::permanent(
                "application_name must not be empty",
            ))
        } else {
            Ok(())
        }
    }

    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.application_name.hash(&mut h);
        self.statement_timeout_ms.hash(&mut h);
        h.finish()
    }
}

/// Minimal mock connection — counts queries + tracks "is broken" state.
#[derive(Debug)]
struct MockPgConnection {
    id: u64,
    queries_issued: AtomicU64,
    is_broken_flag: AtomicBool,
}

impl MockPgConnection {
    fn new(id: u64) -> Self {
        Self {
            id,
            queries_issued: AtomicU64::new(0),
            is_broken_flag: AtomicBool::new(false),
        }
    }

    /// Pretends to run a parameterized query. Production code would route
    /// to `tokio_postgres::Client::query`; the mock just bumps a counter.
    fn query_users_under_limit(&self, limit: i64) -> Vec<UserRow> {
        self.queries_issued.fetch_add(1, Ordering::SeqCst);
        // Synthesize rows so the example has something concrete to print.
        (0..limit.min(3))
            .map(|i| UserRow {
                id: i,
                name: format!("user-{i}"),
            })
            .collect()
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct UserRow {
    id: i64,
    name: String,
}

#[derive(Debug, Clone)]
struct PgError(String);

impl std::fmt::Display for PgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PgError {}

impl From<PgError> for ResourceError {
    fn from(e: PgError) -> Self {
        ResourceError::transient(e.0)
    }
}

#[derive(Clone)]
struct Postgres {
    /// Total connections created — observable by the example.
    create_counter: Arc<AtomicU64>,
}

impl Postgres {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for Postgres {
    type Config = PostgresConfig;
    type Instance = Arc<MockPgConnection>;
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("demo.postgres")
    }

    async fn create(
        &self,
        config: &PostgresConfig,
        _ctx: &ResourceContext,
    ) -> Result<Arc<MockPgConnection>, ResourceError> {
        let counter = Arc::clone(&self.create_counter);
        let app = config.application_name.clone();
        let id = counter.fetch_add(1, Ordering::SeqCst);
        tracing::info!(connection_id = id, application_name = %app, "creating mock postgres connection");
        // Real impl would call `tokio_postgres::Config::connect` here.
        Ok(Arc::new(MockPgConnection::new(id)))
    }

    async fn destroy(&self, runtime: Arc<MockPgConnection>) -> Result<(), ResourceError> {
        tracing::info!(
            connection_id = runtime.id,
            "destroying mock postgres connection"
        );
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl nebula_resource::HasCredentialSlots for Postgres {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

impl PoolProvider for Postgres {
    fn is_broken(&self, runtime: &Arc<MockPgConnection>) -> BrokenCheck {
        if runtime.is_broken_flag.load(Ordering::Acquire) {
            BrokenCheck::Broken("mock connection flagged broken".into())
        } else {
            BrokenCheck::Healthy
        }
    }

    async fn recycle(
        &self,
        runtime: &Arc<MockPgConnection>,
        metrics: &nebula_resource::topology::pooled::InstanceMetrics,
    ) -> Result<RecycleDecision, ResourceError> {
        // Drop after 100 queries to demonstrate the recycle hook; real impl
        // would `DISCARD ALL` for transactional cleanliness.
        if runtime.queries_issued.load(Ordering::Acquire) >= 100 {
            tracing::info!(
                connection_id = runtime.id,
                "recycle: drop after query budget"
            );
            return Ok(RecycleDecision::Drop);
        }
        if metrics.error_count >= 3 {
            return Ok(RecycleDecision::Drop);
        }
        Ok(RecycleDecision::Keep)
    }
}

// ─── ScopedTestSchema — ResourceAction that creates a per-branch schema ────

/// Creates a temporary test schema and tears it down on branch exit.
///
/// In a real workflow, this would issue `CREATE SCHEMA test_<id>` against
/// the global pool's connection at branch entry, and `DROP SCHEMA ... CASCADE`
/// on cleanup. The example just records the lifecycle on a shared counter
/// so the run prints visible configure / cleanup events.
struct ScopedTestSchema {
    /// Shared lifecycle log so `main` can verify configure/cleanup ordering.
    events: Arc<parking_lot::Mutex<Vec<String>>>,
    schema_name: String,
}

impl ScopedTestSchema {
    fn new(schema_name: impl Into<String>, events: Arc<parking_lot::Mutex<Vec<String>>>) -> Self {
        Self {
            events,
            schema_name: schema_name.into(),
        }
    }

    /// Synchronous "configure" entry point — real `ResourceAction::configure`
    /// is async; we use a plain method here because the example does not run
    /// the full engine, only demonstrates the lifecycle ordering.
    fn configure(&self) -> ScopedSchemaHandle {
        self.events
            .lock()
            .push(format!("configure({})", self.schema_name));
        tracing::info!(schema = %self.schema_name, "ScopedTestSchema: CREATE SCHEMA");
        ScopedSchemaHandle {
            schema_name: self.schema_name.clone(),
            events: Arc::clone(&self.events),
        }
    }
}

/// Returned by `configure`, dropped to trigger `cleanup`.
struct ScopedSchemaHandle {
    schema_name: String,
    events: Arc<parking_lot::Mutex<Vec<String>>>,
}

impl Drop for ScopedSchemaHandle {
    fn drop(&mut self) {
        self.events
            .lock()
            .push(format!("cleanup({})", self.schema_name));
        tracing::info!(schema = %self.schema_name, "ScopedTestSchema: DROP SCHEMA CASCADE");
    }
}

// ─── QueryUsers action — slot-binding pattern (manually wired) ─────────────

/// Models the slot-binding pattern from ADR-0043. In Phase 3 prod code,
/// the macro emits this struct + `FromWorkflowNode` impl that resolves the
/// slots at dispatch. Here we wire the slots by hand so the example focuses
/// on the lifecycle.
#[allow(
    dead_code,
    reason = "auth field models the credential slot — full resolution lives in the engine"
)]
struct QueryUsers {
    /// Resource slot — declared `#[resource(key = "db")]` in ADR-0043 form.
    db: Arc<MockPgConnection>,
    /// Credential slot — declared `#[credential(key = "auth")]` in ADR-0043
    /// form. Production code never logs the field; here it is held only for
    /// demonstration shape.
    auth: DatabaseCredential,
}

#[derive(Debug, Deserialize)]
struct QueryUsersInput {
    limit: i64,
}

impl QueryUsers {
    /// Body of `StatelessAction::execute` from ADR-0043 §11. The full Variant
    /// A trait shape is exercised in `crates/engine/tests/end_to_end_pipeline.rs`;
    /// here we run the body inline because the example does not spin up the
    /// engine.
    fn execute(&self, input: QueryUsersInput) -> Vec<UserRow> {
        tracing::debug!(
            user = %self.auth.username,
            db = %self.auth.database,
            limit = input.limit,
            "QueryUsers: SELECT id, name FROM users LIMIT $1",
        );
        self.db.query_users_under_limit(input.limit)
    }
}

// ─── Wiring + main ─────────────────────────────────────────────────────────

fn ctx_for_demo() -> ResourceContext {
    ResourceContext::minimal(Scope::default(), CancellationToken::new())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    println!("=== Postgres Pool example ===\n");

    // 1. Manager owns the registry. Register a Pool-topology Postgres at the global scope.
    //    Production code would scope to Organization / Project.
    let manager = Arc::new(Manager::new());
    let postgres = Postgres::new();
    let create_counter = Arc::clone(&postgres.create_counter);

    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 4,
        create_timeout: Duration::from_secs(2),
        ..PoolConfig::default()
    };
    let pg_config = PostgresConfig {
        application_name: "nebula-example".into(),
        statement_timeout_ms: 30_000,
    };
    let pool_runtime =
        Pooled::<Postgres>::new(pool_config, ResourceConfig::fingerprint(&pg_config));
    manager.register(RegistrationSpec {
        resource: postgres.clone(),
        config: pg_config,
        scope: ScopeLevel::Global,
        slot_identity: SlotIdentity::Unbound,
        topology: pool_runtime,
        recovery_gate: None,
    })?;
    println!("[1] Postgres pool registered (min=0, max=4)");

    // 2. ScopedTestSchema simulates an engine branch entering a `ResourceAction` scope. The handle
    //    is dropped at end of scope, which triggers the cleanup hook.
    let lifecycle = Arc::new(parking_lot::Mutex::new(Vec::<String>::new()));
    let scoped = ScopedTestSchema::new("test_run_42", Arc::clone(&lifecycle));
    let _scope_handle = scoped.configure();
    println!("[2] ScopedTestSchema configured — schema test_run_42 active for downstream");

    // 3. Five simulated workflow nodes acquire from the pool, run a query, and release. We share
    //    one credential — in production each workflow's credential slot is resolved independently
    //    from the credential store.
    let creds = DatabaseCredential::new("nebula", "hunter2", "appdb");

    println!("\n[3] Running 5 simulated workflow nodes:");
    for run in 0..5 {
        let ctx = ctx_for_demo();
        let lease = manager
            .acquire_pooled::<Postgres>(&ctx, &AcquireOptions::default())
            .await?;

        // Build the per-execution Action with slots filled. The engine does
        // this via `FromWorkflowNode::from_workflow_node` in production.
        let action = QueryUsers {
            db: Arc::clone(&*lease),
            auth: creds.clone(),
        };
        let rows = action.execute(QueryUsersInput { limit: 3 });
        println!("  run={run} connection_id={} rows={}", lease.id, rows.len());

        // Lease drops here — pool runs `recycle` on the worker thread.
        drop(lease);
    }

    // 4. Give the release queue a moment to drain its background work, then print pool stats and
    //    assert on the counter.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let stats = manager.pool_stats::<Postgres>(&ScopeLevel::Global).await;
    println!(
        "\n[4] Pool stats: idle={}, in_use={}, capacity={}",
        stats.as_ref().map(|s| s.idle).unwrap_or(0),
        stats.as_ref().map(|s| s.in_use).unwrap_or(0),
        stats.as_ref().map(|s| s.capacity).unwrap_or(0),
    );
    let total_creates = create_counter.load(Ordering::SeqCst);
    println!("    Total Resource::create invocations: {total_creates}");
    assert!(
        (1..=4).contains(&total_creates),
        "should have created at most max_size=4 connections; got {total_creates}",
    );

    // 5. Drop the scope handle to force ScopedTestSchema cleanup. In a real engine, this happens
    //    automatically when the branch exits.
    drop(_scope_handle);
    println!("\n[5] ScopedTestSchema cleaned up — schema dropped on branch exit");

    println!("\n[6] Lifecycle log:");
    for ev in lifecycle.lock().iter() {
        println!("    {ev}");
    }

    // Gracefully shutdown the manager so all background tasks (release queue,
    // health probes) drain before the process exits.
    manager.shutdown();

    println!("\n=== Done ===");
    Ok(())
}
