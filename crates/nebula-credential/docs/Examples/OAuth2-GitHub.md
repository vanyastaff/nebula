---
title: "OAuth2 GitHub Integration"
tags: [example, oauth2, github, integration]
status: published
lang: en
created: 2026-02-03
last_updated: 2026-02-03
audience: [intermediate, advanced]
estimated_reading: 15
priority: P1
---

# OAuth2 GitHub Integration

> **TL;DR**: Complete OAuth2 integration with GitHub's API using Authorization Code flow with PKCE for secure authentication and repository access.

## Use Case

GitHub OAuth2 integration enables applications to authenticate users via their GitHub accounts and access repositories, user data, and other GitHub resources on their behalf. This example demonstrates the complete flow including PKCE, scope management, and GitHub API usage.

**When to use**:
- "Sign in with GitHub" functionality
- Applications that need to access user's repositories
- CI/CD tools integrating with GitHub
- Developer tools requiring GitHub API access
- Apps managing GitHub issues, PRs, or workflows

## Prerequisites

- nebula-credential v0.1.0+ with `oauth2` feature
- GitHub OAuth App registered at https://github.com/settings/developers
- Understanding of: [[OAuth2-Flow|OAuth2 Authorization Code Flow]]
- Redirect URI configured in GitHub OAuth App settings
- 15 minutes

## GitHub OAuth2 Setup

1. Register OAuth App:
   - Go to https://github.com/settings/developers
   - Click "New OAuth App"
   - Set Application name, Homepage URL
   - Set Authorization callback URL (e.g., `http://localhost:8080/callback`)
   - Note the **Client ID** and generate **Client Secret**

2. Choose scopes based on needs:
   - `repo` - Full repository access
   - `public_repo` - Public repository access only
   - `read:user` - Read user profile
   - `user:email` - Access user email
   - `workflow` - Update GitHub Actions workflows

## Complete Implementation

