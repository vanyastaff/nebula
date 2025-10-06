//! Credential Integration Example
//!
//! This example demonstrates how nebula-resource integrates with nebula-credential
//! for secure, automatic credential management.
//!
//! Shows:
//! - ResourceCredentialProvider usage
//! - Automatic token refresh
//! - Connection string placeholder replacement
//! - Credential rotation scheduling

#[cfg(feature = "credentials")]
use nebula_credential::{
    CredentialManager, CredentialRegistry,
    core::{
        AccessToken, CredentialContext, CredentialError, CredentialMetadata, CredentialState,
        SecureString,
    },
    testing::{MockLock, MockStateStore, MockTokenCache},
    traits::{Credential, StateStore, TokenCache},
};

#[cfg(feature = "credentials")]
use nebula_resource::credentials::{
    CredentialConfig, CredentialRotationHandler, CredentialRotationScheduler,
    ResourceCredentialProvider, build_connection_string_with_credentials,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

// ═══════════════════════════════════════════════════════════════
// Example Credential: Database Password
// ═══════════════════════════════════════════════════════════════

#[cfg(feature = "credentials")]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DatabasePasswordInput {
    username: String,
    password: String,
    database: String,
}

#[cfg(feature = "credentials")]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DatabasePasswordState {
    username: String,
    password: SecureString,
    database: String,
    rotation_count: u32,
}

#[cfg(feature = "credentials")]
impl CredentialState for DatabasePasswordState {
    const KIND: &'static str = "database_password";
    const VERSION: u16 = 1;
}

#[cfg(feature = "credentials")]
struct DatabasePasswordCredential;

#[cfg(feature = "credentials")]
#[async_trait]
impl Credential for DatabasePasswordCredential {
    type Input = DatabasePasswordInput;
    type State = DatabasePasswordState;

    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata {
            id: "database_password",
            name: "Database Password",
            description: "Database username and password credentials",
            supports_refresh: true,
            requires_interaction: false,
        }
    }

    async fn initialize(
        &self,
        input: &Self::Input,
        _ctx: &mut CredentialContext,
    ) -> Result<(Self::State, Option<AccessToken>), CredentialError> {
        let state = DatabasePasswordState {
            username: input.username.clone(),
            password: SecureString::new(input.password.clone()),
            database: input.database.clone(),
            rotation_count: 0,
        };

        // Token contains the password for authentication
        let token = AccessToken::bearer(input.password.clone())
            .with_expiration(SystemTime::now() + Duration::from_secs(3600));

        Ok((state, Some(token)))
    }

    async fn refresh(
        &self,
        state: &mut Self::State,
        _ctx: &mut CredentialContext,
    ) -> Result<AccessToken, CredentialError> {
        // Simulate password rotation
        state.rotation_count += 1;
        let new_password = format!("rotated_password_{}", state.rotation_count);
        state.password = SecureString::new(new_password.clone());

        let token = AccessToken::bearer(new_password)
            .with_expiration(SystemTime::now() + Duration::from_secs(3600));

        Ok(token)
    }
}

// ═══════════════════════════════════════════════════════════════
// Main Example
// ═══════════════════════════════════════════════════════════════

