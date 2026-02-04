---
title: "PostgreSQL Database Credentials"
tags: [example, database, postgresql, intermediate]
status: published
lang: ru
created: 2026-02-03
last_updated: 2026-02-03
audience: [intermediate]
estimated_reading: 10
priority: P1
---

# PostgreSQL Database Credentials

> **TL;DR**: Используйте `DatabaseCredential` trait из nebula-credential для безопасного хранения PostgreSQL connection strings с автоматической ротацией.

## Обзор

PostgreSQL credentials в nebula-credential управляются через `Credential` trait с поддержкой:
- **Secure Storage**: Connection strings хранятся в `SecretString`
- **Auto-Rotation**: `RotatableCredential` для zero-downtime rotation
- **Testing**: `TestableCredential` для проверки подключения
- **Scoped Access**: Credential isolation по workflow/node scope

**Architecture**: См. [[../../specs/001-credential-docs/architecture.md#storage-abstraction]]

## Prerequisites

- [[Installation|Установлен nebula-credential]]
- [[Core-Concepts|Понимание Credential trait]]
- PostgreSQL server доступен

## Complete Example

### Dependencies

```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1", features = ["full"] }
tokio-postgres = "0.7"
deadpool-postgres = "0.12"
```

### Implementing Database Credential

```rust
// File: examples/postgres_credential.rs
use nebula_credential::{
    Credential, CredentialContext, CredentialError,
    RotatableCredential, TestableCredential,
    SecretString, TestResult, TestDetails,
};
use async_trait::async_trait;
use tokio_postgres::{Client, NoTls};
use std::collections::HashMap;

/// PostgreSQL credential configuration
#[derive(Debug, Clone)]
pub struct PostgresConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub ssl_mode: bool,
}

/// PostgreSQL credential output
#[derive(Debug, Clone)]
pub struct PostgresCredential {
    pub config: PostgresConfig,
    pub password: SecretString,
    pub connection_string: SecretString,
}

impl PostgresCredential {
    pub fn new(config: PostgresConfig, password: SecretString) -> Self {
        let connection_string = SecretString::new(format!(
            "postgres://{}:{}@{}:{}/{}",
            config.username,
            password.expose(),
            config.host,
            config.port,
            config.database
        ));

        Self {
            config,
            password,
            connection_string,
        }
    }

    /// Get redacted connection string for logging
    pub fn connection_string_redacted(&self) -> String {
        format!(
            "postgres://{}:***@{}:{}/{}",
            self.config.username,
            self.config.host,
            self.config.port,
            self.config.database
        )
    }
}

/// PostgreSQL credential provider
pub struct PostgresCredentialProvider {
    config: PostgresConfig,
    password_source: PasswordSource,
}

#[derive(Debug, Clone)]
pub enum PasswordSource {
    Environment(String), // Env var name
    Static(SecretString),
    SecretsManager { path: String },
}

impl PostgresCredentialProvider {
    pub fn new(config: PostgresConfig, password_source: PasswordSource) -> Self {
        Self {
            config,
            password_source,
        }
    }

    async fn resolve_password(&self) -> Result<SecretString, CredentialError> {
        match &self.password_source {
            PasswordSource::Environment(var_name) => {
                let password = std::env::var(var_name)
                    .map_err(|_| CredentialError::ConfigurationError(
                        format!("Environment variable {} not found", var_name)
                    ))?;
                Ok(SecretString::new(password))
            }
            PasswordSource::Static(password) => {
                Ok(password.clone())
            }
            PasswordSource::SecretsManager { path } => {
                // Integration with storage provider
                // For now, return error indicating not implemented
                Err(CredentialError::ConfigurationError(
                    format!("Secrets manager integration not yet implemented for path: {}", path)
                ))
            }
        }
    }
}

#[async_trait]
impl Credential for PostgresCredentialProvider {
    type Output = PostgresCredential;
    type Error = CredentialError;

    async fn retrieve(
        &self,
        _ctx: &CredentialContext,
    ) -> Result<Self::Output, Self::Error> {
        let password = self.resolve_password().await?;
        Ok(PostgresCredential::new(self.config.clone(), password))
    }

    fn credential_type(&self) -> &'static str {
        "postgresql"
    }

    fn supports_refresh(&self) -> bool {
        false // Connection string doesn't expire
    }
}

#[async_trait]
impl TestableCredential for PostgresCredentialProvider {
    async fn test(
        &self,
        ctx: &CredentialContext,
    ) -> Result<TestResult, CredentialError> {
        let start = std::time::Instant::now();
        let credential = self.retrieve(ctx).await?;

        // Attempt connection
        match tokio_postgres::connect(credential.connection_string.expose(), NoTls).await {
            Ok((client, connection)) => {
                // Spawn connection handler
                tokio::spawn(async move {
                    if let Err(e) = connection.await {
                        eprintln!("Connection error: {}", e);
                    }
                });

                // Test query
                match client.query_one("SELECT version()", &[]).await {
                    Ok(row) => {
                        let version: String = row.get(0);
                        let latency_ms = start.elapsed().as_millis() as u64;

                        let details = TestDetails {
                            latency_ms,
                            endpoint_tested: credential.connection_string_redacted(),
                            permissions_verified: vec!["SELECT".to_string()],
                            metadata: HashMap::from([
                                ("postgres_version".to_string(), 
                                 serde_json::json!(version)),
                            ]),
                        };

                        Ok(TestResult::success("PostgreSQL connection successful")
                            .with_details(details))
                    }
                    Err(e) => {
                        Ok(TestResult::failure(format!("Query failed: {}", e)))
                    }
                }
            }
            Err(e) => {
                Ok(TestResult::failure(format!("Connection failed: {}", e)))
            }
        }
    }

    fn test_description(&self) -> &str {
        "Testing PostgreSQL connection by executing SELECT version()"
    }
}

/// Rotation policy for database credentials
pub struct DatabaseRotationPolicy {
    pub rotate_every_days: u64,
}

impl nebula_credential::RotationPolicy for DatabaseRotationPolicy {
    fn should_rotate_by_age(&self, created_at: chrono::DateTime<chrono::Utc>) -> bool {
        let age = chrono::Utc::now() - created_at;
        age.num_days() as u64 >= self.rotate_every_days
    }

    fn should_rotate_by_usage(&self, _usage_count: u64) -> bool {
        false // Database credentials don't rotate by usage
    }
}

#[async_trait]
impl RotatableCredential for PostgresCredentialProvider {
    type Policy = DatabaseRotationPolicy;

    async fn rotate(
        &self,
        _current: &Self::Output,
        policy: &Self::Policy,
        ctx: &CredentialContext,
    ) -> Result<Self::Output, Self::Error> {
        println!("🔄 Rotating PostgreSQL password (policy: every {} days)", 
            policy.rotate_every_days);

        // In production: call database ALTER USER to change password
        // For demo: generate new password
        let new_password = SecretString::new(
            format!("rotated_password_{}", chrono::Utc::now().timestamp())
        );

        // Return new credential
        let new_credential = PostgresCredential::new(self.config.clone(), new_password);
        
        println!("✅ Password rotated successfully");
        Ok(new_credential)
    }

    async fn needs_rotation(
        &self,
        credential: &Self::Output,
        policy: &Self::Policy,
    ) -> Result<bool, Self::Error> {
        // Check if credential age exceeds policy
        // In production: track creation time in metadata
        Ok(false) // For demo
    }
}
```

### Usage Example

```rust
// File: examples/use_postgres_credential.rs
use nebula_credential::{
    Credential, TestableCredential, CredentialContext, OwnerId,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🐘 PostgreSQL Credential Example\n");

    // 1. Configure PostgreSQL credential
    let config = PostgresConfig {
        host: "localhost".to_string(),
        port: 5432,
        database: "myapp_db".to_string(),
        username: "app_user".to_string(),
        ssl_mode: false,
    };

    let provider = PostgresCredentialProvider::new(
        config,
        PasswordSource::Environment("PG_PASSWORD".to_string()),
    );

    // 2. Create credential context
    let ctx = CredentialContext::new(OwnerId::new("my-workflow"))
        .with_metadata("environment".to_string(), "production".to_string());

    // 3. Retrieve credential
    println!("📡 Retrieving PostgreSQL credential...");
    let credential = provider.retrieve(&ctx).await?;
    
    println!("✅ Credential retrieved");
    println!("   Connection: {}", credential.connection_string_redacted());
    println!("   Credential type: {}", provider.credential_type());

    // 4. Test credential validity
    println!("\n🧪 Testing credential...");
    let test_result = provider.test(&ctx).await?;

    if test_result.success {
        println!("✅ Test passed: {}", test_result.message);
        if let Some(details) = test_result.details {
            println!("   Latency: {}ms", details.latency_ms);
            println!("   Version: {:?}", details.metadata.get("postgres_version"));
        }
    } else {
        println!("❌ Test failed: {}", test_result.message);
    }

    // 5. Use with connection pool
    println!("\n🏊 Creating connection pool...");
    use deadpool_postgres::{Config as PoolConfig, Runtime};
    
    let mut pool_config = PoolConfig::new();
    pool_config.url = Some(credential.connection_string.expose().to_string());
    pool_config.pool = Some(deadpool::managed::PoolConfig {
        max_size: 20,
        ..Default::default()
    });

    let pool = pool_config.create_pool(Some(Runtime::Tokio1), NoTls)?;
    println!("✅ Pool created (max size: 20)");

    // 6. Execute query
    let client = pool.get().await?;
    let rows = client.query("SELECT 1 as test", &[]).await?;
    let test_value: i32 = rows[0].get(0);
    println!("   Query result: {}", test_value);

    Ok(())
}
```

## Key Concepts

### 1. Credential Trait для Database

```rust
impl Credential for PostgresCredentialProvider {
    type Output = PostgresCredential; // Содержит connection string
    type Error = CredentialError;
    
    async fn retrieve(&self, ctx: &CredentialContext) 
        -> Result<Self::Output, Self::Error> {
        // Resolve password from source
        // Build connection string
    }
}
```

### 2. SecretString для Passwords

```rust
// ✅ GOOD: Password auto-zeroized
let password = SecretString::new(env::var("PG_PASSWORD")?);
let connection_string = SecretString::new(format!("postgres://user:{}@host/db", 
    password.expose()));

// Redacted logging
println!("Config: {:?}", credential); // Password shows as ***
```

**Security**: См. [[../../specs/001-credential-docs/security-spec.md#secret-handling]]

### 3. TestableCredential для Validation

```rust
impl TestableCredential for PostgresCredentialProvider {
    async fn test(&self, ctx: &CredentialContext) 
        -> Result<TestResult, CredentialError> {
        // Attempt real connection
        // Execute test query
        // Return structured result
    }
}
```

### 4. RotatableCredential для Password Rotation

```rust
impl RotatableCredential for PostgresCredentialProvider {
    type Policy = DatabaseRotationPolicy;
    
    async fn rotate(&self, current: &Self::Output, policy: &Self::Policy, ...) 
        -> Result<Self::Output, Self::Error> {
        // Generate new password
        // ALTER USER ... PASSWORD in database
        // Return new credential
    }
}
```

## Security Best Practices

> [!warning] Connection String Security
> Используйте `SecretString` для connection strings. Никогда не логируйте passwords.

**Storage Integration**:
```rust
// Integration с AWS Secrets Manager
let provider = PostgresCredentialProvider::new(
    config,
    PasswordSource::SecretsManager { 
        path: "postgres/production/password".to_string() 
    },
);
```

**TLS Enforcement**:
```rust
use tokio_postgres_rustls::MakeRustlsConnect;

let tls = MakeRustlsConnect::new(rustls::ClientConfig::default());
let (client, connection) = tokio_postgres::connect(
    credential.connection_string.expose(), 
    tls // ✅ TLS required
).await?;
```

## Common Issues

### Issue 1: Connection Pool Exhausted

**Symptoms**: `Timeout waiting for connection`

**Solution**:
```rust
pool_config.pool = Some(PoolConfig {
    max_size: 50, // Increase pool size
    timeouts: Timeouts {
        wait: Some(Duration::from_secs(10)),
        ..Default::default()
    },
});
```

### Issue 2: Authentication Failed

**Symptoms**: `CredentialError::AuthenticationFailed`

**Solution**: Test credential first:
```rust
let test_result = provider.test(&ctx).await?;
if !test_result.success {
    eprintln!("Credential invalid: {}", test_result.message);
}
```

## Related Examples

- **Other Databases**: [[Database-MySQL]] - MySQL credentials | [[Database-MongoDB]] - MongoDB credentials | [[Database-Redis]] - Redis credentials
- **Cloud Credentials**: [[AWS-Credentials]] - AWS access keys | [[AWS-AssumeRole]] - AWS temporary credentials
- **Basic Auth**: [[API-Key-Basic]] - Simple API key authentication

## See Also

- [[Core-Concepts|Core Concepts]] - понимание Credential trait
- [[Rotate-Credentials|Rotation Guide]] - credential rotation patterns
- [[API-Reference|API Reference]] - полная документация

**Spec References**:
- [[../../specs/001-credential-docs/architecture.md#credential-trait-hierarchy]]
- [[../../specs/001-credential-docs/security-spec.md#database-security]]

## Sources

- [tokio-postgres Documentation](https://docs.rs/tokio-postgres/)
- [nebula-credential API](../Reference/API-Reference.md)
