//! Monitoring & metrics example
//!
//! Demonstrates how to:
//! - Subscribe to lifecycle events via the `EventBus`
//! - Inspect pool health state and quarantine status
//! - Use `ManagerBuilder` for configuration

use std::sync::Arc;
use std::time::Duration;

use nebula_core::ResourceKey;
use nebula_resource::events::{EventBus, ResourceEvent};
use nebula_resource::manager::ManagerBuilder;
use nebula_resource::pool::PoolConfig;
use nebula_resource::resource::{Config, Resource};
use nebula_resource::{Context, ExecutionId, Result, Scope, WorkflowId};

// -----------------------------------------------------------------------------
// Demo resource
// -----------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct DemoConfig {
    name: String,
}

impl Config for DemoConfig {
    fn validate(&self) -> Result<()> {
        Ok(())
    }
}

struct DemoResource;

impl Resource for DemoResource {
    type Config = DemoConfig;
    type Instance = String;
    type Deps = ();

    fn metadata(&self) -> nebula_resource::ResourceMetadata {
        let key = ResourceKey::try_from("demo-metric").expect("valid resource key");
        nebula_resource::ResourceMetadata::new(
            key,
            "Demo Metric Resource",
            "Example resource for monitoring metrics output",
        )
        .with_icon("demo")
        .with_tag("category:demo")
    }

    async fn create(&self, config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
        tokio::time::sleep(Duration::from_millis(10)).await;
        Ok(format!("instance-{}", config.name))
    }
}

// -----------------------------------------------------------------------------
// Main
// -----------------------------------------------------------------------------

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("=== Nebula Resource: Monitoring & Metrics Example ===\n");

    // 1. Build a manager with a shared event bus.
    let event_bus = Arc::new(EventBus::new(256));

    let manager = Arc::new(
        ManagerBuilder::new()
            .event_bus(Arc::clone(&event_bus))
            .build(),
    );

    // 2. Subscribe to events and print them (background task).
    let event_bus_clone = Arc::clone(&event_bus);
    tokio::spawn(async move {
        let mut rx = event_bus_clone.subscribe();
        while let Some(ev) = rx.recv().await {
            match &ev {
                ResourceEvent::Created {
                    resource_key,
                    scope,
                } => {
                    println!("  [event] Created resource_id={resource_key} scope={scope:?}");
                }
                ResourceEvent::Acquired {
                    resource_key,
                    wait_duration,
                } => {
                    println!(
                        "  [event] Acquired resource_id={resource_key} wait={wait_duration:?}"
                    );
                }
                ResourceEvent::Released {
                    resource_key,
                    usage_duration,
                } => {
                    println!(
                        "  [event] Released resource_id={resource_key} usage_duration={usage_duration:?}"
                    );
                }
                ResourceEvent::HealthChanged {
                    resource_key,
                    from,
                    to,
                } => {
                    println!(
                        "  [event] HealthChanged resource_id={resource_key} {from:?} -> {to:?}"
                    );
                }
                ResourceEvent::PoolExhausted {
                    resource_key,
                    waiters,
                } => {
                    println!(
                        "  [event] PoolExhausted resource_id={resource_key} waiters={waiters}"
                    );
                }
                ResourceEvent::Quarantined {
                    resource_key,
                    reason,
                } => {
                    println!("  [event] Quarantined resource_id={resource_key} reason={reason}");
                }
                ResourceEvent::ConfigReloaded {
                    resource_key,
                    scope,
                } => {
                    println!("  [event] ConfigReloaded resource_id={resource_key} scope={scope:?}");
                }
                other => {
                    println!("  [event] {other:?}");
                }
            }
        }
    });

    // 3. Configure pool.
    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 4,
        acquire_timeout: Duration::from_secs(5),
        idle_timeout: Duration::from_secs(60),
        max_lifetime: Duration::from_secs(300),
        ..Default::default()
    };

    // 4. Register the resource.
    manager.register(
        DemoResource,
        DemoConfig {
            name: "metrics-demo".to_string(),
        },
        pool_config,
    )?;

    let ctx = Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new());

    let demo_key = ResourceKey::try_from("demo-metric").expect("valid resource key");

    // 5. Check health state (should be None initially).
    println!("Health state: {:?}", manager.get_health_state(&demo_key));
    println!(
        "Quarantined: {}",
        manager.quarantine().is_quarantined(demo_key.as_ref())
    );
    println!("Registered: {}", manager.is_registered(&demo_key));

    // 6. Acquire two guards.
    println!("\nAcquiring two resources...");
    let _guard1 = manager.acquire(&demo_key, &ctx).await?;
    let _guard2 = manager.acquire(&demo_key, &ctx).await?;
    // Allow events to propagate
    tokio::time::sleep(Duration::from_millis(20)).await;

    // 7. Release one.
    drop(_guard1);
    tokio::time::sleep(Duration::from_millis(20)).await;
    println!("Released one guard.");

    // 8. Release second.
    drop(_guard2);
    tokio::time::sleep(Duration::from_millis(20)).await;
    println!("Released second guard.");

    // 9. Check health and quarantine status again.
    println!(
        "\nHealth state after usage: {:?}",
        manager.get_health_state(&demo_key)
    );
    println!(
        "Quarantined after usage: {}",
        manager.quarantine().is_quarantined(demo_key.as_ref())
    );

    // 10. Shutdown.
    manager.shutdown().await?;
    println!("\n=== Done ===");
    Ok(())
}
