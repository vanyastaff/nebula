---
title: "OAuth2 Client Credentials Flow"
tags: [example, oauth2, service-to-service, intermediate]
status: published
lang: ru
created: 2026-02-03
last_updated: 2026-02-03
audience: [intermediate]
estimated_reading: 10
priority: P1
---

# OAuth2 Client Credentials Flow

> **TL;DR**: Используйте `OAuth2ClientCredentials` из nebula-credential для machine-to-machine authentication без участия пользователя.

## Обзор

Client Credentials Flow — это OAuth 2.0 grant для service-to-service authentication. В nebula-credential это реализовано через `Credential` trait с автоматическим refresh и caching.

**Когда использовать**:
- Backend-to-backend коммуникация
- Scheduled jobs, cron tasks
- Микросервисная архитектура
- CI/CD pipelines

**Architecture Reference**: См. [[../../specs/001-credential-docs/architecture.md#oauth2-protocol]] для trait hierarchy  
**Implementation**: См. [[../../specs/001-credential-docs/technical-design.md#oauth2-implementation]]

## Prerequisites

- [[Installation|Установлен nebula-credential]]
- [[Core-Concepts|Понимание Credential trait]]
- Зарегистрированное приложение у OAuth2 провайдера
- Client ID и Client Secret

## Complete Example

### Dependencies

```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
```

### Использование nebula-credential API

```rust
// File: examples/oauth2_client_credentials.rs
use nebula_credential::{
    Credential, CredentialContext, OwnerId, SecretString,
    oauth2::{OAuth2Config, OAuth2GrantType, OAuth2ClientCredentials, OAuth2Credential},
};
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("🔐 OAuth2 Client Credentials Example\n");

    // 1. Создание OAuth2 конфигурации
    let config = OAuth2Config {
        client_id: std::env::var("CLIENT_ID")?,
        client_secret: SecretString::new(std::env::var("CLIENT_SECRET")?),
        token_endpoint: "https://auth.example.com/oauth/token".to_string(),
        grant_type: OAuth2GrantType::ClientCredentials,
        scopes: vec!["read:users".to_string(), "write:orders".to_string()],
        ..Default::default()
    };

    println!("✅ OAuth2 Configuration:");
    println!("   Client ID: {}", config.client_id);
    println!("   Token Endpoint: {}", config.token_endpoint);
    println!("   Scopes: {:?}", config.scopes);

    // 2. Создание credential instance
    let credential = OAuth2ClientCredentials::new(config);

    // 3. Создание credential context
    let ctx = CredentialContext::new(OwnerId::new("service-app"));

    // 4. Получение access token через Credential trait
    println!("\n📡 Requesting access token...");
    
    match credential.retrieve(&ctx).await {
        Ok(oauth2_cred) => {
            println!("✅ Access token obtained");
            println!("   Token type: {}", oauth2_cred.token_type);
            println!("   Expires at: {}", oauth2_cred.expires_at);
            println!("   Scopes: {:?}", oauth2_cred.scopes);
            
            // 5. Использование токена в API запросе
            let access_token = oauth2_cred.access_token.expose();
            println!("\n📦 Using access token for API request:");
            println!("   Authorization: Bearer {}...{}", 
                &access_token[..8], 
                &access_token[access_token.len()-4..]
            );
            
            // Пример использования с reqwest
            let client = reqwest::Client::new();
            let response = client
                .get("https://api.example.com/users")
                .bearer_auth(access_token)
                .send()
                .await?;
            
            println!("   Response status: {}", response.status());
        }
        Err(e) => {
            println!("❌ Failed to obtain token: {}", e);
        }
    }

    // 6. Проверка возможности refresh
    println!("\n🔄 Credential capabilities:");
    println!("   Supports refresh: {}", credential.supports_refresh());
    println!("   Credential type: {}", credential.credential_type());

    Ok(())
}
```

### Реализация Custom OAuth2 Provider

```rust
// File: examples/custom_oauth2_provider.rs
use nebula_credential::{
    Credential, CredentialContext, CredentialError,
    oauth2::{OAuth2Config, OAuth2Credential, OAuth2TokenResponse},
};
use async_trait::async_trait;

/// Custom OAuth2 provider implementation
pub struct CustomOAuth2Provider {
    config: OAuth2Config,
    http_client: reqwest::Client,
}

impl CustomOAuth2Provider {
    pub fn new(config: OAuth2Config) -> Self {
        Self {
            config,
            http_client: reqwest::Client::new(),
        }
    }

    async fn request_token(&self) -> Result<OAuth2TokenResponse, CredentialError> {
        let params = [
            ("grant_type", "client_credentials"),
            ("client_id", &self.config.client_id),
            ("client_secret", self.config.client_secret.expose()),
            ("scope", &self.config.scopes.join(" ")),
        ];

        let response = self.http_client
            .post(&self.config.token_endpoint)
            .form(&params)
            .send()
            .await
            .map_err(|e| CredentialError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            return Err(CredentialError::AuthenticationFailed(
                format!("Token request failed: {}", response.status())
            ));
        }

        response.json::<OAuth2TokenResponse>()
            .await
            .map_err(|e| CredentialError::InvalidResponse(e.to_string()))
    }
}

#[async_trait]
impl Credential for CustomOAuth2Provider {
    type Output = OAuth2Credential;
    type Error = CredentialError;

    async fn retrieve(
        &self,
        _ctx: &CredentialContext,
    ) -> Result<Self::Output, Self::Error> {
        let token_response = self.request_token().await?;
        
        Ok(OAuth2Credential::from_token_response(
            token_response,
            self.config.clone(),
        ))
    }

    fn credential_type(&self) -> &'static str {
        "oauth2_client_credentials"
    }

    fn supports_refresh(&self) -> bool {
        false // Client Credentials не имеет refresh token
    }
}
```

### Testable Implementation

```rust
// File: examples/oauth2_testable.rs
use nebula_credential::{
    Credential, TestableCredential, CredentialContext,
    TestResult, TestDetails, CredentialError,
    oauth2::OAuth2ClientCredentials,
};
use async_trait::async_trait;
use std::collections::HashMap;

#[async_trait]
impl TestableCredential for OAuth2ClientCredentials {
    async fn test(
        &self,
        ctx: &CredentialContext,
    ) -> Result<TestResult, CredentialError> {
        let start = std::time::Instant::now();

        // Attempt to retrieve token
        match self.retrieve(ctx).await {
            Ok(cred) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                
                let details = TestDetails {
                    latency_ms,
                    endpoint_tested: self.config.token_endpoint.clone(),
                    permissions_verified: cred.scopes.clone(),
                    metadata: HashMap::from([
                        ("token_type".to_string(), 
                         serde_json::json!(cred.token_type)),
                        ("expires_in".to_string(), 
                         serde_json::json!((cred.expires_at - chrono::Utc::now()).num_seconds())),
                    ]),
                };

                Ok(TestResult::success("OAuth2 token obtained successfully")
                    .with_details(details))
            }
            Err(e) => {
                Ok(TestResult::failure(format!("Token retrieval failed: {}", e)))
            }
        }
    }

    fn test_description(&self) -> &str {
        "Testing OAuth2 Client Credentials flow by requesting access token"
    }
}

// Usage example
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let credential = OAuth2ClientCredentials::new(/* config */);
    let ctx = CredentialContext::new(OwnerId::new("test-app"));

    let test_result = credential.test(&ctx).await?;

    if test_result.success {
        println!("✅ Test passed: {}", test_result.message);
        if let Some(details) = test_result.details {
            println!("   Latency: {}ms", details.latency_ms);
            println!("   Endpoint: {}", details.endpoint_tested);
        }
    } else {
        println!("❌ Test failed: {}", test_result.message);
    }

    Ok(())
}
```

## Key Concepts

### 1. Credential Trait Implementation

nebula-credential использует trait-based design:

```rust
pub trait Credential {
    type Output;  // OAuth2Credential
    type Error;   // CredentialError
    
    async fn retrieve(&self, ctx: &CredentialContext) 
        -> Result<Self::Output, Self::Error>;
    
    fn credential_type(&self) -> &'static str;
    fn supports_refresh(&self) -> bool;
}
```

**Architecture**: См. [[Architecture#credential-trait-hierarchy]]

### 2. SecretString for Sensitive Data

```rust
use nebula_credential::SecretString;

// ✅ GOOD: Auto-zeroization при drop
let client_secret = SecretString::new("secret_value");

// Expose только когда необходимо
let secret_value = client_secret.expose();

// ❌ BAD: Plain String в памяти
let secret = "secret_value".to_string();
```

**Security**: См. [[../../specs/001-credential-docs/security-spec.md#secret-handling]]

### 3. CredentialContext

```rust
let ctx = CredentialContext::new(OwnerId::new("my-service"))
    .with_metadata("environment".to_string(), "production".to_string())
    .with_trace_id(Some("trace-123".to_string()));

// Context передается во все credential операции
let result = credential.retrieve(&ctx).await?;
```

### 4. Provider-Specific Configuration

**Auth0**:
```rust
OAuth2Config {
    token_endpoint: "https://YOUR_DOMAIN.auth0.com/oauth/token".to_string(),
    // Requires audience for API access
    additional_params: HashMap::from([
        ("audience".to_string(), "https://api.example.com".to_string())
    ]),
    ..config
}
```

**Okta**:
```rust
OAuth2Config {
    token_endpoint: "https://YOUR_DOMAIN.okta.com/oauth2/default/v1/token".to_string(),
    scopes: vec!["custom_scope".to_string()],
    ..config
}
```

**Azure AD**:
```rust
OAuth2Config {
    token_endpoint: format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
        tenant_id
    ),
    scopes: vec!["https://graph.microsoft.com/.default".to_string()],
    ..config
}
```

## Security Best Practices

> [!warning] Client Secret Security
> Используйте `SecretString` для всех sensitive values. Никогда не логируйте client_secret.

**Storage**:
```rust
// ✅ GOOD: From secrets manager
let secret = aws_secrets_manager.get_secret("oauth2/client_secret").await?;
let config = OAuth2Config {
    client_secret: SecretString::new(secret),
    // ...
};

// ❌ BAD: Hardcoded
let config = OAuth2Config {
    client_secret: SecretString::new("hardcoded_secret"), // NEVER!
    // ...
};
```

**Secure Logging**:
```rust
// SecretString auto-redacts in Debug/Display
println!("Config: {:?}", config); 
// Output: OAuth2Config { client_secret: SecretString(***), ... }
```

## Common Issues

### Issue 1: `invalid_client`

**Symptoms**: `CredentialError::AuthenticationFailed("invalid_client")`

**Solution**: Проверьте `client_id` и `client_secret`.

### Issue 2: `invalid_scope`

**Symptoms**: `CredentialError::InvalidScope`

**Solution**: Убедитесь что scopes разрешены для client.

## See Also

- [[OAuth2-Flow|OAuth2 Authorization Code Flow]] - для user authentication
- [[JWT-Validation|JWT Token Validation]] - валидация полученных токенов
- [[Core-Concepts|Core Concepts]] - понимание Credential trait
- [[API-Reference|API Reference]] - полная документация API
- [[Architecture|Architecture]] - trait hierarchy

**Spec References**:
- [[../../specs/001-credential-docs/architecture.md#oauth2-protocol]]
- [[../../specs/001-credential-docs/technical-design.md#oauth2-implementation]]
- [[../../specs/001-credential-docs/security-spec.md#oauth2-security]]

## Sources

- [RFC 6749 Section 4.4 - Client Credentials Grant](https://datatracker.ietf.org/doc/html/rfc6749#section-4.4)
- [nebula-credential API Documentation](../Reference/API-Reference.md)