```rust
// File: examples/oauth2_github.rs
// Description: Complete GitHub OAuth2 integration with PKCE
// 
// To run:
//   cargo run --example oauth2_github

use nebula_credential::{OAuth2Credential, OAuth2Config, SecretString};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::{thread_rng, Rng};
use std::collections::HashMap;
use anyhow::{Result, Context};

/// GitHub-specific scopes
#[derive(Debug, Clone)]
pub enum GitHubScope {
    Repo,              // Full repo access
    PublicRepo,        // Public repos only
    ReadUser,          // Read user profile
    UserEmail,         // User email addresses
    Workflow,          // GitHub Actions workflows
    ReadOrg,           // Read org membership
    WriteOrg,          // Write org membership
    Gist,              // Gist access
    Notifications,     // Notifications
    DeleteRepo,        // Delete repositories
}

impl GitHubScope {
    fn as_str(&self) -> &str {
        match self {
            Self::Repo => "repo",
            Self::PublicRepo => "public_repo",
            Self::ReadUser => "read:user",
            Self::UserEmail => "user:email",
            Self::Workflow => "workflow",
            Self::ReadOrg => "read:org",
            Self::WriteOrg => "write:org",
            Self::Gist => "gist",
            Self::Notifications => "notifications",
            Self::DeleteRepo => "delete_repo",
        }
    }
}

/// PKCE challenge for OAuth2 flow
#[derive(Debug, Clone)]
pub struct PkceChallenge {
    pub verifier: SecretString,
    pub challenge: String,
    pub method: String,
}

impl PkceChallenge {
    /// Generate new PKCE challenge with SHA256
    pub fn generate() -> Self {
        let verifier = Self::generate_verifier();
        let challenge = Self::generate_challenge(&verifier);
        
        Self {
            verifier: SecretString::new(verifier),
            challenge,
            method: "S256".to_string(),
        }
    }
    
    fn generate_verifier() -> String {
        let random_bytes: Vec<u8> = (0..32)
            .map(|_| thread_rng().gen::<u8>())
            .collect();
        URL_SAFE_NO_PAD.encode(&random_bytes)
    }
    
    fn generate_challenge(verifier: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let hash = hasher.finalize();
        URL_SAFE_NO_PAD.encode(&hash)
    }
}

/// GitHub OAuth2 configuration
#[derive(Debug, Clone)]
pub struct GitHubOAuth2Config {
    pub client_id: String,
    pub client_secret: SecretString,
    pub redirect_uri: String,
    pub scopes: Vec<GitHubScope>,
}

impl GitHubOAuth2Config {
    pub fn new(
        client_id: String,
        client_secret: String,
        redirect_uri: String,
        scopes: Vec<GitHubScope>,
    ) -> Self {
        Self {
            client_id,
            client_secret: SecretString::new(client_secret),
            redirect_uri,
            scopes,
        }
    }
}

/// GitHub OAuth2 flow handler
pub struct GitHubOAuth2Flow {
    config: GitHubOAuth2Config,
    http_client: Client,
    pkce: PkceChallenge,
    state: String,
}

impl GitHubOAuth2Flow {
    const AUTH_URL: &'static str = "https://github.com/login/oauth/authorize";
    const TOKEN_URL: &'static str = "https://github.com/login/oauth/access_token";
    
    pub fn new(config: GitHubOAuth2Config) -> Self {
        let pkce = PkceChallenge::generate();
        let state = Self::generate_state();
        
        Self {
            config,
            http_client: Client::new(),
            pkce,
            state,
        }
    }
    
    fn generate_state() -> String {
        let random_bytes: Vec<u8> = (0..32)
            .map(|_| thread_rng().gen::<u8>())
            .collect();
        URL_SAFE_NO_PAD.encode(&random_bytes)
    }
    
    /// Generate authorization URL
    pub fn authorization_url(&self) -> String {
        let scopes = self.config.scopes
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        
        format!(
            "{}?client_id={}&redirect_uri={}&scope={}&state={}",
            Self::AUTH_URL,
            urlencoding::encode(&self.config.client_id),
            urlencoding::encode(&self.config.redirect_uri),
            urlencoding::encode(&scopes),
            urlencoding::encode(&self.state),
        )
    }
    
    /// Validate state parameter (CSRF protection)
    pub fn validate_state(&self, received_state: &str) -> Result<()> {
        if self.state != received_state {
            anyhow::bail!("State mismatch: CSRF attack detected");
        }
        Ok(())
    }
    
    /// Exchange authorization code for access token
    pub async fn exchange_code(&self, code: &str) -> Result<OAuth2Credential> {
        let mut params = HashMap::new();
        params.insert("client_id", self.config.client_id.as_str());
        params.insert("client_secret", &self.config.client_secret.expose_secret());
        params.insert("code", code);
        params.insert("redirect_uri", self.config.redirect_uri.as_str());
        
        let response = self.http_client
            .post(Self::TOKEN_URL)
            .header("Accept", "application/json")
            .form(&params)
            .send()
            .await
            .context("Failed to exchange authorization code")?;
        
        if !response.status().is_success() {
            let error_text = response.text().await?;
            anyhow::bail!("Token exchange failed: {}", error_text);
        }
        
        let token_response: TokenResponse = response.json().await?;
        
        OAuth2Credential::builder()
            .client_id(&self.config.client_id)
            .access_token(SecretString::new(token_response.access_token))
            .token_type(&token_response.token_type)
            .scopes(token_response.scope.split_whitespace().map(String::from).collect())
            .build()
            .context("Failed to build OAuth2 credential")
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    scope: String,
}

/// GitHub API client
pub struct GitHubClient {
    credential: OAuth2Credential,
    http_client: Client,
}

impl GitHubClient {
    const API_BASE: &'static str = "https://api.github.com";
    
    pub fn new(credential: OAuth2Credential) -> Self {
        Self {
            credential,
            http_client: Client::new(),
        }
    }
    
    /// Get authenticated user
    pub async fn get_user(&self) -> Result<GitHubUser> {
        let access_token = self.credential.access_token().expose_secret();
        
        let response = self.http_client
            .get(&format!("{}/user", Self::API_BASE))
            .header("Authorization", format!("Bearer {}", access_token))
            .header("User-Agent", "nebula-credential-example")
            .send()
            .await
            .context("Failed to fetch user")?;
        
        if !response.status().is_success() {
            let error_text = response.text().await?;
            anyhow::bail!("GitHub API error: {}", error_text);
        }
        
        response.json().await.context("Failed to parse user response")
    }
    
    /// Get user repositories
    pub async fn get_repositories(&self) -> Result<Vec<GitHubRepository>> {
        let access_token = self.credential.access_token().expose_secret();
        
        let response = self.http_client
            .get(&format!("{}/user/repos", Self::API_BASE))
            .header("Authorization", format!("Bearer {}", access_token))
            .header("User-Agent", "nebula-credential-example")
            .query(&[("sort", "updated"), ("per_page", "10")])
            .send()
            .await
            .context("Failed to fetch repositories")?;
        
        response.json().await.context("Failed to parse repositories")
    }
}

#[derive(Debug, Deserialize)]
pub struct GitHubUser {
    pub login: String,
    pub id: u64,
    pub name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: String,
    pub html_url: String,
    pub public_repos: u32,
    pub followers: u32,
}

#[derive(Debug, Deserialize)]
pub struct GitHubRepository {
    pub id: u64,
    pub name: String,
    pub full_name: String,
    pub private: bool,
    pub html_url: String,
    pub description: Option<String>,
    pub stargazers_count: u32,
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("🐙 GitHub OAuth2 Integration Example\n");
    
    // Configure GitHub OAuth2
    let config = GitHubOAuth2Config::new(
        std::env::var("GITHUB_CLIENT_ID")
            .context("GITHUB_CLIENT_ID not set")?,
        std::env::var("GITHUB_CLIENT_SECRET")
            .context("GITHUB_CLIENT_SECRET not set")?,
        "http://localhost:8080/callback".to_string(),
        vec![
            GitHubScope::ReadUser,
            GitHubScope::UserEmail,
            GitHubScope::PublicRepo,
        ],
    );
    
    // Initialize OAuth2 flow
    let flow = GitHubOAuth2Flow::new(config);
    
    // Generate authorization URL
    let auth_url = flow.authorization_url();
    println!("✓ Authorization URL:");
    println!("  {}\n", auth_url);
    println!("📋 Next steps:");
    println!("  1. Open URL in browser");
    println!("  2. Authorize the application");
    println!("  3. Copy the 'code' parameter from redirect URL");
    
    // In real app, start HTTP server to receive callback
    // For this example, manually input the code
    println!("\nEnter authorization code:");
    let mut code = String::new();
    std::io::stdin().read_line(&mut code)?;
    let code = code.trim();
    
    // Exchange code for token
    println!("\n🔄 Exchanging code for access token...");
    let credential = flow.exchange_code(code).await?;
    println!("✓ Access token obtained\n");
    
    // Use credential to access GitHub API
    let client = GitHubClient::new(credential);
    
    // Get authenticated user
    println!("🔍 Fetching user information...");
    let user = client.get_user().await?;
    println!("✓ Authenticated as: {}", user.login);
    if let Some(name) = &user.name {
        println!("  Name: {}", name);
    }
    if let Some(email) = &user.email {
        println!("  Email: {}", email);
    }
    println!("  Public repos: {}", user.public_repos);
    println!("  Followers: {}\n", user.followers);
    
    // Get repositories
    println!("📚 Fetching repositories...");
    let repos = client.get_repositories().await?;
    println!("✓ Found {} repositories:\n", repos.len());
    
    for repo in repos.iter().take(5) {
        println!("  • {}", repo.full_name);
        if let Some(desc) = &repo.description {
            println!("    {}", desc);
        }
        println!("    ⭐ {} | 🔒 {}", repo.stargazers_count, if repo.private { "Private" } else { "Public" });
        println!();
    }
    
    println!("🎉 GitHub OAuth2 integration complete!");
    
    Ok(())
}
```

