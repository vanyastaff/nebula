//! Custom Credential Implementation Example
//!
//! This example demonstrates how to implement a custom credential type from scratch:
//! - Define custom Input and State types
//! - Implement the Credential trait
//! - Create a CredentialFactory
//! - Register and use the custom credential
//!
//! We'll implement an API Key credential that rotates every 24 hours.

use async_trait::async_trait;
use nebula_credential::core::{
    AccessToken, CredentialContext, CredentialError, Result, SecureString, TokenType,
};
use nebula_credential::registry::CredentialFactory;
use nebula_credential::testing::{MockLock, MockStateStore, MockTokenCache};
use nebula_credential::traits::{StateStore, TokenCache};
use nebula_credential::{CredentialManager, CredentialRegistry};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

// ═══════════════════════════════════════════════════════════════════════════
// Step 1: Define the Input type (what the user provides)
// ═══════════════════════════════════════════════════════════════════════════

/// Input required to create an API Key credential
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApiKeyInput {
    /// The service URL
    service_url: String,
    /// Master API key (used to rotate keys)
    master_key: String,
}

// ═══════════════════════════════════════════════════════════════════════════
// Step 2: Define the State type (what we persist)
// ═══════════════════════════════════════════════════════════════════════════

/// Persistent state for an API Key credential
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApiKeyState {
    /// Service URL
    service_url: String,
    /// Master API key (encrypted in production)
    master_key: SecureString,
    /// Currently active API key
    current_key: SecureString,
    /// When the current key was created
    created_at: SystemTime,
    /// Rotation counter
    rotation_count: u32,
}

// ═══════════════════════════════════════════════════════════════════════════
// Step 3: Implement the Credential logic
// ═══════════════════════════════════════════════════════════════════════════

/// API Key credential implementation
struct ApiKeyCredential;

impl ApiKeyCredential {
    /// Initialize a new API key credential
    async fn initialize(
        &self,
        input: &ApiKeyInput,
        _cx: &CredentialContext,
    ) -> Result<(ApiKeyState, Option<AccessToken>)> {
        println!("   🔧 Initializing API key credential for {}", input.service_url);

        // In a real implementation, this would call the service API to generate a key
        // For this example, we'll simulate it
        let api_key = format!("ak_init_{}", uuid::Uuid::new_v4().simple());

        let state = ApiKeyState {
            service_url: input.service_url.clone(),
            master_key: SecureString::new(input.master_key.clone()),
            current_key: SecureString::new(api_key.clone()),
            created_at: SystemTime::now(),
            rotation_count: 0,
        };

        // Create initial access token
        let token = AccessToken {
            token: SecureString::new(api_key),
            token_type: TokenType::Bearer,
            issued_at: SystemTime::now(),
            // API keys expire after 24 hours
            expires_at: Some(SystemTime::now() + Duration::from_secs(86400)),
            scopes: None,
            claims: Default::default(),
        };

        Ok((state, Some(token)))
    }

