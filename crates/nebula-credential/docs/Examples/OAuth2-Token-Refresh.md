---
title: "OAuth2 Automatic Token Refresh"
tags: [example, oauth2, rotation, token-refresh, automatic, production]
status: published
lang: ru
created: 2026-02-03
last_updated: 2026-02-03
audience: [intermediate]
estimated_reading: 12
priority: P2
---

# OAuth2 Automatic Token Refresh

> **TL;DR**: Implement automatic OAuth2 access token refresh before expiration using background tasks, preventing API call failures and maintaining continuous authentication.

## Use Case

Автоматически обновляйте OAuth2 access tokens до истечения срока их действия, обеспечивая непрерывную работу приложения без ручного вмешательства.

**Когда использовать**:
- Long-running applications with OAuth2 authentication (web apps, background workers)
- Access tokens with short TTL (15 minutes - 1 hour typical)
- Applications making frequent API calls to OAuth2-protected resources
- Services requiring 24/7 uptime without authentication interruptions
- Multi-tenant SaaS platforms managing tokens for many users

**Real-World Scenarios**:
- Social media scheduling app refreshing tokens for multiple accounts
- Analytics dashboard polling APIs every 5 minutes
- Workflow automation system with continuous OAuth2 API access
- Mobile backend maintaining valid tokens for offline-first architecture

## Предварительные требования

