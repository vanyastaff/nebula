//! Simple Registration Example
//!
//! This example demonstrates the simplified credential registration API.
//! Instead of manually wrapping credentials in CredentialAdapter,
//! you can use `register_credential()` for automatic type-safe registration.

use async_trait::async_trait;
use nebula_credential::core::{
    AccessToken, CredentialContext, CredentialError, CredentialMetadata, CredentialState,
    Result, SecureString,
};
use nebula_credential::testing::{MockLock, MockStateStore, MockTokenCache};
use nebula_credential::traits::{Credential, StateStore, TokenCache};
use nebula_credential::{CredentialManager, CredentialRegistry};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

// ═══════════════════════════════════════════════════════════════
// Define a simple API Key credential
// ═══════════════════════════════════════════════════════════════

#[derive(Serialize, Deserialize)]
struct ApiKeyInput {
    api_key: String,
    description: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct ApiKeyState {
    key: SecureString,
    description: String,
    created_at: SystemTime,
}

impl CredentialState for ApiKeyState {
    const KIND: &'static str = "api_key";
    const VERSION: u16 = 1;
}

struct ApiKeyCredential;

#[async_trait]
impl Credential for ApiKeyCredential {
    type Input = ApiKeyInput;
    type State = ApiKeyState;

    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata {
            id: "api_key",
            name: "API Key",
            description: "Simple API key authentication",
            supports_refresh: false,
            requires_interaction: false,
        }
    }

    async fn initialize(
        &self,
        input: &Self::Input,
        _ctx: &mut CredentialContext,
    ) -> Result<(Self::State, Option<AccessToken>)> {
        let state = ApiKeyState {
            key: SecureString::new(input.api_key.clone()),
            description: input.description.clone(),
            created_at: SystemTime::now(),
        };

        let token = AccessToken::bearer(input.api_key.clone())
            .with_expiration(SystemTime::now() + Duration::from_secs(86400)); // 24h

        Ok((state, Some(token)))
    }
}

// ═══════════════════════════════════════════════════════════════
// Define an OAuth2 credential
// ═══════════════════════════════════════════════════════════════

#[derive(Serialize, Deserialize)]
struct OAuth2Input {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct OAuth2State {
    client_id: String,
    client_secret: SecureString,
    access_token: SecureString,
    refresh_token: SecureString,
    expires_at: SystemTime,
}

impl CredentialState for OAuth2State {
    const KIND: &'static str = "oauth2";
    const VERSION: u16 = 1;
}

struct OAuth2Credential;

#[async_trait]
impl Credential for OAuth2Credential {
    type Input = OAuth2Input;
    type State = OAuth2State;

    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata {
            id: "oauth2",
            name: "OAuth 2.0",
            description: "OAuth 2.0 authentication with refresh",
            supports_refresh: true,
            requires_interaction: true,
        }
    }

    async fn initialize(
        &self,
        input: &Self::Input,
        _ctx: &mut CredentialContext,
    ) -> Result<(Self::State, Option<AccessToken>)> {
        // Simulate OAuth2 flow
        let state = OAuth2State {
            client_id: input.client_id.clone(),
            client_secret: SecureString::new(input.client_secret.clone()),
            access_token: SecureString::new("initial_access_token".to_string()),
            refresh_token: SecureString::new("refresh_token_123".to_string()),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
        };

        let token = AccessToken::bearer(state.access_token.expose().to_string())
            .with_expiration(state.expires_at);

        Ok((state, Some(token)))
    }

    async fn refresh(
        &self,
        state: &mut Self::State,
        _ctx: &mut CredentialContext,
    ) -> Result<AccessToken> {
        // Simulate refresh
        state.access_token = SecureString::new("refreshed_access_token".to_string());
        state.expires_at = SystemTime::now() + Duration::from_secs(3600);

        Ok(AccessToken::bearer(state.access_token.expose().to_string())
            .with_expiration(state.expires_at))
    }
}

