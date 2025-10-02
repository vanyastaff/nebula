//! Authenticator Usage Example
//!
//! This example demonstrates the ClientAuthenticator pattern for creating
//! authenticated clients from tokens. It shows:
//! - Basic authenticator usage
//! - AuthenticateWith extension trait
//! - Chain authenticators for composition
//! - Custom authenticator implementation

use async_trait::async_trait;
use nebula_credential::authenticator::{ApiKeyHeader, AuthenticateWith, ChainAuthenticator, ClientAuthenticator, HttpBearer};
use nebula_credential::core::{AccessToken, CredentialError, SecureString, TokenType};
use nebula_credential::testing::{MockLock, MockStateStore, MockTokenCache, TestCredentialFactory};
use nebula_credential::traits::{StateStore, TokenCache};
use nebula_credential::{CredentialManager, CredentialRegistry};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

// ═══════════════════════════════════════════════════════════════
// Custom authenticators
// ═══════════════════════════════════════════════════════════════

/// Simple mock HTTP request
#[derive(Debug, Clone)]
struct MockRequest {
    headers: HashMap<String, String>,
}

impl MockRequest {
    fn new() -> Self {
        Self {
            headers: HashMap::new(),
        }
    }

    fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    fn get_header(&self, key: &str) -> Option<&String> {
        self.headers.get(key)
    }
}

/// Custom authenticator that adds Bearer token
struct BearerAuthenticator;

#[async_trait]
impl ClientAuthenticator for BearerAuthenticator {
    type Target = MockRequest;
    type Output = MockRequest;

    async fn authenticate(
        &self,
        request: Self::Target,
        token: &AccessToken,
    ) -> Result<Self::Output, CredentialError> {
        if !matches!(token.token_type, TokenType::Bearer) {
            return Err(CredentialError::InvalidConfiguration {
                reason: "BearerAuthenticator requires Bearer token".to_string(),
            });
        }

        let auth_value = token.token.with_exposed(|s| format!("Bearer {s}"));
        Ok(request.header("Authorization", auth_value))
    }
}

/// Custom authenticator that adds API version header
struct ApiVersionAuthenticator {
    version: String,
}

impl ApiVersionAuthenticator {
    fn new(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
        }
    }
}

#[async_trait]
impl ClientAuthenticator for ApiVersionAuthenticator {
    type Target = MockRequest;
    type Output = MockRequest;

    async fn authenticate(
        &self,
        request: Self::Target,
        _token: &AccessToken,
    ) -> Result<Self::Output, CredentialError> {
        Ok(request.header("X-API-Version", &self.version))
    }
}

/// Custom authenticator that creates a client from token
struct MockClientAuthenticator;

#[derive(Debug)]
struct MockClient {
    token: String,
    created_at: SystemTime,
}

#[async_trait]
impl ClientAuthenticator for MockClientAuthenticator {
    type Target = ();
    type Output = MockClient;

