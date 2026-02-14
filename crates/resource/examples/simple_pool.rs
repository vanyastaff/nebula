//! Simple pooling example
//!
//! Demonstrates basic resource pool usage

use nebula_resource::{
    core::{
        context::ResourceContextBuilder,
        error::ResourceResult,
        resource::{ResourceId, ResourceInstanceMetadata, TypedResourceInstance},
        traits::PoolConfig,
    },
    pool::{PoolStrategy, ResourcePool},
};
use std::sync::Arc;
use std::time::Duration;

async fn create_connection() -> ResourceResult<TypedResourceInstance<String>> {
    let metadata = ResourceInstanceMetadata {
        instance_id: uuid::Uuid::new_v4(),
        resource_id: ResourceId::new("connection", "1.0"),
        state: nebula_resource::core::lifecycle::LifecycleState::Ready,
        context: ResourceContextBuilder::default().build(),
        created_at: chrono::Utc::now(),
        last_accessed_at: None,
        tags: std::collections::HashMap::new(),
    };

    // Simulate connection creation
    tokio::time::sleep(Duration::from_millis(50)).await;

    Ok(TypedResourceInstance::new(
        Arc::new(format!("Connection-{}", uuid::Uuid::new_v4())),
        metadata,
    ))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Simple Resource Pool Example ===\n");

    // Create pool configuration
    let config = PoolConfig {
        min_size: 2,
        max_size: 10,
        max_lifetime: Duration::from_secs(300),
        idle_timeout: Duration::from_secs(60),
        validation_interval: Duration::from_secs(30),
        acquire_timeout: Duration::from_secs(5),
    };

    // Create a pool with FIFO strategy
    let pool = ResourcePool::new(config, PoolStrategy::Fifo, create_connection);

    println!("Pool created with:");
    println!("  - Min size: 2");
    println!("  - Max size: 10");
    println!("  - Strategy: FIFO\n");

    // Acquire a resource
    println!("Acquiring resource...");
    let resource = pool.acquire().await?;
    println!("✓ Resource acquired: {}\n", resource.instance_id());

    // Check pool stats
    let stats = pool.stats();
    println!("Pool statistics:");
    println!("  - Active: {}", stats.active_count);
    println!("  - Idle: {}", stats.idle_count);
    println!("  - Total acquisitions: {}", stats.total_acquisitions);
    println!("  - Utilization: {:.0}%\n", stats.utilization * 100.0);

    // Release the resource
    let id = resource.instance_id();
    drop(resource);
    pool.release(id).await?;
    println!("✓ Resource released\n");

    // Final stats
    let final_stats = pool.stats();
    println!("Final statistics:");
    println!("  - Active: {}", final_stats.active_count);
    println!("  - Idle: {}", final_stats.idle_count);
    println!("  - Total releases: {}", final_stats.total_releases);

    println!("\n=== Example completed! ===");

    Ok(())
}