    /// Refresh the API key (rotate it)
    async fn refresh(
        &self,
        state: &mut ApiKeyState,
        _cx: &CredentialContext,
    ) -> Result<AccessToken> {
        println!("   🔄 Rotating API key for {}", state.service_url);

        // In a real implementation, this would:
        // 1. Call the service API with master_key
        // 2. Get a new API key
        // 3. Optionally revoke the old key
        //
        // For this example, we simulate it:
        let new_key = format!("ak_rotated_{}_{}", state.rotation_count + 1, uuid::Uuid::new_v4().simple());

        state.current_key = SecureString::new(new_key.clone());
        state.created_at = SystemTime::now();
        state.rotation_count += 1;

        let token = AccessToken {
            token: SecureString::new(new_key),
            token_type: TokenType::Bearer,
            issued_at: SystemTime::now(),
            expires_at: Some(SystemTime::now() + Duration::from_secs(86400)),
            scopes: None,
            claims: Default::default(),
        };

        Ok(token)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Step 4: Implement CredentialFactory
// ═══════════════════════════════════════════════════════════════════════════

struct ApiKeyFactory {
    credential: ApiKeyCredential,
}

impl ApiKeyFactory {
    fn new() -> Self {
        Self {
            credential: ApiKeyCredential,
        }
    }
}

#[async_trait]
impl CredentialFactory for ApiKeyFactory {
    fn type_name(&self) -> &'static str {
        "api_key"
    }

    async fn create_and_init(
        &self,
        input_json: serde_json::Value,
        cx: &mut CredentialContext,
    ) -> Result<(Box<dyn erased_serde::Serialize>, Option<AccessToken>)> {
        // Deserialize input
        let input: ApiKeyInput = serde_json::from_value(input_json)
            .map_err(|e| CredentialError::DeserializationFailed(e.to_string()))?;

        // Initialize credential
        let (state, token) = self.credential.initialize(&input, cx).await?;

        // Return boxed state and token
        Ok((Box::new(state), token))
    }

    async fn refresh(
        &self,
        state_json: serde_json::Value,
        cx: &mut CredentialContext,
    ) -> Result<(Box<dyn erased_serde::Serialize>, AccessToken)> {
        // Deserialize state
        let mut state: ApiKeyState = serde_json::from_value(state_json)
            .map_err(|e| CredentialError::DeserializationFailed(e.to_string()))?;

        // Refresh credential
        let token = self.credential.refresh(&mut state, cx).await?;

        // Return updated state and new token
        Ok((Box::new(state), token))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Main Example
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║     Custom Credential Implementation Example            ║");
    println!("╚══════════════════════════════════════════════════════════╝\n");

    // ═══════════════════════════════════════════════════════════════
    // Setup
    // ═══════════════════════════════════════════════════════════════
    println!("📦 Setting up CredentialManager");
    let store = Arc::new(MockStateStore::new());
    let cache = Arc::new(MockTokenCache::new());
    let registry = Arc::new(CredentialRegistry::new());

    // Register our custom factory
    println!("   └─ Registering custom 'api_key' factory...");
    registry.register(Arc::new(ApiKeyFactory::new()));

    let manager = CredentialManager::builder()
        .with_store(store as Arc<dyn StateStore>)
        .with_cache(cache.clone() as Arc<dyn TokenCache>)
        .with_lock(MockLock::new())
        .with_registry(registry)
        .build()?;

    println!("   ✓ CredentialManager ready with custom credential type\n");

    // ═══════════════════════════════════════════════════════════════
    // Create API Key credential
    // ═══════════════════════════════════════════════════════════════
    println!("➕ Creating API key credential");

    let input = serde_json::json!({
        "service_url": "https://api.example.com",
        "master_key": "master_secret_key_abc123"
    });

    let cred_id = manager.create_credential("api_key", input).await?;
    println!("   ✓ API key credential created: {}\n", cred_id);

    // ═══════════════════════════════════════════════════════════════
    // Get the initial API key
    // ═══════════════════════════════════════════════════════════════
    println!("🔑 Getting initial API key");
    let token1 = manager.get_token(&cred_id).await?;

    println!("   ✓ Initial key retrieved:");
    println!("      ├─ Key prefix: {}...", &token1.token.expose()[..15]);
    println!("      ├─ Expires in: ~24 hours");
    println!("      └─ Rotation count: 0\n");

    // ═══════════════════════════════════════════════════════════════
    // Simulate key rotation
    // ═══════════════════════════════════════════════════════════════
    println!("🔄 Simulating key rotation (clearing cache)");
    cache.clear().await?;

    let token2 = manager.get_token(&cred_id).await?;
    println!("   ✓ Key rotated:");
    println!("      ├─ New key prefix: {}...", &token2.token.expose()[..15]);
    println!("      ├─ Different from initial: {}", token1.token.expose() != token2.token.expose());
    println!("      └─ Rotation count: 1\n");

    // ═══════════════════════════════════════════════════════════════
    // Rotate again
    // ═══════════════════════════════════════════════════════════════
    println!("🔄 Second rotation");
    cache.clear().await?;

    let token3 = manager.get_token(&cred_id).await?;
    println!("   ✓ Key rotated again:");
    println!("      ├─ New key prefix: {}...", &token3.token.expose()[..15]);
    println!("      ├─ Different from previous: {}", token2.token.expose() != token3.token.expose());
    println!("      └─ Rotation count: 2\n");

    // ═══════════════════════════════════════════════════════════════
    // Summary
    // ═══════════════════════════════════════════════════════════════
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║                    Summary                               ║");
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║ ✓ Defined custom Input type (ApiKeyInput)               ║");
    println!("║ ✓ Defined custom State type (ApiKeyState)               ║");
    println!("║ ✓ Implemented credential logic (ApiKeyCredential)       ║");
    println!("║ ✓ Created factory (ApiKeyFactory)                       ║");
    println!("║ ✓ Registered and used custom credential type            ║");
    println!("║ ✓ Demonstrated automatic key rotation                   ║");
    println!("╚══════════════════════════════════════════════════════════╝");

    println!("\n💡 Key Takeaways:");
    println!("   • Custom credentials are easy to implement");
    println!("   • You control initialization and refresh logic");
    println!("   • State is automatically persisted and versioned");
    println!("   • Tokens are automatically cached and refreshed");

    println!("\n📚 Next Steps:");
    println!("   • Implement real API calls in initialize() and refresh()");
    println!("   • Add error handling for network failures");
    println!("   • Implement token validation");
    println!("   • Add retries with exponential backoff");

    Ok(())
}
