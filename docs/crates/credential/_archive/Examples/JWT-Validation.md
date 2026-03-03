---
title: "JWT Token Validation"
tags: [example, jwt, authentication, validation, intermediate]
status: published
lang: ru
created: 2026-02-03
last_updated: 2026-02-03
audience: [intermediate]
estimated_reading: 8
priority: P1
---

# JWT Token Validation

> **TL;DR**: Используйте `Credential` trait из nebula-credential для валидации JWT tokens с HS256/RS256 algorithms и проверкой claims.

## Обзор

JWT credentials в nebula-credential управляются через `Credential` trait pattern с поддержкой:
- **Signature Validation**: Проверка что token не подделан
- **Claims Validation**: Проверка issuer, audience, expiration
- **Algorithm Enforcement**: Защита от algorithm confusion attacks
- **Key Management**: Secure storage signing keys в `SecretString`

**Architecture Reference**: См. [[../../specs/001-credential-docs/architecture.md#jwt-protocol]]  
**Security**: См. [[../../specs/001-credential-docs/security-spec.md#jwt-security]]

## Prerequisites

- [[Installation|Установлен nebula-credential]]
- [[OAuth2-Flow|Понимание OAuth2 tokens]]
- [[Core-Concepts|Понимание Credential trait]]

## Complete Example

### Dependencies

```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1", features = ["full"] }
jsonwebtoken = "9"
serde = { version = "1.0", features = ["derive"] }
```

### Implementing JWT Credential

```rust
// File: examples/jwt_credential.rs
use nebula_credential::{
    Credential, CredentialContext, CredentialError,
    TestableCredential, SecretString,
    TestResult, TestDetails, OwnerId,
};
use async_trait::async_trait;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// JWT configuration
#[derive(Debug, Clone)]
pub struct JwtConfig {
    pub algorithm: Algorithm,
    pub issuer: String,
    pub audience: String,
}

/// JWT claims
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    pub sub: String,
    pub iss: String,
    pub aud: String,
    pub exp: u64,
    pub iat: u64,
    #[serde(flatten)]
    pub custom: HashMap<String, serde_json::Value>,
}

/// JWT credential output
#[derive(Debug, Clone)]
pub struct JwtCredential {
    pub config: JwtConfig,
    pub token: SecretString,
    pub claims: JwtClaims,
}

impl JwtCredential {
    pub fn new(config: JwtConfig, token: SecretString, claims: JwtClaims) -> Self {
        Self {
            config,
            token,
            claims,
        }
    }

    /// Get token preview for logging
    pub fn token_preview(&self) -> String {
        let full_token = self.token.expose();
        if full_token.len() > 20 {
            format!("{}...{}", &full_token[..10], &full_token[full_token.len() - 10..])
        } else {
            "***".to_string()
        }
    }
}

/// JWT credential provider
pub struct JwtCredentialProvider {
    config: JwtConfig,
    key_source: KeySource,
}

#[derive(Debug, Clone)]
pub enum KeySource {
    HmacSecret(SecretString), // For HS256
    RsaPublicKey(String),     // For RS256 (PEM format)
    EcdsaPublicKey(String),   // For ES256 (PEM format)
}

impl JwtCredentialProvider {
    pub fn new(config: JwtConfig, key_source: KeySource) -> Self {
        Self { config, key_source }
    }

    fn create_validation(&self) -> Validation {
        let mut validation = Validation::new(self.config.algorithm);
        validation.set_issuer(&[&self.config.issuer]);
        validation.set_audience(&[&self.config.audience]);
        validation.leeway = 60; // 60 seconds clock skew tolerance
        validation
    }

    fn get_decoding_key(&self) -> Result<DecodingKey, CredentialError> {
        match &self.key_source {
            KeySource::HmacSecret(secret) => {
                Ok(DecodingKey::from_secret(secret.expose().as_bytes()))
            }
            KeySource::RsaPublicKey(pem) => {
                DecodingKey::from_rsa_pem(pem.as_bytes())
                    .map_err(|e| CredentialError::ConfigurationError(
                        format!("Invalid RSA public key: {}", e)
                    ))
            }
            KeySource::EcdsaPublicKey(pem) => {
                DecodingKey::from_ec_pem(pem.as_bytes())
                    .map_err(|e| CredentialError::ConfigurationError(
                        format!("Invalid ECDSA public key: {}", e)
                    ))
            }
        }
    }

    async fn validate_token(&self, token: &str) -> Result<JwtClaims, CredentialError> {
        let validation = self.create_validation();
        let decoding_key = self.get_decoding_key()?;

        let token_data = decode::<JwtClaims>(token, &decoding_key, &validation)
            .map_err(|e| CredentialError::AuthenticationFailed(
                format!("JWT validation failed: {}", e)
            ))?;

        Ok(token_data.claims)
    }
}

#[async_trait]
impl Credential for JwtCredentialProvider {
    type Output = JwtCredential;
    type Error = CredentialError;

    async fn retrieve(
        &self,
        ctx: &CredentialContext,
    ) -> Result<Self::Output, Self::Error> {
        // In production, token would come from context metadata or header
        let token_str = ctx.metadata.get("jwt_token")
            .ok_or_else(|| CredentialError::ConfigurationError(
                "JWT token not found in context metadata".to_string()
            ))?;

        let claims = self.validate_token(token_str).await?;
        
        Ok(JwtCredential::new(
            self.config.clone(),
            SecretString::new(token_str.to_string()),
            claims,
        ))
    }

    fn credential_type(&self) -> &'static str {
        "jwt"
    }

    fn supports_refresh(&self) -> bool {
        false // JWT tokens are typically not refreshed, new ones are issued
    }
}

#[async_trait]
impl TestableCredential for JwtCredentialProvider {
    async fn test(
        &self,
        ctx: &CredentialContext,
    ) -> Result<TestResult, CredentialError> {
        let start = std::time::Instant::now();
        
        match self.retrieve(ctx).await {
            Ok(credential) => {
                let latency_ms = start.elapsed().as_millis() as u64;

                let details = TestDetails {
                    latency_ms,
                    endpoint_tested: self.config.issuer.clone(),
                    permissions_verified: vec!["jwt:validate".to_string()],
                    metadata: HashMap::from([
                        ("subject".to_string(), 
                         serde_json::json!(credential.claims.sub)),
                        ("issuer".to_string(), 
                         serde_json::json!(credential.claims.iss)),
                        ("audience".to_string(), 
                         serde_json::json!(credential.claims.aud)),
                        ("algorithm".to_string(), 
                         serde_json::json!(format!("{:?}", self.config.algorithm))),
                    ]),
                };

                Ok(TestResult::success("JWT token validated successfully")
                    .with_details(details))
            }
            Err(e) => {
                Ok(TestResult::failure(format!("JWT validation failed: {}", e)))
            }
        }
    }

    fn test_description(&self) -> &str {
        "Testing JWT token by validating signature and claims"
    }
}
```

### Usage Example

```rust
// File: examples/use_jwt_credential.rs
use nebula_credential::{
    Credential, TestableCredential, CredentialContext, OwnerId,
};
use jsonwebtoken::Algorithm;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔐 JWT Credential Example\n");

    // 1. Configure JWT validation
    let config = JwtConfig {
        algorithm: Algorithm::HS256,
        issuer: "https://auth.example.com".to_string(),
        audience: "https://api.example.com".to_string(),
    };

    let secret = SecretString::new("my-256-bit-secret-key".to_string());
    
    let provider = JwtCredentialProvider::new(
        config,
        KeySource::HmacSecret(secret),
    );

    // 2. Create credential context with JWT token
    let test_token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9..."; // Example token
    
    let ctx = CredentialContext::new(OwnerId::new("api-gateway"))
        .with_metadata("jwt_token".to_string(), test_token.to_string());

    // 3. Validate and retrieve credential
    println!("📡 Validating JWT token...");
    
    match provider.retrieve(&ctx).await {
        Ok(credential) => {
            println!("✅ Token validated successfully");
            println!("   Token: {}", credential.token_preview());
            println!("   Subject: {}", credential.claims.sub);
            println!("   Issuer: {}", credential.claims.iss);
            println!("   Credential type: {}", provider.credential_type());
        }
        Err(e) => {
            println!("❌ Validation failed: {}", e);
            return Ok(());
        }
    }

    // 4. Test credential
    println!("\n🧪 Testing credential...");
    let test_result = provider.test(&ctx).await?;

    if test_result.success {
        println!("✅ Test passed: {}", test_result.message);
        if let Some(details) = test_result.details {
            println!("   Latency: {}ms", details.latency_ms);
            println!("   Algorithm: {:?}", details.metadata.get("algorithm"));
        }
    } else {
        println!("❌ Test failed: {}", test_result.message);
    }

    Ok(())
}
```

## Key Concepts

### 1. Credential Trait для JWT

```rust
impl Credential for JwtCredentialProvider {
    type Output = JwtCredential; // Contains validated claims
    type Error = CredentialError;
    
    async fn retrieve(&self, ctx: &CredentialContext) 
        -> Result<Self::Output, Self::Error> {
        // Extract token from context
        // Validate signature and claims
        // Return credential with claims
    }
}
```

**Architecture**: См. [[Architecture#credential-trait-hierarchy]]

### 2. SecretString для Keys и Tokens

```rust
// ✅ GOOD: Signing key auto-zeroized
let secret = SecretString::new("my-secret-key".to_string());
let provider = JwtCredentialProvider::new(
    config,
    KeySource::HmacSecret(secret),
);

// Token also stored in SecretString
let credential = JwtCredential {
    token: SecretString::new(token_str),
    // ...
};
```

**Security**: См. [[../../specs/001-credential-docs/security-spec.md#jwt-security]]

### 3. Algorithm Support

```rust
// HMAC (symmetric)
KeySource::HmacSecret(SecretString::new("secret"))

// RSA (asymmetric)
KeySource::RsaPublicKey(public_key_pem)

// ECDSA (asymmetric)
KeySource::EcdsaPublicKey(public_key_pem)
```

## Security Best Practices

> [!warning] Algorithm Enforcement
> Всегда enforce algorithm. Не позволяйте token определять algorithm.

```rust
// ✅ GOOD: Explicit algorithm
let config = JwtConfig {
    algorithm: Algorithm::RS256, // Enforced
    // ...
};

// ❌ BAD: Accepting multiple algorithms increases attack surface
```

**Claims Validation**:
```rust
let mut validation = Validation::new(algorithm);
validation.set_issuer(&["https://auth.example.com"]); // ✅ Verify issuer
validation.set_audience(&["https://api.example.com"]); // ✅ Verify audience
validation.validate_exp = true; // ✅ Check expiration
```

## Common Issues

### Issue 1: InvalidSignature

**Symptoms**: `CredentialError::AuthenticationFailed`

**Solution**: Verify secret/key matches token issuer.

### Issue 2: ExpiredSignature

**Symptoms**: Token validation fails with expiration error

**Solution**: Check system clock or increase leeway:
```rust
validation.leeway = 60; // 60 seconds tolerance
```

## Related Examples

- **OAuth2 & Tokens**: [[OAuth2-Flow]] - OAuth2 with PKCE | [[OAuth2-ClientCredentials]] - Service-to-service auth
- **Enterprise Auth**: [[SAML-Authentication]] - Enterprise SSO | [[Kerberos-Authentication]] - Kerberos tickets
- **Certificate Auth**: [[mTLS-Certificate]] - Mutual TLS authentication

## See Also

- [[Core-Concepts|Core Concepts]] - понимание Credential trait
- [[API-Reference|API Reference]] - полная документация

**Spec References**:
- [[../../specs/001-credential-docs/architecture.md#jwt-protocol]]
- [[../../specs/001-credential-docs/security-spec.md#jwt-security]]
- [[../../specs/001-credential-docs/technical-design.md#jwt-validation]]

## Sources

- [RFC 7519 - JWT](https://datatracker.ietf.org/doc/html/rfc7519)
- [nebula-credential API](../Reference/API-Reference.md)