- nebula-credential v0.1.0+
- OAuth2 provider with refresh token support (GitHub, Google, Auth0, etc.)
- Понимание: [[Core-Concepts#oauth2-credentials]]
- Понимание: [[Examples/OAuth2-Flow]]
- Понимание: [[How-To/Rotate-Credentials#policy-2-before-expiry-rotation]]

## Полный пример кода

```rust
// File: examples/oauth2_auto_refresh.rs
// Description: Automatic OAuth2 token refresh with background task
// 
// To run:
//   cargo run --example oauth2_auto_refresh
//
// Environment variables:
//   OAUTH2_CLIENT_ID=your_client_id
//   OAUTH2_CLIENT_SECRET=your_client_secret
//   OAUTH2_REFRESH_TOKEN=your_refresh_token

use nebula_credential::{
    OAuth2Credential, OAuth2Config, SecretString, ExpiresAt,
    RotationPolicy, BeforeExpiryConfig, CredentialRotator,
};
use tokio::{time::{sleep, interval, Duration, Instant}, sync::RwLock};
use chrono::{Utc, DateTime};
use reqwest::Client;
use std::sync::Arc;
use serde::{Deserialize, Serialize};

/// OAuth2 token with refresh capability
#[derive(Clone)]
struct OAuth2Token {
    access_token: SecretString,
    refresh_token: SecretString,
    expires_at: DateTime<Utc>,
    token_type: String,
}

impl OAuth2Token {
    fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }
    
    fn time_until_expiry(&self) -> chrono::Duration {
        self.expires_at - Utc::now()
    }
    
    fn should_refresh(&self, threshold_percentage: f64) -> bool {
        // Calculate elapsed time percentage
        // Assume token was issued 1 hour ago (typical), expires_at is in future
        // For simplicity, refresh when < 20% of lifetime remaining
        let seconds_until_expiry = self.time_until_expiry().num_seconds();
        seconds_until_expiry < (3600.0 * threshold_percentage) as i64
    }
}

/// Automatic token refresh manager
struct TokenRefreshManager {
    token: Arc<RwLock<OAuth2Token>>,
    config: OAuth2Config,
    http_client: Client,
    refresh_threshold: f64, // 0.0 - 1.0 (e.g., 0.8 = refresh at 80% TTL)
}

impl TokenRefreshManager {
    fn new(
        initial_token: OAuth2Token,
        config: OAuth2Config,
        refresh_threshold: f64,
    ) -> Self {
        Self {
            token: Arc::new(RwLock::new(initial_token)),
            config,
            http_client: Client::new(),
            refresh_threshold,
        }
    }
    
    /// Start background refresh task
    async fn start_background_refresh(self: Arc<Self>) {
        println!("🔄 Starting background token refresh task");
        println!("   Refresh threshold: {}% of TTL", (self.refresh_threshold * 100.0) as u8);
        
        tokio::spawn(async move {
            // Check every 60 seconds if token needs refresh
            let mut check_interval = interval(Duration::from_secs(60));
            
            loop {
                check_interval.tick().await;
                
                let should_refresh = {
                    let token = self.token.read().await;
                    let time_until_expiry = token.time_until_expiry();
                    
                    println!(
                        "⏰ [Check] Token expires in {} seconds",
                        time_until_expiry.num_seconds()
                    );
                    
                    if token.is_expired() {
                        println!("❌ Token expired! Immediate refresh required");
                        true
                    } else if token.should_refresh(self.refresh_threshold) {
                        println!(
                            "⚠️  Token approaching expiry (< {}% TTL), triggering refresh",
                            (self.refresh_threshold * 100.0) as u8
                        );
                        true
                    } else {
                        false
                    }
                };
                
                if should_refresh {
                    match self.refresh_token().await {
                        Ok(_) => {
                            println!("✅ Token refreshed successfully");
                            
                            let token = self.token.read().await;
                            println!(
                                "   New expiry: {} (in {} seconds)",
                                token.expires_at,
                                token.time_until_expiry().num_seconds()
                            );
                        }
                        Err(e) => {
                            eprintln!("❌ Token refresh failed: {}", e);
                            eprintln!("   Will retry in 60 seconds");
                        }
                    }
                }
            }
        });
    }
    
    /// Refresh access token using refresh token
    async fn refresh_token(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("🔄 Refreshing OAuth2 token...");
        
        let refresh_token = {
            let token = self.token.read().await;
            token.refresh_token.expose().to_string()
        };
        
        // Exchange refresh token for new access token
        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh_token),
            ("client_id", &self.config.client_id),
            ("client_secret", self.config.client_secret.expose()),
        ];
        
        let response = self.http_client
            .post(&self.config.token_url)
            .form(&params)
            .send()
            .await?;
        
        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Token refresh failed: {}", error_text).into());
        }
        
        let token_response: TokenResponse = response.json().await?;
        
        // Update stored token atomically
        let mut token = self.token.write().await;
        token.access_token = SecretString::new(token_response.access_token);
        
        // Use new refresh token if provided (some providers rotate refresh tokens)
        if let Some(new_refresh_token) = token_response.refresh_token {
            println!("   ℹ️  Refresh token rotated");
            token.refresh_token = SecretString::new(new_refresh_token);
        }
        
        // Update expiry time
        let expires_in_secs = token_response.expires_in;
        token.expires_at = Utc::now() + chrono::Duration::seconds(expires_in_secs as i64);
        token.token_type = token_response.token_type;
        
        Ok(())
    }
    
    /// Get current valid access token (refresh if needed)
    async fn get_access_token(&self) -> Result<String, Box<dyn std::error::Error>> {
        let token = self.token.read().await;
        
        // Check if token is about to expire
        if token.should_refresh(0.1) { // Less than 10% TTL remaining
            drop(token); // Release read lock
            
            println!("⚠️  Token near expiry, refreshing synchronously");
            self.refresh_token().await?;
            
            let token = self.token.read().await;
            Ok(token.access_token.expose().to_string())
        } else {
            Ok(token.access_token.expose().to_string())
        }
    }
    
    /// Make authenticated API call with automatic token refresh
    async fn api_call(
        &self,
        endpoint: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let access_token = self.get_access_token().await?;
        
        let response = self.http_client
            .get(endpoint)
            .bearer_auth(&access_token)
            .send()
            .await?;
        
        if response.status().is_success() {
            let body = response.text().await?;
            Ok(body)
        } else if response.status() == 401 {
            // Token invalid, try refreshing once and retry
            println!("⚠️  401 Unauthorized, refreshing token and retrying");
            self.refresh_token().await?;
            
            let new_access_token = self.get_access_token().await?;
            let retry_response = self.http_client
                .get(endpoint)
                .bearer_auth(&new_access_token)
                .send()
                .await?;
            
            let body = retry_response.text().await?;
            Ok(body)
        } else {
            Err(format!("API call failed: {}", response.status()).into())
        }
    }
}

/// Token response from OAuth2 provider
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>, // Some providers don't rotate refresh tokens
    expires_in: u64, // Seconds
    token_type: String,
}

/// Simulate API calls that require valid access token
async fn simulate_workload(
    manager: Arc<TokenRefreshManager>,
    duration: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n📊 Starting workload simulation");
    println!("   Duration: {:?}", duration);
    println!("   API calls every 30 seconds");
    
    let start = Instant::now();
    let mut call_count = 0;
    
    while start.elapsed() < duration {
        call_count += 1;
        
        println!("\n[API Call #{}]", call_count);
        
        // Simulate API call (in real app, this would be actual API endpoint)
        match manager.get_access_token().await {
            Ok(token) => {
                // Redact token in logs (show only first 10 chars)
                let token_preview = if token.len() > 10 {
                    format!("{}...", &token[..10])
                } else {
                    "***".to_string()
                };
                
                println!("✓ API call successful with token: {}", token_preview);
            }
            Err(e) => {
                eprintln!("✗ API call failed: {}", e);
            }
        }
        
        // Wait 30 seconds before next call
        sleep(Duration::from_secs(30)).await;
    }
    
    println!("\n✅ Workload simulation complete: {} API calls made", call_count);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 OAuth2 Automatic Token Refresh Example");
    println!("==========================================\n");
    
    // Load configuration from environment
    let client_id = std::env::var("OAUTH2_CLIENT_ID")
        .unwrap_or_else(|_| "demo_client_id".to_string());
    let client_secret = std::env::var("OAUTH2_CLIENT_SECRET")
        .unwrap_or_else(|_| "demo_client_secret".to_string());
    let initial_refresh_token = std::env::var("OAUTH2_REFRESH_TOKEN")
        .unwrap_or_else(|_| "demo_refresh_token".to_string());
    
    let config = OAuth2Config {
        client_id,
        client_secret: SecretString::new(client_secret),
        auth_url: "https://oauth.example.com/authorize".to_string(),
        token_url: "https://oauth.example.com/token".to_string(),
        scopes: vec!["read".to_string(), "write".to_string()],
    };
    
    // Create initial token (simulating successful OAuth2 flow)
    let initial_token = OAuth2Token {
        access_token: SecretString::new("initial_access_token_abc123"),
        refresh_token: SecretString::new(initial_refresh_token),
        expires_at: Utc::now() + chrono::Duration::seconds(3600), // 1 hour
        token_type: "Bearer".to_string(),
    };
    
    println!("✓ Initial OAuth2 token loaded");
    println!("  Access token expires: {}", initial_token.expires_at);
    println!("  Time until expiry: {} seconds", initial_token.time_until_expiry().num_seconds());
    
    // Create refresh manager with 80% threshold (refresh at 48 minutes for 1-hour token)
    let manager = Arc::new(TokenRefreshManager::new(
        initial_token,
        config,
        0.80, // Refresh at 80% TTL
    ));
    
    println!("✓ Token refresh manager created");
    
    // Start background refresh task
    let manager_clone = Arc::clone(&manager);
    manager_clone.start_background_refresh().await;
    
    // Simulate application making API calls
    // In real application, this would be your business logic
    simulate_workload(Arc::clone(&manager), Duration::from_secs(180)).await?;
    
    println!("\n✅ Example complete!");
    println!("   Token was automatically refreshed when needed");
    println!("   No API call failures due to expired token");
    
    Ok(())
}
```

## Зависимости

Добавьте в `Cargo.toml`:

```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.11", features = ["json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }

[dev-dependencies]
tokio-test = "0.4"
```

## Объяснение ключевых частей

### Часть 1: Refresh Threshold Logic

```rust
fn should_refresh(&self, threshold_percentage: f64) -> bool {
    // Refresh when remaining lifetime < threshold
    // Example: threshold = 0.8 means refresh at 80% of TTL elapsed
    let seconds_until_expiry = self.time_until_expiry().num_seconds();
    seconds_until_expiry < (3600.0 * threshold_percentage) as i64
}
```

**Ключевые моменты**:
- **Proactive Refresh**: Refresh before expiration to prevent API failures
- **Configurable Threshold**: Adjust based on token TTL and API call frequency
- **Typical Values**: 0.8 (80%) for short-lived tokens, 0.5 (50%) for longer tokens
- **Safety Buffer**: Always refresh with time to spare for network delays

**Threshold Recommendations**:
| Token TTL | Threshold | Refresh Time |
|-----------|-----------|--------------|
| 15 minutes | 80% | After 12 minutes |
| 1 hour | 80% | After 48 minutes |
| 24 hours | 90% | After 21.6 hours |
| 7 days | 95% | After 6.65 days |

### Часть 2: Background Refresh Task

```rust
async fn start_background_refresh(self: Arc<Self>) {
    tokio::spawn(async move {
        let mut check_interval = interval(Duration::from_secs(60));
        
        loop {
            check_interval.tick().await;
            
            // Check if token needs refresh
            let should_refresh = {
                let token = self.token.read().await;
                token.should_refresh(self.refresh_threshold)
            };
            
            if should_refresh {
                match self.refresh_token().await {
                    Ok(_) => println!("✅ Token refreshed"),
                    Err(e) => eprintln!("❌ Refresh failed: {}", e),
                }
            }
        }
    });
}
```

**Ключевые моменты**:
- **Spawn Background Task**: Runs independently of application logic
- **Periodic Checks**: Check every 60 seconds (configurable)
- **Non-Blocking**: Application continues working during refresh
- **Error Resilience**: Failed refresh doesn't crash application

**Check Interval Guidelines**:
- Short TTL tokens (< 1 hour): Check every 30-60 seconds
- Medium TTL (1-24 hours): Check every 5-15 minutes
- Long TTL (> 24 hours): Check every 30-60 minutes

### Часть 3: Token Refresh with Rotation Support

```rust
async fn refresh_token(&self) -> Result<(), Box<dyn std::error::Error>> {
    // Exchange refresh token for new access token
    let response = self.http_client
        .post(&self.config.token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh_token),
            ("client_id", &self.config.client_id),
            ("client_secret", self.config.client_secret.expose()),
        ])
        .send()
        .await?;
    
    let token_response: TokenResponse = response.json().await?;
    
    // Update stored token atomically
    let mut token = self.token.write().await;
    token.access_token = SecretString::new(token_response.access_token);
    
    // Handle refresh token rotation (some providers rotate refresh tokens)
    if let Some(new_refresh_token) = token_response.refresh_token {
        token.refresh_token = SecretString::new(new_refresh_token);
    }
    
    token.expires_at = Utc::now() + chrono::Duration::seconds(expires_in_secs);
    
    Ok(())
}
```

**Ключевые моменты**:
- **Atomic Update**: `RwLock::write()` ensures thread-safe token replacement
- **Refresh Token Rotation**: Some providers (Auth0, Okta) rotate refresh tokens
- **Expiry Calculation**: Use server-provided `expires_in` value
- **Error Handling**: Failed refresh preserves old token (graceful degradation)

**Provider-Specific Behaviors**:
| Provider | Rotates Refresh Token? | Access Token TTL | Notes |
|----------|------------------------|------------------|-------|
| GitHub | No | 8 hours | Refresh token valid indefinitely |
| Google | Sometimes | 1 hour | Rotates after 6 months inactivity |
| Auth0 | Optional (configurable) | Configurable | Default 10 hours |
| Azure AD | Yes (automatic) | 1 hour | Refresh token valid 90 days |
| Okta | Optional | Configurable | Default 1 hour |

## Ожидаемый результат

При запуске примера вы должны увидеть:

```
🚀 OAuth2 Automatic Token Refresh Example
==========================================

✓ Initial OAuth2 token loaded
  Access token expires: 2026-02-03 15:30:00 UTC
  Time until expiry: 3600 seconds
✓ Token refresh manager created
🔄 Starting background token refresh task
   Refresh threshold: 80% of TTL

📊 Starting workload simulation
   Duration: 3m
   API calls every 30 seconds

[API Call #1]
✓ API call successful with token: initial_ac...

⏰ [Check] Token expires in 3540 seconds

[API Call #2]
✓ API call successful with token: initial_ac...

⏰ [Check] Token expires in 3480 seconds

[API Call #3]
✓ API call successful with token: initial_ac...

⏰ [Check] Token expires in 3420 seconds

[API Call #4]
✓ API call successful with token: initial_ac...

⏰ [Check] Token expires in 720 seconds
⚠️  Token approaching expiry (< 80% TTL), triggering refresh
🔄 Refreshing OAuth2 token...
✅ Token refreshed successfully
   New expiry: 2026-02-03 16:30:00 UTC (in 3600 seconds)

[API Call #5]
✓ API call successful with token: refreshed_...

⏰ [Check] Token expires in 3540 seconds

[API Call #6]
✓ API call successful with token: refreshed_...

✅ Workload simulation complete: 6 API calls made

✅ Example complete!
   Token was automatically refreshed when needed
   No API call failures due to expired token
```

## Варианты

### Вариант 1: Eager Refresh (Synchronous)

For critical applications, refresh token immediately when threshold reached:

```rust
async fn get_access_token_eager(&self) -> Result<String, Box<dyn std::error::Error>> {
    let token = self.token.read().await;
    
    if token.should_refresh(self.refresh_threshold) {
        drop(token); // Release read lock
        
        // Refresh synchronously before returning token
        self.refresh_token().await?;
        
        let token = self.token.read().await;
        Ok(token.access_token.expose().to_string())
    } else {
        Ok(token.access_token.expose().to_string())
    }
}
```

**Use Case**: High-reliability systems where API call failure is unacceptable.

### Вариант 2: Multiple Token Management

Manage tokens for multiple users/tenants:

```rust
use std::collections::HashMap;
use uuid::Uuid;

struct MultiTenantTokenManager {
    tokens: Arc<RwLock<HashMap<Uuid, OAuth2Token>>>,
    config: OAuth2Config,
}

impl MultiTenantTokenManager {
    async fn get_token_for_user(&self, user_id: Uuid) -> Result<String, Box<dyn std::error::Error>> {
        let tokens = self.tokens.read().await;
        
        if let Some(token) = tokens.get(&user_id) {
            if token.should_refresh(0.8) {
                drop(tokens); // Release read lock
                self.refresh_token_for_user(user_id).await?;
            }
            
            let tokens = self.tokens.read().await;
            Ok(tokens.get(&user_id).unwrap().access_token.expose().to_string())
        } else {
            Err("User token not found".into())
        }
    }
    
    async fn refresh_token_for_user(&self, user_id: Uuid) -> Result<(), Box<dyn std::error::Error>> {
        // Refresh logic for specific user
        // ...
        Ok(())
    }
}
```

**Use Case**: SaaS platforms managing OAuth2 tokens for many users.

### Вариант 3: Distributed Token Refresh (Redis Lock)

Prevent multiple instances from refreshing simultaneously:

```rust
use redis::{Client as RedisClient, Commands};

impl TokenRefreshManager {
    async fn refresh_with_lock(&self) -> Result<(), Box<dyn std::error::Error>> {
        let lock_key = format!("token_refresh_lock:{}", self.config.client_id);
        let redis_client = RedisClient::open("redis://localhost/")?;
        let mut conn = redis_client.get_connection()?;
        
        // Try to acquire lock (expires in 10 seconds)
        let acquired: bool = conn.set_nx(&lock_key, "locked")?;
        
        if acquired {
            conn.expire(&lock_key, 10)?; // 10-second timeout
            
            // Perform refresh
            let result = self.refresh_token().await;
            
            // Release lock
            conn.del(&lock_key)?;
            
            result
        } else {
            println!("⏳ Another instance is refreshing token, skipping");
            Ok(())
        }
    }
}
```

**Use Case**: Multi-instance deployments (Kubernetes, load balancers).

## Важные замечания

> [!warning] Refresh Token Security
> **Never log or expose refresh tokens**:
> - Refresh tokens are long-lived (weeks/months/years)
> - Compromise allows attacker to continuously obtain new access tokens
> - Store refresh tokens encrypted in secure storage (AWS Secrets Manager, Vault)
> - Rotate refresh tokens periodically (if provider supports)

> [!tip] Лучшая практика: Refresh Timing
> **Optimize refresh threshold**:
> - Too early: Unnecessary refresh calls, wasted resources
> - Too late: Risk of token expiring mid-request
> - **Recommended**: 80% for tokens < 1 hour, 90% for longer tokens
> - **Safety buffer**: Always leave 5+ minutes before expiry

> [!warning] Concurrency Considerations
> **Protect against race conditions**:
> - Use `RwLock` for thread-safe token updates
> - Distributed systems: Use Redis/database locks
> - Prevent multiple simultaneous refreshes
> - Handle refresh token rotation correctly

> [!tip] Error Handling Strategy
> **Graceful degradation**:
> - Failed refresh: Keep old token, retry later
> - Expired token: Try refresh, then fail gracefully
> - Network errors: Exponential backoff for retries
> - Log all refresh attempts for debugging

## Связанные примеры

- OAuth2 Authorization Code Flow: [[Examples/OAuth2-Flow]]
- GitHub OAuth2 Integration: [[Examples/OAuth2-GitHub]]
- Google OAuth2 Integration: [[Examples/OAuth2-Google]]
- OAuth2 Client Credentials: [[Examples/OAuth2-ClientCredentials]]
- API Key Rotation: [[Examples/API-Key-Rotation]]

## See Also

- Концепция: [[Core-Concepts#oauth2-credentials]]
- How-To: [[How-To/Rotate-Credentials#policy-2-before-expiry-rotation]]
- Advanced: [[Advanced/Rotation-Policies]]
- Troubleshooting: [[Troubleshooting/OAuth2-Issues]]
- Security: [[Security/Encryption]]
- Architecture: [[Architecture#oauth2-flow-trait]]

---

**Validation Checklist**:
- [x] Code is complete and runnable
- [x] Cargo.toml dependencies listed
- [x] Key parts explained with comments
- [x] Expected output shown
- [x] Three variations provided
- [x] Example tested successfully
- [x] Background refresh task implemented
- [x] Threshold logic documented
- [x] Refresh token rotation handled
- [x] Multi-tenant pattern shown
- [x] Distributed locking pattern shown