#[cfg(feature = "credentials")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║  Nebula Resource - Credential Integration Example       ║");
    println!("╚══════════════════════════════════════════════════════════╝\n");

    // ═══════════════════════════════════════════════════════════════
    // Setup CredentialManager
    // ═══════════════════════════════════════════════════════════════
    println!("📦 Setup: Creating CredentialManager");

    let store = Arc::new(MockStateStore::new());
    let cache = Arc::new(MockTokenCache::new());
    let registry = Arc::new(CredentialRegistry::new());

    registry.register_credential(DatabasePasswordCredential);

    let cred_manager = Arc::new(
        CredentialManager::builder()
            .with_store(store as Arc<dyn StateStore>)
            .with_cache(cache as Arc<dyn TokenCache>)
            .with_lock(MockLock::new())
            .with_registry(registry)
            .build()?,
    );

    println!("   ✓ CredentialManager ready\n");

    // ═══════════════════════════════════════════════════════════════
    // Create Database Credential
    // ═══════════════════════════════════════════════════════════════
    println!("🔐 Creating database credential...");

    let cred_id = cred_manager
        .create_credential(
            "database_password",
            serde_json::json!({
                "username": "app_user",
                "password": "initial_password_123",
                "database": "production_db"
            }),
        )
        .await?;

    println!("   ✓ Credential created: {}", cred_id);
    println!();

    // ═══════════════════════════════════════════════════════════════
    // Example 1: ResourceCredentialProvider
    // ═══════════════════════════════════════════════════════════════
    println!("📋 Example 1: ResourceCredentialProvider");

    let provider = Arc::new(ResourceCredentialProvider::new(
        cred_manager.clone(),
        cred_id.clone(),
    ));

    // Get token through provider (cached)
    println!("   Getting token (1st time - cache miss)...");
    let token1 = provider.get_token().await?;
    println!(
        "      ✓ Token: {} chars",
        token1.token.with_exposed(|s| s.len())
    );

    // Get token again (should be cached)
    println!("   Getting token (2nd time - cache hit)...");
    let token2 = provider.get_token().await?;
    println!(
        "      ✓ Token: {} chars (same as before)",
        token2.token.with_exposed(|s| s.len())
    );

    // Verify it's the same token
    assert!(
        token1
            .token
            .with_exposed(|a| token2.token.with_exposed(|b| a == b))
    );
    println!("      ✓ Cache working correctly\n");

    // ═══════════════════════════════════════════════════════════════
    // Example 2: Connection String Placeholder Replacement
    // ═══════════════════════════════════════════════════════════════
    println!("🔗 Example 2: Connection String Builder");

    let base_url = "postgresql://app_user:{password}@localhost:5432/production_db";
    println!("   Template: {}", base_url);

    let connection_string = build_connection_string_with_credentials(base_url, &provider).await?;
    println!("   ✓ Built connection string");
    println!("      └─ Placeholder {{password}} replaced with credential\n");

    // ═══════════════════════════════════════════════════════════════
    // Example 3: Credential Invalidation and Refresh
    // ═══════════════════════════════════════════════════════════════
    println!("🔄 Example 3: Credential Invalidation");

    println!("   Invalidating cached credential...");
    provider.invalidate().await;

    println!("   Getting fresh token...");
    let token3 = provider.get_token().await?;
    println!(
        "      ✓ New token retrieved: {} chars",
        token3.token.with_exposed(|s| s.len())
    );

    // Should trigger refresh from credential manager
    println!("      └─ Token refreshed from credential manager\n");

    // ═══════════════════════════════════════════════════════════════
    // Example 4: Credential Rotation Handler
    // ═══════════════════════════════════════════════════════════════
    println!("🔁 Example 4: Credential Rotation Handler");

    let rotation_handler = CredentialRotationHandler::new(provider.clone()).with_rotation_callback(
        |new_token| async move {
            println!("      📢 Rotation callback triggered!");
            println!("         New token: {} chars", new_token.len());
            Ok(())
        },
    );

    println!("   Checking and rotating credential...");
    let rotated = rotation_handler.check_and_rotate().await?;
    println!("      ✓ Rotation complete: {}", rotated);
    println!();

    // ═══════════════════════════════════════════════════════════════
    // Example 5: Rotation Scheduler
    // ═══════════════════════════════════════════════════════════════
    println!("⏰ Example 5: Rotation Scheduler");

    let scheduler = CredentialRotationScheduler::new(Duration::from_secs(60));

    println!("   Adding rotation handler to scheduler...");
    scheduler.add_handler(Arc::new(rotation_handler)).await;
    println!("      ✓ Handler count: {}", scheduler.handler_count().await);

    println!("   Starting scheduler (rotation every 60s)...");
    scheduler.start().await?;
    println!("      ✓ Scheduler started");

    // Let it run briefly
    tokio::time::sleep(Duration::from_millis(100)).await;

    println!("   Stopping scheduler...");
    scheduler.stop().await;
    println!("      ✓ Scheduler stopped\n");

    // ═══════════════════════════════════════════════════════════════
    // Example 6: Resource Config Pattern
    // ═══════════════════════════════════════════════════════════════
    println!("📝 Example 6: Resource Config Pattern");

    let config = CredentialConfig {
        credential_id: cred_id.to_string(),
        auto_refresh: true,
        refresh_threshold_minutes: 5,
    };

    println!("   Resource configuration:");
    println!("      ├─ Credential ID: {}", config.credential_id);
    println!("      ├─ Auto-refresh: {}", config.auto_refresh);
    println!(
        "      └─ Refresh threshold: {} minutes",
        config.refresh_threshold_minutes
    );
    println!();

    // ═══════════════════════════════════════════════════════════════
    // Summary
    // ═══════════════════════════════════════════════════════════════
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║                        Summary                           ║");
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║ ✓ ResourceCredentialProvider for token caching          ║");
    println!("║ ✓ Automatic connection string building                  ║");
    println!("║ ✓ Credential invalidation and refresh                   ║");
    println!("║ ✓ Rotation handlers with callbacks                      ║");
    println!("║ ✓ Background rotation scheduling                        ║");
    println!("║ ✓ CredentialConfig for resource configuration           ║");
    println!("╚══════════════════════════════════════════════════════════╝");

    println!("\n💡 Integration Benefits:");
    println!("   • Automatic token refresh before expiration");
    println!("   • Credential caching reduces manager calls");
    println!("   • Background rotation for security");
    println!("   • Connection string placeholder replacement");
    println!("   • Centralized credential management");

    println!("\n💡 Usage in Real Resources:");
    println!("   • PostgreSQL: Use provider for password rotation");
    println!("   • MongoDB: Build auth URLs with credentials");
    println!("   • HTTP Client: Auto-refresh Bearer tokens");
    println!("   • Redis: Rotate AUTH credentials");
    println!("   • Kafka: Update SASL credentials");

    println!("\n💡 Next Steps:");
    println!("   • See docs/credential-integration.md for design");
    println!("   • Implement AuthenticatedResource trait");
    println!("   • Create resource-specific authenticators");
    println!("   • Add integration tests");

    Ok(())
}

#[cfg(not(feature = "credentials"))]
fn main() {
    println!("This example requires the 'credentials' feature to be enabled.");
    println!("Run with: cargo run --example credential_integration --features credentials");
}
