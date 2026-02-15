//! Simple pooling example
//!
//! Demonstrates basic resource pool usage with the bb8-style API.

use std::time::Duration;

use async_trait::async_trait;

use nebula_resource::{
    context::ResourceContext,
    error::ResourceResult,
    pool::{Pool, PoolConfig},
    resource::{Resource, ResourceConfig},
    scope::ResourceScope,
};

/// Example resource configuration
#[derive(Debug, Clone, serde::Deserialize)]
struct ConnectionConfig {
    host: String,
}

impl ResourceConfig for ConnectionConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.host.is_empty() {
            return Err(nebula_resource::error::ResourceError::configuration(
                "host cannot be empty",
            ));
        }
        Ok(())
    }
}

/// Example resource that simulates a database connection
struct ConnectionResource;

#[async_trait]
impl Resource for ConnectionResource {
    type Config = ConnectionConfig;
    type Instance = String;

    fn id(&self) -> &str {
        "connection"
    }

    async fn create(
        &self,
        config: &Self::Config,
        _ctx: &ResourceContext,
    ) -> ResourceResult<Self::Instance> {
        // Simulate connection creation
        tokio::time::sleep(Duration::from_millis(50)).await;
        Ok(format!(
            "Connection-{}-{}",
            config.host,
            uuid::Uuid::new_v4()
        ))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Simple Resource Pool Example ===\n");

    // Create pool configuration
    let pool_config = PoolConfig {
        min_size: 2,
        max_size: 10,
        max_lifetime: Duration::from_secs(300),
        idle_timeout: Duration::from_secs(60),
        validation_interval: Duration::from_secs(30),
        acquire_timeout: Duration::from_secs(5),
    };

    let resource_config = ConnectionConfig {
        host: "localhost".to_string(),
    };

    // Create a pool
    let pool = Pool::new(ConnectionResource, resource_config, pool_config);

    println!("Pool created with:");
    println!("  - Min size: 2");
    println!("  - Max size: 10\n");

    // Acquire a resource
    let ctx = ResourceContext::new(ResourceScope::Global, "example-wf", "example-ex");

    println!("Acquiring resource...");
    let resource = pool.acquire(&ctx).await?;
    println!("  Resource acquired: {}\n", *resource);

    // Check pool stats
    let stats = pool.stats();
    println!("Pool statistics:");
    println!("  - Active: {}", stats.active);
    println!("  - Idle: {}", stats.idle);
    println!("  - Total acquisitions: {}", stats.total_acquisitions);

    // Release the resource by dropping the guard
    drop(resource);

    // Give the spawned return-to-pool task a moment
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Final stats
    let final_stats = pool.stats();
    println!("\nFinal statistics:");
    println!("  - Active: {}", final_stats.active);
    println!("  - Idle: {}", final_stats.idle);
    println!("  - Total releases: {}", final_stats.total_releases);

    println!("\n=== Example completed! ===");

    Ok(())
}
