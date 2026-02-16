// Advanced pooled resource example.
//
// Demonstrates LIFO strategy, maintenance scheduling, health checking,
// resource dependencies, and event bus subscription.

use std::sync::Arc;
use std::time::Duration;

use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::events::EventBus;
use nebula_resource::health::{HealthCheckable, HealthStatus};
use nebula_resource::pool::{Pool, PoolConfig, PoolStrategy};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;

// -- Configuration ----------------------------------------------------------

#[derive(Debug, Clone)]
struct DbConfig {
    connection_string: String,
}

impl Config for DbConfig {
    fn validate(&self) -> Result<()> {
        if self.connection_string.is_empty() {
            return Err(nebula_resource::error::Error::configuration(
                "connection_string must not be empty",
            ));
        }
        Ok(())
    }
}

// -- Instance ---------------------------------------------------------------

/// Simulated database connection.
#[derive(Debug)]
struct DbConnection {
    id: u64,
    query_count: u64,
}

// -- Resource ---------------------------------------------------------------

struct DbResource {
    next_id: std::sync::atomic::AtomicU64,
}

impl Resource for DbResource {
    type Config = DbConfig;
    type Instance = DbConnection;

    fn id(&self) -> &str {
        "postgres"
    }

    async fn create(&self, _config: &DbConfig, _ctx: &Context) -> Result<DbConnection> {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        println!("  [create] new connection #{id}");
        Ok(DbConnection { id, query_count: 0 })
    }

    async fn is_valid(&self, conn: &DbConnection) -> Result<bool> {
        // Reject connections that have served too many queries.
        Ok(conn.query_count < 100)
    }

    async fn recycle(&self, conn: &mut DbConnection) -> Result<()> {
        // Reset per-checkout state.
        conn.query_count = 0;
        Ok(())
    }

    async fn cleanup(&self, conn: DbConnection) -> Result<()> {
        println!("  [cleanup] closing connection #{}", conn.id);
        Ok(())
    }

    /// This resource depends on a config store (for connection string lookup).
    fn dependencies(&self) -> Vec<&str> {
        vec!["config-store"]
    }
}

// -- Health checking --------------------------------------------------------

/// Wrap DbResource for health checks.
struct DbHealthChecker;

impl HealthCheckable for DbHealthChecker {
    async fn health_check(&self) -> Result<HealthStatus> {
        // In a real app, run `SELECT 1` or equivalent.
        Ok(HealthStatus::healthy()
            .with_latency(Duration::from_millis(2))
            .with_metadata("connections", "3"))
    }

    fn health_check_interval(&self) -> Duration {
        Duration::from_secs(15)
    }
}

// -- Main -------------------------------------------------------------------

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("=== Advanced Pooled Resource Example ===\n");

    // 1. Configure pool with LIFO strategy and automatic maintenance.
    let pool_config = PoolConfig {
        min_size: 2,
        max_size: 8,
        acquire_timeout: Duration::from_secs(5),
        idle_timeout: Duration::from_secs(120),
        max_lifetime: Duration::from_secs(600),
        strategy: PoolStrategy::Lifo,
        maintenance_interval: Some(Duration::from_secs(30)),
        ..Default::default()
    };

    println!("Pool config:");
    println!("  strategy: LIFO (hot working set)");
    println!(
        "  min_size: {}, max_size: {}",
        pool_config.min_size, pool_config.max_size
    );
    println!("  maintenance: every 30s\n");

    // 2. Set up event bus and subscribe before creating the pool.
    let event_bus = Arc::new(EventBus::new(256));
    let mut event_rx = event_bus.subscribe();

    // Spawn a task that prints events as they arrive.
    tokio::spawn(async move {
        while let Ok(event) = event_rx.recv().await {
            println!("  [event] {event:?}");
        }
    });

    // 3. Create pool with event bus.
    let db_config = DbConfig {
        connection_string: "host=localhost dbname=nebula".into(),
    };
    let resource = DbResource {
        next_id: std::sync::atomic::AtomicU64::new(1),
    };
    let pool = Pool::with_event_bus(resource, db_config, pool_config, Some(event_bus))?;
    println!("Pool created\n");

    // 4. Dependencies.
    let res = DbResource {
        next_id: std::sync::atomic::AtomicU64::new(1),
    };
    println!("Dependencies for '{}': {:?}", res.id(), res.dependencies());

    // 5. Health checking.
    let checker = DbHealthChecker;
    let status = checker.health_check().await?;
    println!("Health: {:?}, latency={:?}\n", status.state, status.latency);

    // 6. Use the pool.
    let ctx = Context::new(Scope::Global, "demo-wf", "demo-ex");

    println!("Acquiring connections...");
    let mut conn1 = pool.acquire(&ctx).await?;
    conn1.query_count += 5;
    println!("  conn #{}: ran {} queries", conn1.id, conn1.query_count);

    let mut conn2 = pool.acquire(&ctx).await?;
    conn2.query_count += 3;
    println!("  conn #{}: ran {} queries", conn2.id, conn2.query_count);

    // Return connections.
    drop(conn1);
    drop(conn2);
    tokio::time::sleep(Duration::from_millis(50)).await;

    // With LIFO, the next acquire returns the most recently released.
    let conn3 = pool.acquire(&ctx).await?;
    println!("  LIFO re-acquired conn #{} (most recent)", conn3.id);

    drop(conn3);
    tokio::time::sleep(Duration::from_millis(50)).await;

    println!("\nPool stats: {:?}", pool.stats());

    // 7. Shutdown.
    pool.shutdown().await?;
    println!("\nPool shut down cleanly.");

    Ok(())
}