    async fn authenticate(
        &self,
        _target: Self::Target,
        token: &AccessToken,
    ) -> Result<Self::Output, CredentialError> {
        Ok(MockClient {
            token: token.token.with_exposed(ToString::to_string),
            created_at: SystemTime::now(),
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// Main example
// ═══════════════════════════════════════════════════════════════

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║    Nebula Credential - Authenticator Usage Example      ║");
    println!("╚══════════════════════════════════════════════════════════╝\n");

    // ═══════════════════════════════════════════════════════════════
    // Setup CredentialManager
    // ═══════════════════════════════════════════════════════════════
    println!("📦 Setup: Creating CredentialManager");
    let store = Arc::new(MockStateStore::new());
    let cache = Arc::new(MockTokenCache::new());
    let registry = Arc::new(CredentialRegistry::new());

    registry.register(Arc::new(TestCredentialFactory::new()));

    let manager = CredentialManager::builder()
        .with_store(store as Arc<dyn StateStore>)
        .with_cache(cache as Arc<dyn TokenCache>)
        .with_lock(MockLock::new())
        .with_registry(registry)
        .build()?;

    // Create a test credential
    let cred_id = manager
        .create_credential(
            "test_credential",
            json!({
                "value": "secret-api-key-12345",
                "should_fail": false
            }),
        )
        .await?;

    println!("   ✓ Credential created: {}", cred_id);

    // Get token
    let token = manager.get_token(&cred_id).await?;
    println!("   ✓ Token retrieved\n");

    // ═══════════════════════════════════════════════════════════════
    // Example 1: Basic Authenticator Usage
    // ═══════════════════════════════════════════════════════════════
    println!("🔐 Example 1: Basic Authenticator Usage");

    let authenticator = BearerAuthenticator;
    let request = MockRequest::new();

    let authenticated = authenticator.authenticate(request, &token).await?;

    println!("   Request headers:");
    for (key, value) in &authenticated.headers {
        println!("      {}: {}", key, value);
    }
    println!();

    // ═══════════════════════════════════════════════════════════════
    // Example 2: AuthenticateWith Extension Trait
    // ═══════════════════════════════════════════════════════════════
    println!("✨ Example 2: AuthenticateWith Extension Trait");
    println!("   Using fluent API: request.authenticate_with(&auth, &token)\n");

    let request = MockRequest::new();
    let authenticated = request
        .authenticate_with(&BearerAuthenticator, &token)
        .await?;

    println!("   Request headers:");
    for (key, value) in &authenticated.headers {
        println!("      {}: {}", key, value);
    }
    println!();

    // ═══════════════════════════════════════════════════════════════
    // Example 3: Chain Authenticators
    // ═══════════════════════════════════════════════════════════════
    println!("🔗 Example 3: Chain Authenticators");
    println!("   Composing: Bearer auth + API version header\n");

    let chain = ChainAuthenticator::new(BearerAuthenticator, ApiVersionAuthenticator::new("v1"));

    let request = MockRequest::new();
    let authenticated = request.authenticate_with(&chain, &token).await?;

    println!("   Request headers:");
    for (key, value) in &authenticated.headers {
        println!("      {}: {}", key, value);
    }

    assert!(authenticated.get_header("Authorization").is_some());
    assert_eq!(
        authenticated.get_header("X-API-Version"),
        Some(&"v1".to_string())
    );
    println!("   ✓ Both authenticators applied\n");

    // ═══════════════════════════════════════════════════════════════
    // Example 4: Create Client from Token
    // ═══════════════════════════════════════════════════════════════
    println!("🏗️  Example 4: Create Client from Token");

    let client = ()
        .authenticate_with(&MockClientAuthenticator, &token)
        .await?;

    println!("   ✓ Client created:");
    println!("      ├─ Token: {} chars", client.token.len());
    println!("      └─ Created at: {:?}", client.created_at);
    println!();

    // ═══════════════════════════════════════════════════════════════
    // Example 5: Multiple Authenticators for Different Token Types
    // ═══════════════════════════════════════════════════════════════
    println!("🔑 Example 5: Different Token Types");

    // Create different token types
    let bearer_token = AccessToken::bearer("bearer-token-123".to_string())
        .with_expiration(SystemTime::now() + Duration::from_secs(3600));

    let api_key_token = AccessToken {
        token: SecureString::new("api-key-456".to_string()),
        token_type: TokenType::ApiKey,
        issued_at: SystemTime::now(),
        expires_at: Some(SystemTime::now() + Duration::from_secs(3600)),
        scopes: None,
        claims: Default::default(),
    };

    println!("   Bearer token with BearerAuthenticator:");
    let request = MockRequest::new();
    match request.authenticate_with(&BearerAuthenticator, &bearer_token).await {
        Ok(req) => println!("      ✓ {}", req.get_header("Authorization").unwrap()),
        Err(e) => println!("      ✗ Error: {:?}", e),
    }

    println!("\n   API Key token with BearerAuthenticator:");
    let request = MockRequest::new();
    match request.authenticate_with(&BearerAuthenticator, &api_key_token).await {
        Ok(_) => println!("      ✓ Success"),
        Err(e) => println!("      ✗ Expected error: {:?}", e),
    }
    println!();

    // ═══════════════════════════════════════════════════════════════
    // Example 6: Real-world Integration Pattern
    // ═══════════════════════════════════════════════════════════════
    println!("🌐 Example 6: Real-world Integration Pattern\n");

    println!("   Typical usage in application:");
    println!("   ```rust");
    println!("   // 1. Get token from credential manager");
    println!("   let token = manager.get_token(&cred_id).await?;");
    println!();
    println!("   // 2. Create HTTP request");
    println!("   let request = client.get(\"https://api.example.com/data\");");
    println!();
    println!("   // 3. Authenticate using appropriate authenticator");
    println!("   let auth_request = request");
    println!("       .authenticate_with(&HttpBearer, &token)");
    println!("       .await?;");
    println!();
    println!("   // 4. Send request");
    println!("   let response = auth_request.send().await?;");
    println!("   ```\n");

    // ═══════════════════════════════════════════════════════════════
    // Summary
    // ═══════════════════════════════════════════════════════════════
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║                        Summary                           ║");
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║ ✓ ClientAuthenticator separates token from auth logic   ║");
    println!("║ ✓ AuthenticateWith provides fluent API                  ║");
    println!("║ ✓ ChainAuthenticator enables composition                ║");
    println!("║ ✓ Type-safe: Target → Output mapping                    ║");
    println!("║ ✓ Flexible: Works with any client type                  ║");
    println!("╚══════════════════════════════════════════════════════════╝");

    println!("\n💡 Key Benefits:");
    println!("   • Separation of concerns: tokens vs authentication");
    println!("   • Composable: chain multiple authenticators");
    println!("   • Testable: easy to mock authenticators");
    println!("   • Type-safe: compile-time verification");
    println!("   • Reusable: common patterns in core, custom in nodes");

    println!("\n💡 Common Authenticators:");
    println!("   • HttpBearer - Standard Bearer token authentication");
    println!("   • ApiKeyHeader - Custom header-based API keys");
    println!("   • ChainAuthenticator - Compose multiple authenticators");
    println!("   • Custom - Implement for your specific use case");

    println!("\n💡 Real-world Use Cases:");
    println!("   • OpenAI: Bearer token + organization header");
    println!("   • AWS: SigV4 request signing");
    println!("   • Telegram: Bot token → teloxide::Bot client");
    println!("   • Custom APIs: Your specific authentication needs");

    Ok(())
}
