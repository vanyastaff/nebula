//! Credential rotation example
//!
//! This example demonstrates:
//! - Credential configuration
//! - Automatic credential rotation
//! - Rotation callbacks
//! - Scheduler usage
//!
//! Note: Requires the 'credentials' feature to be enabled

#[cfg(feature = "credentials")]
use nebula_resource::credentials::{
    CredentialConfig, CredentialRotationScheduler,
};

#[cfg(not(feature = "credentials"))]
fn main() {
    println!("This example requires the 'credentials' feature.");
    println!("Run with: cargo run --example credential_rotation --features credentials");
}

#[cfg(feature = "credentials")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Nebula Resource - Credential Rotation Example ===\n");

    // 1. Configure credentials
    println!("1. Configuring credentials...");
    let config = CredentialConfig {
        credential_id: "database-creds".to_string(),
        auto_refresh: true,
        refresh_threshold_minutes: 5,
    };
    println!("   ✓ Credential ID: {}", config.credential_id);
    println!("   ✓ Auto-refresh: {}", config.auto_refresh);
    println!("   ✓ Refresh threshold: {} minutes\n", config.refresh_threshold_minutes);

    // 2. Create rotation scheduler
    println!("2. Creating rotation scheduler...");
    let rotation_interval = std::time::Duration::from_secs(60); // 1 minute for demo
    let scheduler = CredentialRotationScheduler::new(rotation_interval);
    println!("   ✓ Rotation interval: {:?}\n", rotation_interval);

    // 3. Configure connection string with placeholders
    println!("3. Connection string examples:");
    let examples = vec![
        "postgresql://user:{{password}}@localhost/db",
        "redis://{{token}}@localhost:6379",
        "mongodb://{{credential}}@cluster.example.com/mydb",
    ];

    for example in examples {
        println!("   - {}", example);
    }
    println!();

    // 4. Demonstrate rotation callback
    println!("4. Rotation callback example:");
    println!("   When credentials rotate, the callback receives the new token");
    println!("   and can update active connections, re-authenticate, etc.\n");

    // 5. Lifecycle management
    println!("5. Scheduler lifecycle:");
    println!("   - Handlers: {}", scheduler.handler_count());
    println!("   - Status: Ready to start");
    println!("   - Interval: Check and rotate every {:?}", rotation_interval);
    println!();

    // Note: We can't actually start the scheduler without a real CredentialManager
    println!("Note: Full rotation requires nebula-credential CredentialManager integration");
    println!("      This example shows the API structure and configuration\n");

    println!("=== Key Features ===");
    println!("✓ Automatic token refresh before expiration");
    println!("✓ Configurable rotation intervals");
    println!("✓ Rotation callbacks for connection updates");
    println!("✓ Thread-safe credential caching");
    println!("✓ Graceful start/stop lifecycle");
    println!("✓ Tracing integration for monitoring");

    println!("\n=== Example completed successfully! ===");

    Ok(())
}
