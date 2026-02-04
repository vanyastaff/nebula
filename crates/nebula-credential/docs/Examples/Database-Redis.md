---
title: "Redis Database Credentials"
tags: [example, database, redis, cache, intermediate]
status: published
lang: ru
created: 2026-02-03
last_updated: 2026-02-03
audience: [intermediate]
estimated_reading: 10
priority: P1
---

# Redis Database Credentials

> **TL;DR**: Используйте `Credential` trait из nebula-credential для безопасного хранения Redis connection strings с поддержкой ACL и TLS.

## Обзор

Redis credentials в nebula-credential управляются через `Credential` trait pattern с поддержкой:
- **Secure Storage**: Connection URLs хранятся в `SecretString`
- **ACL Support**: Redis 6.0+ username/password authentication
- **Testing**: `TestableCredential` для проверки подключения
- **Cloud Support**: AWS ElastiCache, Azure Cache, Redis Cloud

**Architecture Reference**: См. [[../../specs/001-credential-docs/architecture.md#credential-trait-hierarchy]]  
**Security**: См. [[../../specs/001-credential-docs/security-spec.md#database-security]]

## Prerequisites

- [[Installation|Установлен nebula-credential]]
- [[Core-Concepts|Понимание Credential trait]]
- Redis server (v6.0+ для ACL support)

## Complete Example

### Dependencies

```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1", features = ["full"] }
redis = { version = "0.24", features = ["tokio-comp", "connection-manager"] }
```

### Implementing Redis Credential

```rust
// File: examples/redis_credential.rs
use nebula_credential::{
    Credential, CredentialContext, CredentialError,
    TestableCredential, SecretString,
    TestResult, TestDetails, OwnerId,
};
use async_trait::async_trait;
use redis::{Client, AsyncCommands, ConnectionManager};
use std::collections::HashMap;

/// Redis credential configuration
#[derive(Debug, Clone)]
pub struct RedisConfig {
    pub host: String,
    pub port: u16,
    pub database: u8,
    pub username: Option<String>,
    pub tls_enabled: bool,
}

/// Redis credential output
#[derive(Debug, Clone)]
pub struct RedisCredential {
    pub config: RedisConfig,
    pub password: Option<SecretString>,
    pub connection_url: SecretString,
}

impl RedisCredential {
    pub fn new(config: RedisConfig, password: Option<SecretString>) -> Self {
        let scheme = if config.tls_enabled { "rediss" } else { "redis" };

        let connection_url = match (&config.username, &password) {
            (Some(username), Some(pwd)) => {
                SecretString::new(format!(
                    "{}://{}:{}@{}:{}/{}",
                    scheme, username, pwd.expose(), config.host, config.port, config.database
                ))
            }
            (None, Some(pwd)) => {
                SecretString::new(format!(
                    "{}://:{}@{}:{}/{}",
                    scheme, pwd.expose(), config.host, config.port, config.database
                ))
            }
            _ => {
                SecretString::new(format!(
                    "{}://{}:{}/{}",
                    scheme, config.host, config.port, config.database
                ))
            }
        };

        Self {
            config,
            password,
            connection_url,
        }
    }

    /// Get redacted connection URL for logging
    pub fn connection_url_redacted(&self) -> String {
        let scheme = if self.config.tls_enabled { "rediss" } else { "redis" };

        if let Some(ref username) = self.config.username {
            format!(
                "{}://{}:***@{}:{}/{}",
                scheme, username, self.config.host, self.config.port, self.config.database
            )
        } else {
            format!(
                "{}://:***@{}:{}/{}",
                scheme, self.config.host, self.config.port, self.config.database
            )
        }
    }
}

/// Redis credential provider
pub struct RedisCredentialProvider {
    config: RedisConfig,
    password_source: PasswordSource,
}

#[derive(Debug, Clone)]
pub enum PasswordSource {
    Environment(String), // Env var name
    Static(Option<SecretString>),
    SecretsManager { path: String },
}

impl RedisCredentialProvider {
    pub fn new(config: RedisConfig, password_source: PasswordSource) -> Self {
        Self {
            config,
            password_source,
        }
    }

    async fn resolve_password(&self) -> Result<Option<SecretString>, CredentialError> {
        match &self.password_source {
            PasswordSource::Environment(var_name) => {
                match std::env::var(var_name) {
                    Ok(password) => Ok(Some(SecretString::new(password))),
                    Err(_) => Ok(None), // Redis without password
                }
            }
            PasswordSource::Static(password) => {
                Ok(password.clone())
            }
            PasswordSource::SecretsManager { path } => {
                // Integration with storage provider
                Err(CredentialError::ConfigurationError(
                    format!("Secrets manager integration not yet implemented for path: {}", path)
                ))
            }
        }
    }
}

#[async_trait]
impl Credential for RedisCredentialProvider {
    type Output = RedisCredential;
    type Error = CredentialError;

    async fn retrieve(
        &self,
        _ctx: &CredentialContext,
    ) -> Result<Self::Output, Self::Error> {
        let password = self.resolve_password().await?;
        Ok(RedisCredential::new(self.config.clone(), password))
    }

    fn credential_type(&self) -> &'static str {
        "redis"
    }

    fn supports_refresh(&self) -> bool {
        false // Connection URL doesn't expire
    }
}

#[async_trait]
impl TestableCredential for RedisCredentialProvider {
    async fn test(
        &self,
        ctx: &CredentialContext,
    ) -> Result<TestResult, CredentialError> {
        let start = std::time::Instant::now();
        let credential = self.retrieve(ctx).await?;

        // Create Redis client
        let client = Client::open(credential.connection_url.expose().to_string())
            .map_err(|e| CredentialError::ConfigurationError(
                format!("Failed to create Redis client: {}", e)
            ))?;

        // Attempt connection
        let manager = ConnectionManager::new(client).await
            .map_err(|e| CredentialError::ConnectionError(
                format!("Failed to connect: {}", e)
            ))?;

        let mut conn = manager;

        // Test with PING command
        match redis::cmd("PING").query_async::<_, String>(&mut conn).await {
            Ok(pong) => {
                let latency_ms = start.elapsed().as_millis() as u64;

                // Get server info
                let info = redis::cmd("INFO")
                    .arg("server")
                    .query_async::<_, String>(&mut conn)
                    .await
                    .ok();

                let version = info
                    .and_then(|i| i.lines()
                        .find(|l| l.starts_with("redis_version:"))
                        .map(|l| l.replace("redis_version:", "").trim().to_string()))
                    .unwrap_or_else(|| "unknown".to_string());

                let details = TestDetails {
                    latency_ms,
                    endpoint_tested: credential.connection_url_redacted(),
                    permissions_verified: vec!["PING".to_string()],
                    metadata: HashMap::from([
                        ("redis_version".to_string(), 
                         serde_json::json!(version)),
                        ("response".to_string(), 
                         serde_json::json!(pong)),
                    ]),
                };

                Ok(TestResult::success("Redis connection successful")
                    .with_details(details))
            }
            Err(e) => {
                Ok(TestResult::failure(format!("PING command failed: {}", e)))
            }
        }
    }

    fn test_description(&self) -> &str {
        "Testing Redis connection by executing PING command"
    }
}
```

### Usage Example

```rust
// File: examples/use_redis_credential.rs
use nebula_credential::{
    Credential, TestableCredential, CredentialContext, OwnerId,
};
use redis::{Client, AsyncCommands, ConnectionManager};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔴 Redis Credential Example\n");

    // 1. Configure Redis credential
    let config = RedisConfig {
        host: "localhost".to_string(),
        port: 6379,
        database: 0,
        username: None, // Redis < 6.0
        tls_enabled: false,
    };

    let provider = RedisCredentialProvider::new(
        config,
        PasswordSource::Environment("REDIS_PASSWORD".to_string()),
    );

    // 2. Create credential context
    let ctx = CredentialContext::new(OwnerId::new("my-workflow"))
        .with_metadata("environment".to_string(), "production".to_string());

    // 3. Retrieve credential
    println!("📡 Retrieving Redis credential...");
    let credential = provider.retrieve(&ctx).await?;
    
    println!("✅ Credential retrieved");
    println!("   Connection: {}", credential.connection_url_redacted());
    println!("   Credential type: {}", provider.credential_type());

    // 4. Test credential validity
    println!("\n🧪 Testing credential...");
    let test_result = provider.test(&ctx).await?;

    if test_result.success {
        println!("✅ Test passed: {}", test_result.message);
        if let Some(details) = test_result.details {
            println!("   Latency: {}ms", details.latency_ms);
            println!("   Version: {:?}", details.metadata.get("redis_version"));
        }
    } else {
        println!("❌ Test failed: {}", test_result.message);
    }

    // 5. Use with Redis client
    println!("\n📝 Using Redis client...");
    
    let client = Client::open(credential.connection_url.expose().to_string())?;
    let manager = ConnectionManager::new(client).await?;
    let mut conn = manager;

    // Set key-value
    conn.set::<_, _, ()>("mykey", "myvalue").await?;
    println!("   ✓ SET mykey = 'myvalue'");

    // Get value
    let value: String = conn.get("mykey").await?;
    println!("   ✓ GET mykey = '{}'", value);

    // Set with expiration
    conn.set_ex::<_, _, ()>("session:123", "user_data", 300).await?;
    println!("   ✓ SET session:123 with TTL 300s");

    Ok(())
}
```

## Key Concepts

### 1. Credential Trait для Redis

```rust
impl Credential for RedisCredentialProvider {
    type Output = RedisCredential; // Содержит connection URL
    type Error = CredentialError;
    
    async fn retrieve(&self, ctx: &CredentialContext) 
        -> Result<Self::Output, Self::Error> {
        // Resolve password from source
        // Build connection URL with SecretString
    }
}
```

**Architecture**: См. [[Architecture#credential-trait-hierarchy]]

### 2. SecretString для Connection URL

```rust
// ✅ GOOD: Connection URL auto-zeroized
let password = SecretString::new(env::var("REDIS_PASSWORD")?);
let connection_url = SecretString::new(format!(
    "redis://:{}@host/0", 
    password.expose()
));

// Redacted logging
println!("URL: {}", credential.connection_url_redacted()); // Password shows as ***
```

**Security**: См. [[../../specs/001-credential-docs/security-spec.md#secret-handling]]

### 3. TestableCredential Implementation

```rust
impl TestableCredential for RedisCredentialProvider {
    async fn test(&self, ctx: &CredentialContext) 
        -> Result<TestResult, CredentialError> {
        // Create client
        // Execute PING command
        // Return result with server version
    }
}
```

### 4. Redis ACL Support (v6.0+)

```rust
// Username + Password authentication
let config = RedisConfig {
    username: Some("app_user".to_string()),
    // ...
};

// Connection URL: redis://app_user:password@host:6379/0
```

## Security Best Practices

> [!warning] Connection URL Security
> Используйте `SecretString` для connection URLs. Никогда не логируйте passwords.

**TLS Configuration**:
```rust
let config = RedisConfig {
    tls_enabled: true, // ✅ Enable for production
    // ...
};

// Connection URL: rediss://:password@host:6380/0
```

**Storage Integration**:
```rust
// Integration с AWS Secrets Manager
let provider = RedisCredentialProvider::new(
    config,
    PasswordSource::SecretsManager { 
        path: "redis/production/password".to_string() 
    },
);
```

## Common Issues

### Issue 1: Authentication Failed

**Symptoms**: `NOAUTH Authentication required`

**Solution**: Убедиться что password предоставлен:
```rust
let provider = RedisCredentialProvider::new(
    config,
    PasswordSource::Environment("REDIS_PASSWORD".to_string()),
);
```

### Issue 2: Connection Refused

**Symptoms**: `Connection refused (os error 111)`

**Solution**: Проверить что Redis server запущен и доступен:
```bash
redis-cli -h localhost -p 6379 PING
```

## Related Examples

- **Other Databases**: [[Database-PostgreSQL]] - PostgreSQL credentials | [[Database-MySQL]] - MySQL credentials | [[Database-MongoDB]] - MongoDB credentials
- **Cloud Credentials**: [[AWS-Credentials]] - AWS access keys | [[AWS-AssumeRole]] - AWS temporary credentials
- **Basic Auth**: [[API-Key-Basic]] - Simple API key authentication

## See Also

- [[Core-Concepts|Core Concepts]] - понимание Credential trait
- [[API-Reference|API Reference]] - полная документация

**Spec References**:
- [[../../specs/001-credential-docs/architecture.md#credential-trait-hierarchy]]
- [[../../specs/001-credential-docs/security-spec.md#database-security]]
- [[../../specs/001-credential-docs/technical-design.md#database-credentials]]

## Sources

- [redis-rs Documentation](https://docs.rs/redis/)
- [nebula-credential API](../Reference/API-Reference.md)