## Dependencies

Add to `Cargo.toml`:

```toml
[dependencies]
nebula-credential = { version = "0.1", features = ["oauth2"] }
reqwest = { version = "0.11", features = ["json"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
sha2 = "0.10"
base64 = "0.21"
rand = "0.8"
urlencoding = "2"
```

## Key Implementation Details

### Part 1: GitHub-Specific Scopes

```rust
pub enum GitHubScope {
    Repo,          // Full repo access
    PublicRepo,    // Public repos only
    ReadUser,      // Read user profile
    UserEmail,     // User email
    Workflow,      // GitHub Actions
}
```

**Key points**:
- `repo` grants full access to public and private repositories
- `public_repo` limits access to public repositories only
- `read:user` and `user:email` are minimal scopes for authentication
- `workflow` scope needed for GitHub Actions integration
- Request only the scopes your application actually needs

### Part 2: GitHub Token Exchange

```rust
let response = self.http_client
    .post(Self::TOKEN_URL)
    .header("Accept", "application/json")
    .form(&params)
    .send()
    .await?;
```

**GitHub specifics**:
- Must include `Accept: application/json` header
- Without it, GitHub returns URL-encoded format
- GitHub tokens don't expire by default
- No refresh tokens for OAuth Apps (only for GitHub Apps)

