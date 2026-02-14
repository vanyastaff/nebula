---
title: "MongoDB Database Credentials"
tags: [example, database, mongodb, nosql, intermediate]
status: published
lang: ru
created: 2026-02-03
last_updated: 2026-02-03
audience: [intermediate]
estimated_reading: 10
priority: P1
---

# MongoDB Database Credentials

> **TL;DR**: Используйте `Credential` trait из nebula-credential для безопасного хранения MongoDB connection strings с поддержкой SCRAM-SHA-256 authentication и replica sets.

## Обзор

MongoDB credentials в nebula-credential управляются через `Credential` trait pattern с поддержкой:
- **Secure Storage**: Connection URIs хранятся в `SecretString`
- **Authentication**: SCRAM-SHA-256, X.509, AWS IAM mechanisms
- **Testing**: `TestableCredential` для проверки подключения
- **Cloud Support**: MongoDB Atlas, AWS DocumentDB, Azure Cosmos DB

**Architecture Reference**: См. [[../../specs/001-credential-docs/architecture.md#credential-trait-hierarchy]]  
**Security**: См. [[../../specs/001-credential-docs/security-spec.md#database-security]]

## Prerequisites

- [[Installation|Установлен nebula-credential]]
- [[Core-Concepts|Понимание Credential trait]]
- MongoDB server (v4.4+ или MongoDB Atlas)

## Complete Example

### Dependencies

```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1", features = ["full"] }
mongodb = "2.8"
bson = "2.9"
serde = { version = "1.0", features = ["derive"] }
```

### Implementing MongoDB Credential

```rust
// File: examples/mongodb_credential.rs
use nebula_credential::{
    Credential, CredentialContext, CredentialError,
    TestableCredential, SecretString,
    TestResult, TestDetails, OwnerId,
};
use async_trait::async_trait;
use mongodb::{Client, options::ClientOptions, bson::doc};
use std::collections::HashMap;

/// MongoDB credential configuration
#[derive(Debug, Clone)]
pub struct MongodbConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub auth_source: String,
    pub replica_set: Option<String>,
    pub tls_enabled: bool,
}

/// MongoDB credential output
#[derive(Debug, Clone)]
pub struct MongodbCredential {
    pub config: MongodbConfig,
    pub password: SecretString,
    pub connection_uri: SecretString,
}

impl MongodbCredential {
    pub fn new(config: MongodbConfig, password: SecretString) -> Self {
        let replica_set = config.replica_set.as_ref()
            .map(|rs| format!("&replicaSet={}", rs))
            .unwrap_or_default();

        let tls = if config.tls_enabled { "&tls=true" } else { "" };

        let connection_uri = SecretString::new(format!(
            "mongodb://{}:{}@{}:{}/{}?authSource={}{}{}",
            config.username,
            password.expose(),
            config.host,
            config.port,
            config.database,
            config.auth_source,
            replica_set,
            tls
        ));

        Self {
            config,
            password,
            connection_uri,
        }
    }

    /// Get redacted connection URI for logging
    pub fn connection_uri_redacted(&self) -> String {
        let replica_set = self.config.replica_set.as_ref()
            .map(|rs| format!("?replicaSet={}", rs))
            .unwrap_or_default();

        format!(
            "mongodb://{}:***@{}:{}/{}{}",
            self.config.username,
            self.config.host,
            self.config.port,
            self.config.database,
            replica_set
        )
    }
}

/// MongoDB credential provider
pub struct MongodbCredentialProvider {
    config: MongodbConfig,
    password_source: PasswordSource,
}

#[derive(Debug, Clone)]
pub enum PasswordSource {
    Environment(String), // Env var name
    Static(SecretString),
    SecretsManager { path: String },
}

impl MongodbCredentialProvider {
    pub fn new(config: MongodbConfig, password_source: PasswordSource) -> Self {
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
impl Credential for MongodbCredentialProvider {
    type Output = MongodbCredential;
    type Error = CredentialError;

    async fn retrieve(
        &self,
        _ctx: &CredentialContext,
    ) -> Result<Self::Output, Self::Error> {
        let password = self.resolve_password().await?;
        Ok(MongodbCredential::new(self.config.clone(), password))
    }

    fn credential_type(&self) -> &'static str {
        "mongodb"
    }

    fn supports_refresh(&self) -> bool {
        false // Connection string doesn't expire
    }
}

#[async_trait]
impl TestableCredential for MongodbCredentialProvider {
    async fn test(
        &self,
        ctx: &CredentialContext,
    ) -> Result<TestResult, CredentialError> {
        let start = std::time::Instant::now();
        let credential = self.retrieve(ctx).await?;

        // Parse connection URI
        let client_options = ClientOptions::parse(credential.connection_uri.expose())
            .await
            .map_err(|e| CredentialError::ConfigurationError(
                format!("Failed to parse connection URI: {}", e)
            ))?;

        // Attempt connection
        let client = Client::with_options(client_options)
            .map_err(|e| CredentialError::ConnectionError(
                format!("Failed to create client: {}", e)
            ))?;

        // Test connection with ping
        match client.database("admin").run_command(doc! { "ping": 1 }, None).await {
            Ok(_) => {
                let latency_ms = start.elapsed().as_millis() as u64;

                // Get server version
                let build_info = client
                    .database("admin")
                    .run_command(doc! { "buildInfo": 1 }, None)
                    .await
                    .ok();

                let version = build_info
                    .and_then(|doc| doc.get_str("version").ok())
                    .unwrap_or("unknown")
                    .to_string();

                let details = TestDetails {
                    latency_ms,
                    endpoint_tested: credential.connection_uri_redacted(),
                    permissions_verified: vec!["ping".to_string()],
                    metadata: HashMap::from([
                        ("mongodb_version".to_string(), 
                         serde_json::json!(version)),
                        ("database".to_string(), 
                         serde_json::json!(credential.config.database)),
                    ]),
                };

                Ok(TestResult::success("MongoDB connection successful")
                    .with_details(details))
            }
            Err(e) => {
                Ok(TestResult::failure(format!("Connection failed: {}", e)))
            }
        }
    }

    fn test_description(&self) -> &str {
        "Testing MongoDB connection by executing ping command"
    }
}
```

### Usage Example

```rust
// File: examples/use_mongodb_credential.rs
use nebula_credential::{
    Credential, TestableCredential, CredentialContext, OwnerId,
};
use mongodb::{Client, options::ClientOptions, bson::doc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
struct User {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    id: Option<bson::oid::ObjectId>,
    username: String,
    email: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("📊 MongoDB Credential Example\n");

    // 1. Configure MongoDB credential
    let config = MongodbConfig {
        host: "localhost".to_string(),
        port: 27017,
        database: "myapp_db".to_string(),
        username: "app_user".to_string(),
        auth_source: "admin".to_string(),
        replica_set: None,
        tls_enabled: false,
    };

    let provider = MongodbCredentialProvider::new(
        config,
        PasswordSource::Environment("MONGO_PASSWORD".to_string()),
    );

    // 2. Create credential context
    let ctx = CredentialContext::new(OwnerId::new("my-workflow"))
        .with_metadata("environment".to_string(), "production".to_string());

    // 3. Retrieve credential
    println!("📡 Retrieving MongoDB credential...");
    let credential = provider.retrieve(&ctx).await?;
    
    println!("✅ Credential retrieved");
    println!("   Connection: {}", credential.connection_uri_redacted());
    println!("   Credential type: {}", provider.credential_type());

    // 4. Test credential validity
    println!("\n🧪 Testing credential...");
    let test_result = provider.test(&ctx).await?;

    if test_result.success {
        println!("✅ Test passed: {}", test_result.message);
        if let Some(details) = test_result.details {
            println!("   Latency: {}ms", details.latency_ms);
            println!("   Version: {:?}", details.metadata.get("mongodb_version"));
        }
    } else {
        println!("❌ Test failed: {}", test_result.message);
    }

    // 5. Use with MongoDB client
    println!("\n📝 Using MongoDB client...");
    
    let client_options = ClientOptions::parse(credential.connection_uri.expose()).await?;
    let client = Client::with_options(client_options)?;
    let db = client.database(&credential.config.database);
    let users = db.collection::<User>("users");

    // Insert document
    let user = User {
        id: None,
        username: "alice".to_string(),
        email: "alice@example.com".to_string(),
    };
    
    users.insert_one(user, None).await?;
    println!("   ✓ User inserted");

    // Find document
    let found = users.find_one(doc! { "username": "alice" }, None).await?;
    if let Some(u) = found {
        println!("   ✓ User found: {} ({})", u.username, u.email);
    }

    Ok(())
}
```

## Key Concepts

### 1. Credential Trait для MongoDB

```rust
impl Credential for MongodbCredentialProvider {
    type Output = MongodbCredential; // Содержит connection URI
    type Error = CredentialError;
    
    async fn retrieve(&self, ctx: &CredentialContext) 
        -> Result<Self::Output, Self::Error> {
        // Resolve password from source
        // Build connection URI with SecretString
    }
}
```

**Architecture**: См. [[Architecture#credential-trait-hierarchy]]

### 2. SecretString для Connection URI

```rust
// ✅ GOOD: Connection URI auto-zeroized
let password = SecretString::new(env::var("MONGO_PASSWORD")?);
let connection_uri = SecretString::new(format!(
    "mongodb://user:{}@host/db", 
    password.expose()
));

// Redacted logging
println!("URI: {}", credential.connection_uri_redacted()); // Password shows as ***
```

**Security**: См. [[../../specs/001-credential-docs/security-spec.md#secret-handling]]

### 3. TestableCredential Implementation

```rust
impl TestableCredential for MongodbCredentialProvider {
    async fn test(&self, ctx: &CredentialContext) 
        -> Result<TestResult, CredentialError> {
        // Attempt connection
        // Execute ping command
        // Return result with MongoDB version
    }
}
```

### 4. MongoDB-Specific Configuration

```rust
let config = MongodbConfig {
    auth_source: "admin".to_string(), // Database for authentication
    replica_set: Some("rs0".to_string()), // Replica set name
    tls_enabled: true, // Enable TLS/SSL
    // ...
};
```

## Security Best Practices

> [!warning] Connection URI Security
> Используйте `SecretString` для connection URIs. Никогда не логируйте passwords.

**TLS/SSL Configuration**:
```rust
let config = MongodbConfig {
    tls_enabled: true, // ✅ Enable for production
    // ...
};

// Advanced TLS options via MongoDB client
use mongodb::options::{Tls, TlsOptions};

let tls_options = TlsOptions::builder()
    .ca_file_path("/path/to/ca.pem".into())
    .allow_invalid_certificates(false) // ✅ Verify certificates
    .build();
```

**Storage Integration**:
```rust
// Integration с AWS Secrets Manager
let provider = MongodbCredentialProvider::new(
    config,
    PasswordSource::SecretsManager { 
        path: "mongodb/production/password".to_string() 
    },
);
```

## Common Issues

### Issue 1: Authentication Failed

**Symptoms**: `CredentialError::AuthenticationFailed`

**Solution**: Проверить authSource:
```rust
let config = MongodbConfig {
    auth_source: "admin".to_string(), // ✅ Correct
    // ...
};
```

### Issue 2: Connection Timeout

**Symptoms**: Connection hangs

**Solution**: Увеличить timeouts через ClientOptions:
```rust
let mut client_options = ClientOptions::parse(uri).await?;
client_options.connect_timeout = Some(Duration::from_secs(30));
client_options.server_selection_timeout = Some(Duration::from_secs(60));
```

## Related Examples

- **Other Databases**: [[Database-PostgreSQL]] - PostgreSQL credentials | [[Database-MySQL]] - MySQL credentials | [[Database-Redis]] - Redis credentials
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

- [mongodb Rust Driver Documentation](https://docs.rs/mongodb/)
- [nebula-credential API](../Reference/API-Reference.md)
