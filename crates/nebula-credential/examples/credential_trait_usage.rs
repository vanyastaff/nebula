//! Credential Trait Usage Example
//!
//! Demonstrates using the type-safe Credential trait with CredentialAdapter
//! for automatic conversion to CredentialFactory.
//!
//! This example shows the PREFERRED way to implement credentials when you
//! want type safety and don't need to manually handle JSON serialization.

use async_trait::async_trait;
use nebula_credential::core::{
    AccessToken, CredentialContext, CredentialError, CredentialMetadata, CredentialState,
    Result, SecureString,
};
use nebula_credential::testing::{MockLock, MockStateStore, MockTokenCache};
use nebula_credential::traits::{bridge::CredentialAdapter, Credential, StateStore, TokenCache};
use nebula_credential::{CredentialManager, CredentialRegistry};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

// ═══════════════════════════════════════════════════════════════════════════
// Step 1: Define Input and State with strong typing
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DatabaseInput {
    host: String,
    port: u16,
    database: String,
    username: String,
    password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DatabaseState {
    host: String,
    port: u16,
    database: String,
    username: String,
    password: SecureString,
    connection_count: u32,
    last_connected: SystemTime,
}

impl CredentialState for DatabaseState {
    const KIND: &'static str = "database";
    const VERSION: u16 = 1;
}

// ═══════════════════════════════════════════════════════════════════════════
// Step 2: Implement the Credential trait (type-safe!)
// ═══════════════════════════════════════════════════════════════════════════

struct DatabaseCredential;

#[async_trait]
impl Credential for DatabaseCredential {
    type Input = DatabaseInput;
    type State = DatabaseState;

    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata {
            id: "database",
            name: "Database Connection",
            description: "PostgreSQL/MySQL database credentials with connection pooling",
            supports_refresh: true,
            requires_interaction: false,
        }
    }

    async fn initialize(
        &self,
        input: &Self::Input,
        _ctx: &mut CredentialContext,
    ) -> Result<(Self::State, Option<AccessToken>)> {
        println!("   🔧 Initializing database credential");
        println!("      └─ Connecting to {}:{}/{}", input.host, input.port, input.database);

        // Simulate database connection
        tokio::time::sleep(Duration::from_millis(50)).await;

        let state = DatabaseState {
            host: input.host.clone(),
            port: input.port,
            database: input.database.clone(),
            username: input.username.clone(),
            password: SecureString::new(input.password.clone()),
            connection_count: 1,
            last_connected: SystemTime::now(),
        };

        // For databases, the "token" is typically a connection string or session ID
        let connection_string = format!(
            "postgresql://{}:***@{}:{}/{}",
            input.username, input.host, input.port, input.database
        );

        let token = AccessToken::bearer(connection_string)
            .with_expiration(SystemTime::now() + Duration::from_secs(3600));

        Ok((state, Some(token)))
    }

    async fn refresh(
        &self,
        state: &mut Self::State,
        _ctx: &mut CredentialContext,
    ) -> Result<AccessToken> {
        println!("   🔄 Refreshing database connection");
        println!("      └─ Connection count: {}", state.connection_count);

        // Simulate reconnection
        tokio::time::sleep(Duration::from_millis(30)).await;

        state.connection_count += 1;
        state.last_connected = SystemTime::now();

        let connection_string = format!(
            "postgresql://{}:***@{}:{}/{}#conn{}",
            state.username, state.host, state.port, state.database, state.connection_count
        );

        let token = AccessToken::bearer(connection_string)
            .with_expiration(SystemTime::now() + Duration::from_secs(3600));

        Ok(token)
    }

    async fn validate(
        &self,
        state: &Self::State,
        _ctx: &CredentialContext,
    ) -> Result<bool> {
        // Check if connection is recent
        let elapsed = SystemTime::now()
            .duration_since(state.last_connected)
            .unwrap_or(Duration::from_secs(0));

        Ok(elapsed < Duration::from_secs(7200)) // Valid for 2 hours
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Main Example
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║          Credential Trait Usage Example                 ║");
    println!("╚══════════════════════════════════════════════════════════╝\n");

    // ═══════════════════════════════════════════════════════════════
    // Setup: Wrap Credential in CredentialAdapter
    // ═══════════════════════════════════════════════════════════════
    println!("📦 Setting up CredentialManager");

    let registry = Arc::new(CredentialRegistry::new());

    // The magic: CredentialAdapter converts type-safe Credential to CredentialFactory
    println!("   └─ Wrapping DatabaseCredential in CredentialAdapter...");
    let adapter = CredentialAdapter::new(DatabaseCredential);
    registry.register(Arc::new(adapter));

    let manager = CredentialManager::builder()
        .with_store(Arc::new(MockStateStore::new()) as Arc<dyn StateStore>)
        .with_cache(Arc::new(MockTokenCache::new()) as Arc<dyn TokenCache>)
        .with_lock(MockLock::new())
        .with_registry(registry)
        .build()?;

    println!("   ✓ Manager ready\n");

    // ═══════════════════════════════════════════════════════════════
    // Create credential using strongly-typed input
    // ═══════════════════════════════════════════════════════════════
    println!("➕ Creating database credential");

    let input = serde_json::json!({
        "host": "localhost",
        "port": 5432,
        "database": "myapp_production",
        "username": "app_user",
        "password": "super_secret_password"
    });

    let cred_id = manager.create_credential("database", input).await?;
    println!("   ✓ Credential created: {}\n", cred_id);

    // ═══════════════════════════════════════════════════════════════
    // Get connection (token)
    // ═══════════════════════════════════════════════════════════════
    println!("🔗 Getting database connection");
    let token1 = manager.get_token(&cred_id).await?;
    println!("   ✓ Connection established:");
    println!("      └─ Connection string: {}", token1.token.expose());
    println!();

    // ═══════════════════════════════════════════════════════════════
    // Refresh connection
    // ═══════════════════════════════════════════════════════════════
    println!("🔄 Testing connection refresh");
    manager.cache().unwrap().clear().await?;

    let token2 = manager.get_token(&cred_id).await?;
    println!("   ✓ Connection refreshed:");
    println!("      └─ New connection: {}", token2.token.expose());
    println!();

    // ═══════════════════════════════════════════════════════════════
    // Summary
    // ═══════════════════════════════════════════════════════════════
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║                    Summary                               ║");
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║ ✓ Implemented type-safe Credential trait                ║");
    println!("║ ✓ Used CredentialAdapter for automatic conversion       ║");
    println!("║ ✓ No manual JSON handling required                      ║");
    println!("║ ✓ Strong compile-time type safety                       ║");
    println!("║ ✓ Automatic serialization/deserialization               ║");
    println!("╚══════════════════════════════════════════════════════════╝");

    println!("\n💡 Why use Credential trait + CredentialAdapter?");
    println!("   • Type safety: Input and State are strongly typed");
    println!("   • No boilerplate: No manual JSON ser/de in initialize/refresh");
    println!("   • Clean code: Business logic is separate from serialization");
    println!("   • Testable: Easy to test with concrete types");
    println!();
    println!("   Compare this to manual CredentialFactory implementation");
    println!("   where you handle serde_json::Value directly!");

    Ok(())
}
