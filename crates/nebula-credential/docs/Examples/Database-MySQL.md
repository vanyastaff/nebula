---
title: "MySQL Database Credentials"
tags: [example, database, mysql, intermediate]
status: published
lang: ru
created: 2026-02-03
last_updated: 2026-02-03
audience: [intermediate]
estimated_reading: 10
priority: P1
---

# MySQL Database Credentials

> **TL;DR**: Используйте `Credential` trait из nebula-credential для безопасного хранения MySQL connection strings с автоматической ротацией и connection pooling.

## Обзор

MySQL credentials в nebula-credential управляются через `Credential` trait pattern с поддержкой:
- **Secure Storage**: Connection strings хранятся в `SecretString`
- **Auto-Rotation**: `RotatableCredential` для zero-downtime password rotation
- **Testing**: `TestableCredential` для проверки подключения
- **Cloud Support**: AWS RDS, Azure Database for MySQL

**Architecture Reference**: См. [[../../specs/001-credential-docs/architecture.md#credential-trait-hierarchy]]  
**Security**: См. [[../../specs/001-credential-docs/security-spec.md#database-security]]

## Prerequisites

- [[Installation|Установлен nebula-credential]]
- [[Core-Concepts|Понимание Credential trait]]
- MySQL server (v5.7+ или v8.0+)

## Complete Example

### Dependencies

```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1", features = ["full"] }
mysql_async = "0.34"
deadpool = "0.12"
```

### Implementing MySQL Credential

```rust
// File: examples/mysql_credential.rs
use nebula_credential::{
    Credential, CredentialContext, CredentialError,
    RotatableCredential, TestableCredential,
    SecretString, TestResult, TestDetails, OwnerId,
};
use async_trait::async_trait;
use mysql_async::{Pool, OptsBuilder, SslOpts, prelude::*};
use std::collections::HashMap;

/// MySQL credential configuration
#[derive(Debug, Clone)]
pub struct MysqlConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub ssl_mode: bool,
}

/// MySQL credential output
#[derive(Debug, Clone)]
pub struct MysqlCredential {
    pub config: MysqlConfig,
    pub password: SecretString,
    pub connection_string: SecretString,
}

impl MysqlCredential {
    pub fn new(config: MysqlConfig, password: SecretString) -> Self {
        let connection_string = SecretString::new(format!(
            "mysql://{}:{}@{}:{}/{}",
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
            "mysql://{}:***@{}:{}/{}",
            self.config.username,
            self.config.host,
            self.config.port,
            self.config.database
        )
    }
}

/// MySQL credential provider
pub struct MysqlCredentialProvider {
    config: MysqlConfig,
    password_source: PasswordSource,
}

#[derive(Debug, Clone)]
pub enum PasswordSource {
    Environment(String), // Env var name
    Static(SecretString),
    SecretsManager { path: String },
}

impl MysqlCredentialProvider {
    pub fn new(config: MysqlConfig, password_source: PasswordSource) -> Self {
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
                Err(CredentialError::ConfigurationError(
                    format!("Secrets manager integration not yet implemented for path: {}", path)
                ))
            }
        }
    }
}

#[async_trait]
impl Credential for MysqlCredentialProvider {
    type Output = MysqlCredential;
    type Error = CredentialError;

    async fn retrieve(
        &self,
        _ctx: &CredentialContext,
    ) -> Result<Self::Output, Self::Error> {
        let password = self.resolve_password().await?;
        Ok(MysqlCredential::new(self.config.clone(), password))
    }

    fn credential_type(&self) -> &'static str {
        "mysql"
    }

    fn supports_refresh(&self) -> bool {
        false // Connection string doesn't expire
    }
}

#[async_trait]
impl TestableCredential for MysqlCredentialProvider {
    async fn test(
        &self,
        ctx: &CredentialContext,
    ) -> Result<TestResult, CredentialError> {
        let start = std::time::Instant::now();
        let credential = self.retrieve(ctx).await?;

        // Build MySQL options
        let mut opts = OptsBuilder::default()
            .ip_or_hostname(&credential.config.host)
            .tcp_port(credential.config.port)
            .db_name(Some(&credential.config.database))
            .user(Some(&credential.config.username))
            .pass(Some(credential.password.expose()));

        if credential.config.ssl_mode {
            let ssl_opts = SslOpts::default();
            opts = opts.ssl_opts(Some(ssl_opts));
        }

        // Attempt connection
        let pool = Pool::new(opts);
        
        match pool.get_conn().await {
            Ok(mut conn) => {
                // Test query
                match conn.query_first::<String, _>("SELECT VERSION()").await {
                    Ok(Some(version)) => {
                        let latency_ms = start.elapsed().as_millis() as u64;

                        let details = TestDetails {
                            latency_ms,
                            endpoint_tested: credential.connection_string_redacted(),
                            permissions_verified: vec!["SELECT".to_string()],
                            metadata: HashMap::from([
                                ("mysql_version".to_string(), 
                                 serde_json::json!(version)),
                            ]),
                        };

                        Ok(TestResult::success("MySQL connection successful")
                            .with_details(details))
                    }
                    Ok(None) => {
                        Ok(TestResult::failure("Query returned no version"))
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
        "Testing MySQL connection by executing SELECT VERSION()"
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
impl RotatableCredential for MysqlCredentialProvider {
    type Policy = DatabaseRotationPolicy;

    async fn rotate(
        &self,
        _current: &Self::Output,
        policy: &Self::Policy,
        _ctx: &CredentialContext,
    ) -> Result<Self::Output, Self::Error> {
        println!("🔄 Rotating MySQL password (policy: every {} days)", 
            policy.rotate_every_days);

        // In production: call MySQL ALTER USER to change password
        // For demo: generate new password
        let new_password = SecretString::new(
            format!("rotated_password_{}", chrono::Utc::now().timestamp())
        );

        // Return new credential
        let new_credential = MysqlCredential::new(self.config.clone(), new_password);
        
        println!("✅ Password rotated successfully");
        Ok(new_credential)
    }

    async fn needs_rotation(
        &self,
        _credential: &Self::Output,
        _policy: &Self::Policy,
    ) -> Result<bool, Self::Error> {
        // Check if credential age exceeds policy
        // In production: track creation time in metadata
        Ok(false) // For demo
    }
}
```

### Usage Example

```rust
// File: examples/use_mysql_credential.rs
use nebula_credential::{
    Credential, TestableCredential, CredentialContext, OwnerId,
};
use mysql_async::{Pool, OptsBuilder, SslOpts, prelude::*};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🐬 MySQL Credential Example\n");

    // 1. Configure MySQL credential
    let config = MysqlConfig {
        host: "localhost".to_string(),
        port: 3306,
        database: "myapp_db".to_string(),
        username: "app_user".to_string(),
        ssl_mode: false,
    };

    let provider = MysqlCredentialProvider::new(
        config,
        PasswordSource::Environment("MYSQL_PASSWORD".to_string()),
    );

    // 2. Create credential context
    let ctx = CredentialContext::new(OwnerId::new("my-workflow"))
        .with_metadata("environment".to_string(), "production".to_string());

    // 3. Retrieve credential
    println!("📡 Retrieving MySQL credential...");
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
            println!("   Version: {:?}", details.metadata.get("mysql_version"));
        }
    } else {
        println!("❌ Test failed: {}", test_result.message);
    }

    // 5. Use with connection pool
    println!("\n🏊 Creating connection pool...");
    
    let mut opts = OptsBuilder::default()
        .ip_or_hostname(&credential.config.host)
        .tcp_port(credential.config.port)
        .db_name(Some(&credential.config.database))
        .user(Some(&credential.config.username))
        .pass(Some(credential.password.expose()));

    if credential.config.ssl_mode {
        opts = opts.ssl_opts(Some(SslOpts::default()));
    }

    let pool = Pool::new(opts);
    println!("✅ Pool created");

    // 6. Execute query
    let mut conn = pool.get_conn().await?;
    let result: i32 = conn.query_first("SELECT 1 as test").await?.unwrap();
    println!("   Query result: {}", result);

    Ok(())
}
```

## Key Concepts

### 1. Credential Trait для MySQL

```rust
impl Credential for MysqlCredentialProvider {
    type Output = MysqlCredential; // Содержит connection string
    type Error = CredentialError;
    
    async fn retrieve(&self, ctx: &CredentialContext) 
        -> Result<Self::Output, Self::Error> {
        // Resolve password from source
        // Build connection string with SecretString
    }
}
```

**Architecture**: См. [[Architecture#credential-trait-hierarchy]]

### 2. SecretString для Passwords

```rust
// ✅ GOOD: Password auto-zeroized
let password = SecretString::new(env::var("MYSQL_PASSWORD")?);
let connection_string = SecretString::new(format!(
    "mysql://user:{}@host/db", 
    password.expose()
));

// Redacted logging
println!("Config: {:?}", credential); // Password shows as ***
```

**Security**: См. [[../../specs/001-credential-docs/security-spec.md#secret-handling]]

### 3. TestableCredential Implementation

```rust
impl TestableCredential for MysqlCredentialProvider {
    async fn test(&self, ctx: &CredentialContext) 
        -> Result<TestResult, CredentialError> {
        // Attempt real connection
        // Execute SELECT VERSION()
        // Return structured result with latency
    }
}
```

### 4. RotatableCredential для Password Rotation

```rust
impl RotatableCredential for MysqlCredentialProvider {
    type Policy = DatabaseRotationPolicy;
    
    async fn rotate(&self, current: &Self::Output, policy: &Self::Policy, ...) 
        -> Result<Self::Output, Self::Error> {
        // Generate new password
        // Execute: ALTER USER 'app_user'@'%' IDENTIFIED BY 'new_password'
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
let provider = MysqlCredentialProvider::new(
    config,
    PasswordSource::SecretsManager { 
        path: "mysql/production/password".to_string() 
    },
);
```

**SSL/TLS Configuration**:
```rust
use mysql_async::SslOpts;

let ssl_opts = SslOpts::default()
    .with_root_cert_path(Some("/path/to/ca.pem".into()))
    .with_danger_accept_invalid_certs(false); // ✅ Verify certificates

let opts = OptsBuilder::default()
    // ... other options ...
    .ssl_opts(Some(ssl_opts));
```

## Common Issues

### Issue 1: Too Many Connections

**Symptoms**: `ERROR 1040 (HY000): Too many connections`

**Solution**:
```sql
-- Увеличить max_connections
SET GLOBAL max_connections = 500;

-- Проверить текущие connections
SHOW PROCESSLIST;
```

### Issue 2: SSL Connection Failed

**Symptoms**: `ERROR 2026 (HY000): SSL connection error`

**Solution**: Проверить SSL configuration и CA certificate path:
```rust
let ssl_opts = SslOpts::default()
    .with_root_cert_path(Some("/correct/path/to/ca.pem".into()));
```

## Related Examples

- **Other Databases**: [[Database-PostgreSQL]] - PostgreSQL credentials | [[Database-MongoDB]] - MongoDB credentials | [[Database-Redis]] - Redis credentials
- **Cloud Credentials**: [[AWS-Credentials]] - AWS access keys | [[AWS-AssumeRole]] - AWS temporary credentials
- **Basic Auth**: [[API-Key-Basic]] - Simple API key authentication

## See Also

- [[Core-Concepts|Core Concepts]] - понимание Credential trait
- [[Rotate-Credentials|Rotation Guide]] - credential rotation patterns
- [[API-Reference|API Reference]] - полная документация

**Spec References**:
- [[../../specs/001-credential-docs/architecture.md#credential-trait-hierarchy]]
- [[../../specs/001-credential-docs/security-spec.md#database-security]]
- [[../../specs/001-credential-docs/technical-design.md#database-credentials]]

## Sources

- [mysql_async Documentation](https://docs.rs/mysql_async/)
- [nebula-credential API](../Reference/API-Reference.md)