// ═══════════════════════════════════════════════════════════════
// Main example
// ═══════════════════════════════════════════════════════════════

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║    Nebula Credential - Simple Registration Example      ║");
    println!("╚══════════════════════════════════════════════════════════╝\n");

    // ═══════════════════════════════════════════════════════════════
    // Old way: Manual factory registration
    // ═══════════════════════════════════════════════════════════════
    println!("❌ Old Way: Manual CredentialAdapter wrapping");
    println!("   registry.register(Arc::new(CredentialAdapter::new(ApiKeyCredential)));");
    println!("   registry.register(Arc::new(CredentialAdapter::new(OAuth2Credential)));\n");

    // ═══════════════════════════════════════════════════════════════
    // New way: Direct credential registration
    // ═══════════════════════════════════════════════════════════════
    println!("✨ New Way: Direct credential registration");
    let registry = Arc::new(CredentialRegistry::new());

    println!("   registry.register_credential(ApiKeyCredential);");
    registry.register_credential(ApiKeyCredential);

    println!("   registry.register_credential(OAuth2Credential);");
    registry.register_credential(OAuth2Credential);

    println!("\n   ✓ Two credentials registered with simplified API\n");

    // ═══════════════════════════════════════════════════════════════
    // Setup CredentialManager
    // ═══════════════════════════════════════════════════════════════
    println!("📦 Setting up CredentialManager...");
    let store = Arc::new(MockStateStore::new());
    let cache = Arc::new(MockTokenCache::new());

    let manager = CredentialManager::builder()
        .with_store(store as Arc<dyn StateStore>)
        .with_cache(cache as Arc<dyn TokenCache>)
        .with_lock(MockLock::new())
        .with_registry(registry.clone())
        .build()?;

    println!("   ✓ Manager ready\n");

    // ═══════════════════════════════════════════════════════════════
    // List registered credential types
    // ═══════════════════════════════════════════════════════════════
    println!("📋 Registered Credential Types:");
    for metadata in registry.list_metadata() {
        println!("\n   {} ({})", metadata.name, metadata.id);
        println!("      ├─ Description: {}", metadata.description);
        println!("      ├─ Supports refresh: {}", metadata.supports_refresh);
        println!("      └─ Requires interaction: {}", metadata.requires_interaction);
    }
    println!();

    // ═══════════════════════════════════════════════════════════════
    // Use the registered credentials
    // ═══════════════════════════════════════════════════════════════
    println!("\n🔑 Creating API Key credential...");
    let api_key_id = manager
        .create_credential(
            "api_key",
            json!({
                "api_key": "sk-1234567890abcdef",
                "description": "Production API key"
            }),
        )
        .await?;

    println!("   ✓ Created: {}", api_key_id);
    let token = manager.get_token(&api_key_id).await?;
    println!("   ✓ Token: {} chars", token.token.expose().len());

    println!("\n🔐 Creating OAuth2 credential...");
    let oauth_id = manager
        .create_credential(
            "oauth2",
            json!({
                "client_id": "my-client-id",
                "client_secret": "my-client-secret",
                "redirect_uri": "https://example.com/callback"
            }),
        )
        .await?;

    println!("   ✓ Created: {}", oauth_id);
    let token = manager.get_token(&oauth_id).await?;
    println!("   ✓ Token: {} chars", token.token.expose().len());

    // ═══════════════════════════════════════════════════════════════
    // Summary
    // ═══════════════════════════════════════════════════════════════
    println!("\n╔══════════════════════════════════════════════════════════╗");
    println!("║                        Summary                           ║");
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║ ✓ Simplified registration with register_credential()    ║");
    println!("║ ✓ No manual CredentialAdapter wrapping needed           ║");
    println!("║ ✓ Type-safe Credential trait implementation             ║");
    println!("║ ✓ Automatic factory conversion                          ║");
    println!("╚══════════════════════════════════════════════════════════╝");

    println!("\n💡 Benefits:");
    println!("   • Less boilerplate code");
    println!("   • Type safety preserved");
    println!("   • Cleaner, more intuitive API");
    println!("   • Automatic adapter wrapping");

    println!("\n💡 Both methods work:");
    println!("   • register_credential(cred) - Simple & recommended");
    println!("   • register(Arc::new(factory)) - Full control");

    Ok(())
}
