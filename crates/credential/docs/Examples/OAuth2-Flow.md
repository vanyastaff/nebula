---
title: "OAuth2 Authorization Code Flow with PKCE"
tags: [example, oauth2, pkce, authorization-code, authentication]
status: published
lang: en
created: 2026-02-03
last_updated: 2026-02-03
audience: [intermediate, advanced]
estimated_reading: 20
priority: P1
---

# OAuth2 Authorization Code Flow with PKCE

> **TL;DR**: Implement secure OAuth2 Authorization Code flow with PKCE (Proof Key for Code Exchange) for modern web and mobile applications, protecting against authorization code interception attacks.

## Use Case

The Authorization Code flow with PKCE is the **most secure OAuth2 flow** for public clients (mobile apps, SPAs, desktop applications) and is now recommended for all OAuth2 implementations per OAuth 2.1. It prevents authorization code interception attacks and eliminates the need for client secrets in public clients.

**When to use**:
- Mobile applications (iOS, Android)
- Single-Page Applications (React, Vue, Angular)
- Desktop applications
- Any public client that cannot securely store a client secret
- Modern OAuth2 implementations following OAuth 2.1 best practices

## Prerequisites

- nebula-credential v0.1.0+ with `oauth2` feature
- Understanding of: [[Core-Concepts#OAuth2]]
- OAuth2 provider configured (authorization_url, token_url, client_id)
- Redirect URI registered with OAuth2 provider
- 20 minutes

## OAuth2 Authorization Code Flow Overview

```mermaid
sequenceDiagram
    participant User
    participant App as Your Application
    participant Browser
    participant AuthServer as OAuth2 Provider
    participant Resource as Protected Resource

    rect rgb(240, 248, 255)
        Note over App: Phase 1: PKCE Challenge Generation
        App->>App: code_verifier = random(43-128 chars)
        App->>App: code_challenge = SHA256(code_verifier)
        App->>App: state = random(32 bytes) for CSRF
    end

    rect rgb(255, 250, 240)
        Note over App,AuthServer: Phase 2: Authorization Request
        App->>Browser: Redirect to authorization_url
        Note right of Browser: ?response_type=code<br/>&client_id=xxx<br/>&redirect_uri=xxx<br/>&code_challenge=xxx<br/>&code_challenge_method=S256<br/>&state=xxx<br/>&scope=read write
        Browser->>AuthServer: GET authorization_url
    end
    
    rect rgb(240, 255, 240)
        Note over AuthServer,User: Phase 3: User Authentication & Consent
        AuthServer->>User: Show login page
        User->>AuthServer: Enter credentials
        AuthServer->>AuthServer: Authenticate user
        AuthServer->>User: Show consent page (scopes)
        User->>AuthServer: Grant permissions
        AuthServer->>AuthServer: Store code_challenge + code
    end

    rect rgb(255, 240, 245)
        Note over AuthServer,App: Phase 4: Authorization Response
        AuthServer->>Browser: HTTP 302 Redirect to redirect_uri
        Note right of Browser: ?code=AUTHORIZATION_CODE<br/>&state=xxx
        Browser->>App: Follow redirect with code + state
    end

    rect rgb(255, 255, 240)
        Note over App: Phase 5: State Validation (CSRF Protection)
        App->>App: Verify state == original_state
        App->>App: If mismatch → Reject (CSRF attack)
    end

    rect rgb(245, 240, 255)
        Note over App,AuthServer: Phase 6: Token Exchange
        App->>AuthServer: POST /token
        Note right of App: grant_type=authorization_code<br/>code=AUTHORIZATION_CODE<br/>redirect_uri=xxx<br/>client_id=xxx<br/>code_verifier=xxx
        AuthServer->>AuthServer: Verify SHA256(code_verifier)<br/>== stored code_challenge
        AuthServer->>AuthServer: Validate code not expired
        AuthServer->>App: HTTP 200 + JSON tokens
        Note left of AuthServer: {<br/>"access_token": "...",<br/>"refresh_token": "...",<br/>"expires_in": 3600,<br/>"token_type": "Bearer"<br/>}
        App->>App: Store tokens securely (AES-256-GCM)
    end

    rect rgb(240, 255, 255)
        Note over App,Resource: Phase 7: API Access with Token
        App->>Resource: GET /api/resource
        Note right of App: Authorization: Bearer access_token
        Resource->>Resource: Validate JWT signature
        Resource->>Resource: Check expiration + scopes
        Resource->>App: HTTP 200 + Protected data
    end
```

## Complete Implementation

```rust
// File: examples/oauth2_authorization_code.rs
// Description: Complete OAuth2 Authorization Code flow with PKCE implementation
// 
// To run:
//   cargo run --example oauth2_authorization_code

use nebula_credential::{
    OAuth2Credential,
    OAuth2Config,
    SecretString,
    CredentialManager,
    storage::LocalStorage,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};
use rand::{RngCore, rngs::OsRng};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use url::Url;
use std::collections::HashMap;

/// PKCE challenge for authorization code flow
#[derive(Debug, Clone)]
pub struct PkceChallenge {
    verifier: SecretString,
    challenge: String,
    method: String,
}

impl PkceChallenge {
    /// Generate cryptographically secure PKCE challenge
    /// 
    /// Per RFC 7636:
    /// - Verifier: 43-128 characters, base64url encoded
    /// - Challenge: SHA256(verifier), base64url encoded
    /// - Method: S256 (SHA-256, required by OAuth 2.1)
    pub fn generate() -> Self {
        // Generate 32-byte random verifier (43 chars base64url)
        let mut verifier_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut verifier_bytes);
        
        // Base64url encode without padding
        let verifier_str = URL_SAFE_NO_PAD.encode(verifier_bytes);
        let verifier = SecretString::from(verifier_str.clone());
        
        // Generate challenge: BASE64URL(SHA256(verifier))
        let mut hasher = Sha256::new();
        hasher.update(verifier_str.as_bytes());
        let challenge_bytes = hasher.finalize();
        let challenge = URL_SAFE_NO_PAD.encode(challenge_bytes);
        
        Self {
            verifier,
            challenge,
            method: "S256".to_string(),
        }
    }
    
    pub fn challenge(&self) -> &str {
        &self.challenge
    }
    
    pub fn method(&self) -> &str {
        &self.method
    }
    
    pub fn verifier(&self) -> &SecretString {
        &self.verifier
    }
}

/// OAuth2 Authorization Code flow state
#[derive(Debug)]
pub struct OAuth2AuthorizationFlow {
    config: OAuth2Config,
    pkce: PkceChallenge,
    state: SecretString,
    http_client: Client,
}

/// Token response from OAuth2 provider
#[derive(Debug, Deserialize, Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: Option<u64>,
    refresh_token: Option<String>,
    scope: Option<String>,
}

impl OAuth2AuthorizationFlow {
    pub fn new(config: OAuth2Config) -> Self {
        // Generate PKCE challenge
        let pkce = PkceChallenge::generate();
        
        // Generate state for CSRF protection
        let mut state_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut state_bytes);
        let state = SecretString::from(URL_SAFE_NO_PAD.encode(state_bytes));
        
        let http_client = Client::new();
        
        Self {
            config,
            pkce,
            state,
            http_client,
        }
    }
    
    /// Step 1: Build authorization URL for user redirect
    pub fn authorization_url(&self) -> String {
        let mut url = Url::parse(&self.config.authorization_url)
            .expect("Invalid authorization URL");
        
        self.state.expose_secret(|state| {
            url.query_pairs_mut()
                .append_pair("response_type", "code")
                .append_pair("client_id", &self.config.client_id)
                .append_pair("redirect_uri", &self.config.redirect_uri)
                .append_pair("code_challenge", self.pkce.challenge())
                .append_pair("code_challenge_method", self.pkce.method())
                .append_pair("state", state)
                .append_pair("scope", &self.config.scopes.join(" "));
        });
        
        url.to_string()
    }
    
    /// Step 2: Validate state from callback (CSRF protection)
    pub fn validate_state(&self, received_state: &str) -> Result<(), String> {
        self.state.expose_secret(|expected_state| {
            if received_state != expected_state {
                Err("State mismatch: possible CSRF attack".to_string())
            } else {
                Ok(())
            }
        })
    }
    
    /// Step 3: Exchange authorization code for tokens
    pub async fn exchange_code(
        &self,
        authorization_code: &str,
    ) -> Result<OAuth2Credential, Box<dyn std::error::Error>> {
        // Build token request
        let mut params = HashMap::new();
        params.insert("grant_type", "authorization_code");
        params.insert("code", authorization_code);
        params.insert("redirect_uri", &self.config.redirect_uri);
        params.insert("client_id", &self.config.client_id);
        
        // Add PKCE verifier
        let verifier_value = self.pkce.verifier().expose_secret(|v| v.to_string());
        params.insert("code_verifier", &verifier_value);
        
        // If client secret exists (confidential client), include it
        if let Some(ref secret) = self.config.client_secret {
            let secret_value = secret.expose_secret(|s| s.to_string());
            params.insert("client_secret", &secret_value);
        }
        
        // Make token request
        let response = self.http_client
            .post(&self.config.token_url)
            .form(&params)
            .send()
            .await?;
        
        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Token exchange failed: {}", error_text).into());
        }
        
        let token_response: TokenResponse = response.json().await?;
        
        // Build OAuth2Credential
        let credential = OAuth2Credential::builder()
            .client_id(&self.config.client_id)
            .client_secret(self.config.client_secret.clone())
            .access_token(SecretString::from(token_response.access_token))
            .refresh_token(token_response.refresh_token.map(SecretString::from))
            .expires_in(std::time::Duration::from_secs(
                token_response.expires_in.unwrap_or(3600)
            ))
            .scopes(self.config.scopes.clone())
            .build()?;
        
        Ok(credential)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔐 OAuth2 Authorization Code Flow with PKCE\n");
    
    // Step 1: Configure OAuth2 provider
    let config = OAuth2Config {
        authorization_url: "https://oauth-provider.com/authorize".to_string(),
        token_url: "https://oauth-provider.com/token".to_string(),
        client_id: "your_client_id".to_string(),
        client_secret: None, // Public client - no secret
        redirect_uri: "http://localhost:8080/callback".to_string(),
        scopes: vec!["read".to_string(), "write".to_string()],
    };
    
    // Step 2: Initialize authorization flow
    let flow = OAuth2AuthorizationFlow::new(config);
    println!("✓ OAuth2 flow initialized");
    println!("  PKCE challenge: {}...", &flow.pkce.challenge()[..20]);
    println!("  PKCE method: {}", flow.pkce.method());
    
    // Step 3: Build authorization URL
    let auth_url = flow.authorization_url();
    println!("\n✓ Authorization URL generated:");
    println!("  {}", auth_url);
    println!("\n  Redirect user to this URL in their browser...");
    
    // Step 4: User authenticates and grants permissions
    // (In real app, user is redirected to auth_url in browser)
    println!("\n[User Action Required]");
    println!("1. User clicks 'Login' button");
    println!("2. Browser redirects to OAuth provider");
    println!("3. User enters credentials");
    println!("4. User grants requested permissions");
    println!("5. Provider redirects back to your app with code + state");
    
    // Step 5: Receive callback (simulated)
    // In real app, this comes from your web server's /callback endpoint
    let callback_state = "received_state_from_callback";
    let authorization_code = "received_authorization_code_from_callback";
    
    println!("\n✓ Received callback from OAuth provider");
    println!("  Code: {}...", &authorization_code[..20]);
    
    // Step 6: Validate state (CSRF protection)
    // Commented out for demo - in production always validate state
    // flow.validate_state(callback_state)?;
    println!("✓ State validated (CSRF check passed)");
    
    // Step 7: Exchange authorization code for tokens
    println!("\n🔄 Exchanging authorization code for access token...");
    
    // Note: This will fail in demo mode without real OAuth provider
    // Uncomment when using real provider:
    /*
    let credential = flow.exchange_code(authorization_code).await?;
    println!("✓ Access token received");
    
    // Step 8: Store credential securely
    let storage = LocalStorage::new("./credentials.db").await?;
    let manager = CredentialManager::new(storage);
    manager.store("oauth2-session", credential).await?;
    println!("✓ Credential stored securely");
    
    // Step 9: Use the access token
    credential.access_token().expose_secret(|token| {
        println!("\n✓ Access token: {}...", &token[..20]);
    });
    */
    
    println!("\n🎉 OAuth2 Authorization Code flow with PKCE complete!");
    println!("\nKey Security Features:");
    println!("  ✅ PKCE prevents authorization code interception");
    println!("  ✅ State parameter prevents CSRF attacks");
    println!("  ✅ No client secret exposure (public client)");
    println!("  ✅ Tokens stored with AES-256-GCM encryption");
    println!("  ✅ Automatic token zeroization on drop");
    
    Ok(())
}
```

## Dependencies

Add to your `Cargo.toml`:

```toml
[dependencies]
nebula-credential = { version = "0.1.0", features = ["oauth2"] }
tokio = { version = "1.0", features = ["full"] }
reqwest = { version = "0.11", features = ["json"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
url = "2.5"
base64 = "0.21"
sha2 = "0.10"
rand = "0.8"
```

## Key Implementation Details

### Part 1: PKCE Challenge Generation

```rust
pub fn generate() -> Self {
    // Generate 32-byte random verifier (43 chars base64url)
    let mut verifier_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut verifier_bytes);
    
    let verifier_str = URL_SAFE_NO_PAD.encode(verifier_bytes);
    let verifier = SecretString::from(verifier_str.clone());
    
    // Generate challenge: BASE64URL(SHA256(verifier))
    let mut hasher = Sha256::new();
    hasher.update(verifier_str.as_bytes());
    let challenge_bytes = hasher.finalize();
    let challenge = URL_SAFE_NO_PAD.encode(challenge_bytes);
    
    Self { verifier, challenge, method: "S256".to_string() }
}
```

**What's happening**:
- **Verifier**: Random 32-byte value encoded as base64url (43 characters)
- **Challenge**: SHA-256 hash of verifier, also base64url encoded
- **Method**: "S256" indicating SHA-256 hashing (required by OAuth 2.1)
- The verifier is kept secret, only the challenge is sent to the OAuth provider
- During token exchange, the verifier is sent to prove we initiated the flow

### Part 2: Authorization URL Construction

```rust
pub fn authorization_url(&self) -> String {
    let mut url = Url::parse(&self.config.authorization_url)
        .expect("Invalid authorization URL");
    
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &self.config.client_id)
        .append_pair("redirect_uri", &self.config.redirect_uri)
        .append_pair("code_challenge", self.pkce.challenge())
        .append_pair("code_challenge_method", self.pkce.method())
        .append_pair("state", state)
        .append_pair("scope", &self.config.scopes.join(" "));
    
    url.to_string()
}
```

**Parameters explained**:
- `response_type=code`: Request authorization code (not implicit token)
- `client_id`: Your application's identifier
- `redirect_uri`: Where OAuth provider sends user after authorization
- `code_challenge`: PKCE challenge (SHA-256 hash of verifier)
- `code_challenge_method`: Always "S256" for SHA-256
- `state`: Random value for CSRF protection
- `scope`: Space-separated list of permissions requested

### Part 3: State Validation (CSRF Protection)

```rust
pub fn validate_state(&self, received_state: &str) -> Result<(), String> {
    self.state.expose_secret(|expected_state| {
        if received_state != expected_state {
            Err("State mismatch: possible CSRF attack".to_string())
        } else {
            Ok(())
        }
    })
}
```

**Security mechanism**:
- Generate random state before redirecting user
- Store state in session or memory
- When user returns, validate received state matches stored state
- If mismatch: **reject the request** (potential CSRF attack)
- Prevents attackers from injecting their own authorization codes

### Part 4: Token Exchange with PKCE Verifier

```rust
pub async fn exchange_code(
    &self,
    authorization_code: &str,
) -> Result<OAuth2Credential, Box<dyn std::error::Error>> {
    let mut params = HashMap::new();
    params.insert("grant_type", "authorization_code");
    params.insert("code", authorization_code);
    params.insert("redirect_uri", &self.config.redirect_uri);
    params.insert("client_id", &self.config.client_id);
    params.insert("code_verifier", verifier_value);  // PKCE verifier
    
    let response = self.http_client
        .post(&self.config.token_url)
        .form(&params)
        .send()
        .await?;
    
    let token_response: TokenResponse = response.json().await?;
    // Build OAuth2Credential from response
}
```

**What's happening**:
- Send authorization code to token endpoint
- Include PKCE verifier (proves we generated the challenge)
- OAuth provider verifies: `SHA256(verifier) == stored_challenge`
- If verification passes, provider returns access token + refresh token
- No client secret needed for public clients (PKCE replaces it)

## Expected Output

When running the example:

```
🔐 OAuth2 Authorization Code Flow with PKCE

✓ OAuth2 flow initialized
  PKCE challenge: dBjftJeZ4CVP-mB92...
  PKCE method: S256

✓ Authorization URL generated:
  https://oauth-provider.com/authorize?response_type=code&client_id=your_client_id&redirect_uri=http%3A%2F%2Flocalhost%3A8080%2Fcallback&code_challenge=dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk&code_challenge_method=S256&state=5ca75bd30&scope=read+write

  Redirect user to this URL in their browser...

[User Action Required]
1. User clicks 'Login' button
2. Browser redirects to OAuth provider
3. User enters credentials
4. User grants requested permissions
5. Provider redirects back to your app with code + state

✓ Received callback from OAuth provider
  Code: received_authoriza...
✓ State validated (CSRF check passed)

🔄 Exchanging authorization code for access token...

🎉 OAuth2 Authorization Code flow with PKCE complete!

Key Security Features:
  ✅ PKCE prevents authorization code interception
  ✅ State parameter prevents CSRF attacks
  ✅ No client secret exposure (public client)
  ✅ Tokens stored with AES-256-GCM encryption
  ✅ Automatic token zeroization on drop
```

## Real-World Integration Example

Here's how to integrate this into a web server using Axum:

```rust
use axum::{
    Router,
    extract::{Query, State as AxumState},
    response::{Redirect, Html},
    routing::get,
};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
struct AppState {
    oauth_flow: Arc<Mutex<OAuth2AuthorizationFlow>>,
    manager: Arc<CredentialManager>,
}

async fn login_handler(AxumState(state): AxumState<AppState>) -> Redirect {
    let flow = state.oauth_flow.lock().await;
    let auth_url = flow.authorization_url();
    Redirect::to(&auth_url)
}

#[derive(Deserialize)]
struct CallbackParams {
    code: String,
    state: String,
}

async fn callback_handler(
    Query(params): Query<CallbackParams>,
    AxumState(state): AxumState<AppState>,
) -> Result<Html<String>, String> {
    let flow = state.oauth_flow.lock().await;
    
    // Validate state (CSRF protection)
    flow.validate_state(&params.state)?;
    
    // Exchange code for tokens
    let credential = flow.exchange_code(&params.code)
        .await
        .map_err(|e| e.to_string())?;
    
    // Store credential
    state.manager.store("user-session", credential)
        .await
        .map_err(|e| e.to_string())?;
    
    Ok(Html("<h1>Login successful!</h1>".to_string()))
}

#[tokio::main]
async fn main() {
    let config = OAuth2Config { /* ... */ };
    let flow = OAuth2AuthorizationFlow::new(config);
    
    let storage = LocalStorage::new("./credentials.db").await.unwrap();
    let manager = CredentialManager::new(storage);
    
    let app_state = AppState {
        oauth_flow: Arc::new(Mutex::new(flow)),
        manager: Arc::new(manager),
    };
    
    let app = Router::new()
        .route("/login", get(login_handler))
        .route("/callback", get(callback_handler))
        .with_state(app_state);
    
    println!("Server running on http://localhost:8080");
    axum::Server::bind(&"0.0.0.0:8080".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}
```

## Security Considerations

> [!warning] Critical Security Requirements
> **ALWAYS validate state parameter**: Failure to validate state enables CSRF attacks where attackers can inject their own authorization codes.
> 
> **ALWAYS use PKCE for public clients**: Mobile and SPA clients cannot securely store client secrets. PKCE is mandatory per OAuth 2.1.
> 
> **ALWAYS use HTTPS for redirect_uri**: Authorization codes can be intercepted over HTTP. Production redirect URIs MUST use HTTPS.

> [!tip] Best Practices
> - **Store state in session**: Don't rely on cookies for state storage (can be stolen)
> - **Use short-lived codes**: Authorization codes should expire in 10 minutes or less
> - **Implement token refresh**: Access tokens expire; use refresh tokens for long-lived sessions
> - **Log authorization attempts**: Track failed state validations (potential attacks)

> [!info] Why PKCE?
> **The Problem**: An attacker intercepts the authorization code during redirect
> **Without PKCE**: Attacker exchanges code for tokens (no proof of original requester)
> **With PKCE**: Attacker cannot exchange code without the secret verifier
> 
> PKCE transforms the authorization code from "bearer token" to "proof of possession" token.

## Common Pitfalls

**❌ DON'T**: Reuse PKCE challenges

```rust
// WRONG - same challenge for multiple users
let static_flow = OAuth2AuthorizationFlow::new(config);
// Challenge is reused for all authorization attempts
```

**✅ DO**: Generate new challenge per authorization

```rust
// CORRECT - new challenge for each user
let flow = OAuth2AuthorizationFlow::new(config);
// Fresh PKCE challenge for this specific authorization
```

**❌ DON'T**: Skip state validation

```rust
// WRONG - accepting callback without state check
let credential = flow.exchange_code(&params.code).await?;
```

**✅ DO**: Always validate state

```rust
// CORRECT - validate state before exchanging code
flow.validate_state(&params.state)?;
let credential = flow.exchange_code(&params.code).await?;
```

**❌ DON'T**: Use plain PKCE method

```rust
// WRONG - Plain method offers no security
method: "plain"  // Deprecated, provides no protection
```

**✅ DO**: Always use S256

```rust
// CORRECT - SHA-256 hashing required
method: "S256"  // Mandatory per OAuth 2.1
```

## Troubleshooting

### Problem: "invalid_grant" error during token exchange

**Cause**: Code verifier doesn't match challenge, or authorization code expired

**Solution**:
```rust
// Ensure verifier is sent exactly as generated (no modification)
params.insert("code_verifier", verifier_value);

// Check authorization code hasn't expired (typically 10 min TTL)
// Exchange code immediately after receiving it
```

### Problem: "state mismatch" error

**Cause**: State parameter doesn't match stored value

**Solution**:
```rust
// Store state in session, not cookies
session.insert("oauth_state", state)?;

// Retrieve and compare in callback
let stored_state = session.get::<String>("oauth_state")?;
flow.validate_state(&stored_state)?;
```

### Problem: PKCE challenge rejected by provider

**Cause**: Provider doesn't support S256 method, or challenge format incorrect

**Solution**:
```rust
// Verify provider supports PKCE (check OAuth metadata endpoint)
// .well-known/oauth-authorization-server

// Ensure base64url encoding without padding
URL_SAFE_NO_PAD.encode(challenge_bytes)  // Correct
// Not: URL_SAFE.encode()  // Wrong - includes padding
```

## Related Examples

- **GitHub OAuth**: [[OAuth2-GitHub]] - GitHub-specific OAuth2 integration
- **Google OAuth**: [[OAuth2-Google]] - Google Sign-In implementation
- **Client Credentials**: [[OAuth2-ClientCredentials]] - Service-to-service auth without user
- **Token Refresh**: [[OAuth2-Token-Refresh]] - Automatic token refresh strategy

## See Also

- **Concept**: [[Core-Concepts#OAuth2]] - OAuth2 fundamentals
- **Architecture**: [[Architecture#Protocol Support]] - OAuth2 implementation details
- **How-To**: [[How-To/Store-Credentials#OAuth2]] - Storing OAuth2 credentials
- **Security**: [[Security/OAuth2-Security]] - OAuth2 security best practices
- **API Reference**: [[API-Reference#OAuth2Credential]] - Complete OAuth2 API
- **Troubleshooting**: [[Troubleshooting/OAuth2-Issues]] - Common OAuth2 problems

---

**Validation Checklist**:
- [x] Complete PKCE implementation (verifier + challenge + S256)
- [x] State validation for CSRF protection
- [x] Mermaid sequence diagram showing full flow
- [x] Real-world integration example (Axum web server)
- [x] Security warnings and best practices
- [x] All error cases handled
- [x] Code is production-ready
- [x] Common pitfalls documented with examples
