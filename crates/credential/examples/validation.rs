//! Credential validation example
//!
//! Demonstrates US3: Validation and Health Checks
//! - Credential expiration checking
//! - Rotation recommendations
//! - Batch validation

use nebula_credential::prelude::*;
use nebula_credential::rotation::{PeriodicConfig, RotationPolicy};
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Credential Manager: Validation Example ===\n");

    // Create manager
    let storage = Arc::new(MockStorageProvider::new());
    let manager = CredentialManager::builder().storage(storage).build();
    let key = EncryptionKey::from_bytes([0u8; 32]);
    let context = CredentialContext::new("user-1");

    // 1. Store a credential with rotation policy
    println!("1. Storing credential with 30-day rotation policy...");
    let id = CredentialId::new("database-password")?;
    let data = encrypt(&key, b"my-secret-password")?;

    let mut metadata = CredentialMetadata::new();
    metadata.rotation_policy = Some(RotationPolicy::Periodic(PeriodicConfig::new(
        Duration::from_secs(30 * 24 * 3600), // 30 days
        Duration::from_secs(24 * 3600),      // 24 hours
        false,                               // enable_jitter
    )?));

    manager.store(&id, data, metadata, &context).await?;
    println!("   ✓ Credential stored with rotation policy\n");

    // 2. Validate the credential
    println!("2. Validating credential...");
    let result = manager.validate(&id, &context).await?;

    if result.is_valid() {
        println!("   ✓ Credential is valid");
    }

    match result.details {
        ValidationDetails::Valid { expires_at } => {
            if let Some(expiry) = expires_at {
                println!("   ✓ Expires at: {}", expiry);

                // Check if rotation is recommended (less than 25% lifetime remaining)
                use std::time::Duration;
                let max_age = Duration::from_secs(30 * 24 * 3600); // 30 days

                if result.rotation_recommended(max_age) {
                    println!("   ⚠ Rotation recommended: less than 25% lifetime remaining");
                } else {
                    println!("   ✓ Rotation not needed: sufficient lifetime remaining");
                }
            }
        }
        ValidationDetails::Expired { expired_at, now } => {
            println!("   ✗ Credential expired at: {}", expired_at);
            println!("   Current time: {}", now);
        }
        ValidationDetails::NotFound => {
            println!("   ✗ Credential not found");
        }
        ValidationDetails::Invalid { reason } => {
            println!("   ✗ Credential invalid: {}", reason);
        }
    }
    println!();

    // 3. Store multiple credentials with different ages
    println!("3. Storing multiple credentials for batch validation...");
    let ids = vec!["cred-1", "cred-2", "cred-3"];

    for cred_name in &ids {
        let cred_id = CredentialId::new(*cred_name)?;
        let cred_data = encrypt(&key, format!("secret-{}", cred_name).as_bytes())?;

        let mut meta = CredentialMetadata::new();
        meta.rotation_policy = Some(RotationPolicy::Periodic(PeriodicConfig::new(
            Duration::from_secs(90 * 24 * 3600), // 90 days
            Duration::from_secs(24 * 3600),      // 24 hours
            false,                               // enable_jitter
        )?));

        manager.store(&cred_id, cred_data, meta, &context).await?;
    }
    println!("   ✓ Stored {} credentials\n", ids.len());

    // 4. Batch validation
    println!("4. Performing batch validation...");
    let cred_ids: Vec<CredentialId> = ids
        .iter()
        .map(|name| CredentialId::new(*name).unwrap())
        .collect();

    let results = manager.validate_batch(&cred_ids, &context).await?;

    println!("   Validation results:");
    for (id, result) in &results {
        let status = if result.is_valid() {
            "✓ VALID"
        } else {
            "✗ INVALID"
        };
        println!("     {} - {}", id, status);
    }

    let valid_count = results.values().filter(|r| r.is_valid()).count();
    let total = results.len();
    println!("   Summary: {}/{} valid", valid_count, total);
    println!();

    // 5. Demonstrate expired credential detection
    println!("5. Testing expired credential detection...");
    println!("   Note: Credentials expire after their rotation interval");
    println!("   In production, credentials older than the interval");
    println!("   would be flagged as expired.\n");

    println!("=== Validation example completed! ===");
    println!("\nKey takeaways:");
    println!("  • Rotation policies ensure credentials don't live forever");
    println!("  • Validation checks credential health status");
    println!("  • Batch validation efficiently checks multiple credentials");
    println!("  • Rotation recommendations help maintain security");

    Ok(())
}