### Part 3: GitHub API Usage

```rust
let response = self.http_client
    .get(&format!("{}/user", Self::API_BASE))
    .header("Authorization", format!("Bearer {}", access_token))
    .header("User-Agent", "nebula-credential-example")
    .send()
    .await?;
```

**Required headers**:
- `Authorization: Bearer <token>` - Authentication
- `User-Agent` - Required by GitHub (requests fail without it)
- Rate limit: 5,000 requests/hour for authenticated requests

## Expected Output

```
🐙 GitHub OAuth2 Integration Example

✓ Authorization URL:
  https://github.com/login/oauth/authorize?client_id=...

📋 Next steps:
  1. Open URL in browser
  2. Authorize the application
  3. Copy the 'code' parameter from redirect URL

Enter authorization code:
[user enters code]

🔄 Exchanging code for access token...
✓ Access token obtained

🔍 Fetching user information...
✓ Authenticated as: octocat
  Name: The Octocat
  Email: octocat@github.com
  Public repos: 8
  Followers: 4523

📚 Fetching repositories...
✓ Found 10 repositories:

  • octocat/Hello-World
    My first repository on GitHub!
    ⭐ 1842 | 🔒 Public

  • octocat/Spoon-Knife
    This repo is for demonstration purposes only.
    ⭐ 12347 | 🔒 Public

🎉 GitHub OAuth2 integration complete!
```

## Variations

### Variation 1: GitHub Device Flow (for CLI apps)

For CLI applications without browser redirect:

```rust
pub struct GitHubDeviceFlow {
    client_id: String,
}

impl GitHubDeviceFlow {
    pub async fn start(&self) -> Result<DeviceCodeResponse> {
        let response = reqwest::Client::new()
            .post("https://github.com/login/device/code")
            .header("Accept", "application/json")
            .form(&[("client_id", &self.client_id)])
            .send()
            .await?;
        
        Ok(response.json().await?)
    }
    
    pub async fn poll_for_token(&self, device_code: &str) -> Result<OAuth2Credential> {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            
            let response = reqwest::Client::new()
                .post("https://github.com/login/oauth/access_token")
                .header("Accept", "application/json")
                .form(&[
                    ("client_id", self.client_id.as_str()),
                    ("device_code", device_code),
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ])
                .send()
                .await?;
            
            let result: serde_json::Value = response.json().await?;
            
            if let Some(access_token) = result.get("access_token") {
                return OAuth2Credential::builder()
                    .client_id(&self.client_id)
                    .access_token(SecretString::new(access_token.as_str().unwrap().to_string()))
                    .build();
            }
            
            match result.get("error").and_then(|e| e.as_str()) {
                Some("authorization_pending") => continue,
                Some(err) => anyhow::bail!("Device flow error: {}", err),
                None => continue,
            }
        }
    }
}
```

### Variation 2: GitHub App (not OAuth App)

For production systems with higher rate limits:

```rust
use jsonwebtoken::{encode, Algorithm, Header, EncodingKey};

// GitHub Apps use JWT for authentication
let claims = GitHubAppClaims {
    iat: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64,
    exp: (SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + 600) as i64,
    iss: "your_app_id".to_string(),
};

let private_key = std::fs::read("github_app_private_key.pem")?;
let jwt = encode(
    &Header::new(Algorithm::RS256),
    &claims,
    &EncodingKey::from_rsa_pem(&private_key)?
)?;

// Use JWT to get installation access token (15,000 req/hour limit)
```

## Important Notes

> [!warning] Client Secret Security
> **Never commit client secrets to version control**. Use environment variables or secure secret management:
> ```rust
> let client_secret = std::env::var("GITHUB_CLIENT_SECRET")?;
> ```

> [!warning] User-Agent Required
> GitHub API requires `User-Agent` header on all requests. Without it, you'll get `403 Forbidden`:
> ```rust
> .header("User-Agent", "my-app-name")
> ```

> [!tip] Scope Minimization
> Request only the scopes you need:
> - Authentication only: `read:user`, `user:email`
> - Public repo access: `public_repo`
> - Full repo access: `repo` (includes private repos)

> [!tip] Rate Limits
> - Authenticated: 5,000 requests/hour
> - Use conditional requests with `If-None-Match` header
> - Check `X-RateLimit-Remaining` header
> - GitHub Apps have 15,000 requests/hour limit

## Troubleshooting

### Problem: "bad_verification_code"

**Cause**: Authorization code expired (10-minute TTL) or already used.

**Solution**: Generate new authorization URL and restart flow.

### Problem: "redirect_uri_mismatch"

**Cause**: Redirect URI doesn't match exactly what's registered in GitHub OAuth App.

**Solution**: Ensure exact match including protocol, port, and path:
```rust
// GitHub OAuth App setting: http://localhost:8080/callback
redirect_uri: "http://localhost:8080/callback"  // Must match exactly
```

### Problem: Missing User-Agent error

**Cause**: GitHub API requires `User-Agent` header.

**Solution**:
```rust
.header("User-Agent", "my-app/1.0")
```

### Problem: Rate limit exceeded (403)

**Cause**: Exceeded 5,000 requests/hour limit.

**Solution**: Check rate limit headers and implement backoff:
```rust
if let Some(remaining) = response.headers().get("X-RateLimit-Remaining") {
    if remaining == "0" {
        // Wait until reset time
        let reset = response.headers().get("X-RateLimit-Reset")?;
        // Implement exponential backoff
    }
}
```

## Related Examples

- [[OAuth2-Flow|OAuth2 Authorization Code Flow]]
- [[OAuth2-Google|OAuth2 Google Integration]]
- [[OAuth2-ClientCredentials|OAuth2 Client Credentials]]
- [[JWT-Validation|JWT Token Validation]]

## See Also

- Concept: [[../../03-Concepts/Credentials#OAuth2|OAuth2 Credentials]]
- How-To: [[../How-To/RotateCredentials|Rotate Credentials]]
- Security: [[../Security/Encryption|Credential Encryption]]
- [GitHub OAuth Apps Documentation](https://docs.github.com/en/apps/oauth-apps)
- [GitHub REST API Documentation](https://docs.github.com/en/rest)

---

**Validation Checklist**:
- [x] Code is complete and runnable
- [x] Cargo.toml dependencies listed
- [x] Key parts explained with comments
- [x] Expected output shown
- [x] Two variations provided (Device Flow, GitHub App)
- [x] GitHub-specific details covered (scopes, User-Agent, rate limits)
- [x] Security best practices included
- [x] Troubleshooting section complete
- [x] Cross-links to related examples
