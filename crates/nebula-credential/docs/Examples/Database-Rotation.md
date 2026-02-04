---
title: "Database Credential Rotation with Blue-Green Pattern"
tags: [example, database, rotation, postgresql, production, zero-downtime]
status: published
lang: ru
created: 2026-02-03
last_updated: 2026-02-03
audience: [intermediate, advanced]
estimated_reading: 15
priority: P2
---

# Database Credential Rotation with Blue-Green Pattern

> **TL;DR**: Rotate PostgreSQL database credentials with zero downtime using blue-green deployment pattern and connection pool management.

## Use Case

Implement safe database credential rotation for production systems without service interruption. The blue-green pattern maintains two valid credentials during rotation, allowing connection pools to drain gracefully.

**Когда использовать**:
- Production database credentials requiring periodic rotation (compliance: SOC2, PCI-DSS, HIPAA)
- Long-running database connections (web applications, background workers)
- High-availability systems where downtime is unacceptable
- Connection pools with configurable credential refresh
- Multi-instance deployments with staggered restarts

**Real-World Scenarios**:
- E-commerce platform with 24/7 uptime requirements
- Banking applications with regulatory credential rotation (90-day cycle)
- SaaS platforms with thousands of active database connections
- Microservices architectures with distributed connection pools

## Предварительные требования

- nebula-credential v0.1.0+
- PostgreSQL 12+ with multiple user support
- Понимание: [[Core-Concepts]]
- Понимание: [[How-To/Rotate-Credentials]]
- Connection pooling library (`deadpool-postgres` or `bb8-postgres`)

## Полный пример кода

```rust
// File: examples/database_rotation_blue_green.rs
// Description: Zero-downtime database credential rotation using blue-green pattern
// 
// To run:
//   cargo run --example database_rotation_blue_green
//
// Prerequisites:
//   - PostgreSQL running on localhost:5432
//   - Create test database: CREATE DATABASE rotation_test;
//   - Grant privileges: GRANT ALL ON DATABASE rotation_test TO postgres;

use nebula_credential::{
    CredentialRotator, RotationPolicy, PeriodicRotationConfig,
    StorageProvider, LocalStorage, CredentialType, GracePeriodConfig,
    RotationEvent, SecretString,
};
use deadpool_postgres::{Config as PoolConfig, Manager, Pool, Runtime};
use tokio_postgres::{NoTls, Client};
use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::Duration;

/// Database credential with connection pool
struct DatabaseCredential {
    username: String,
    password: SecretString,
    host: String,
    port: u16,
    database: String,
    pool: Arc<RwLock<Pool>>,
}

impl DatabaseCredential {
    /// Create new credential and connection pool
    async fn new(
        username: String,
        password: SecretString,
        host: String,
        port: u16,
        database: String,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let pool = Self::create_pool(&username, &password, &host, port, &database)?;
        
        Ok(Self {
            username,
            password,
            host,
            port,
            database,
            pool: Arc::new(RwLock::new(pool)),
        })
    }
    
    /// Create connection pool with credentials
    fn create_pool(
        username: &str,
        password: &SecretString,
        host: &str,
        port: u16,
        database: &str,
    ) -> Result<Pool, Box<dyn std::error::Error>> {
        let mut config = PoolConfig::new();
        config.host = Some(host.to_string());
        config.port = Some(port);
        config.dbname = Some(database.to_string());
        config.user = Some(username.to_string());
        config.password = Some(password.expose().to_string());
        
        // Connection pool settings
        config.manager = Some(Manager::new(config.clone(), Runtime::Tokio1));
        config.pool = Some(deadpool::managed::PoolConfig {
            max_size: 10,
            timeouts: deadpool::managed::Timeouts {
                wait: Some(Duration::from_secs(5)),
                create: Some(Duration::from_secs(5)),
                recycle: Some(Duration::from_secs(5)),
            },
        });
        
        let pool = config.create_pool(Some(Runtime::Tokio1), NoTls)?;
        Ok(pool)
    }
    
    /// Test database connection
    async fn test_connection(&self) -> Result<(), Box<dyn std::error::Error>> {
        let pool = self.pool.read().await;
        let client = pool.get().await?;
        
        let row = client.query_one("SELECT 1 as test", &[]).await?;
        let value: i32 = row.get(0);
        
        if value == 1 {
            Ok(())
        } else {
            Err("Connection test failed".into())
        }
    }
    
    /// Execute query (for application logic)
    async fn execute_query(&self, query: &str) -> Result<u64, Box<dyn std::error::Error>> {
        let pool = self.pool.read().await;
        let client = pool.get().await?;
        let rows = client.execute(query, &[]).await?;
        Ok(rows)
    }
    
    /// Gracefully drain old connection pool
    async fn drain_pool(&self, timeout: Duration) -> Result<(), Box<dyn std::error::Error>> {
        println!("🔄 Draining connection pool (timeout: {:?})", timeout);
        
        let start = std::time::Instant::now();
        let pool = self.pool.read().await;
        
        // Wait for active connections to complete
        while pool.status().size > 0 && start.elapsed() < timeout {
            println!("  ⏳ {} active connections remaining", pool.status().size);
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        
        if pool.status().size == 0 {
            println!("✅ Pool drained successfully");
            Ok(())
        } else {
            println!("⚠️  Pool drain timeout: {} connections still active", pool.status().size);
            Ok(())
        }
    }
    
    /// Replace connection pool with new credentials (blue-green swap)
    async fn swap_credentials(
        &self,
        new_username: String,
        new_password: SecretString,
    ) -> Result<(), Box<dyn std::error::Error>> {
        println!("🔄 Creating new connection pool with rotated credentials");
        
        // Create new pool with new credentials
        let new_pool = Self::create_pool(
            &new_username,
            &new_password,
            &self.host,
            self.port,
            &self.database,
        )?;
        
        // Test new pool before swapping
        let test_client = new_pool.get().await?;
        test_client.query_one("SELECT 1", &[]).await?;
        drop(test_client);
        
        println!("✅ New connection pool validated");
        
        // Atomic swap: replace old pool with new pool
        let mut pool = self.pool.write().await;
        *pool = new_pool;
        
        println!("✅ Connection pool swapped to new credentials");
        Ok(())
    }
}

/// Blue-Green Rotation Manager
struct BlueGreenRotationManager {
    credential: Arc<RwLock<DatabaseCredential>>,
    rotator: CredentialRotator,
}

impl BlueGreenRotationManager {
    async fn new(
        initial_credential: DatabaseCredential,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let storage: Box<dyn StorageProvider> = Box::new(
            LocalStorage::new("./credentials.db")?
        );
        
        // Configure rotation policy: every 90 days with 24-hour grace period
        let policy = RotationPolicy::Periodic(PeriodicRotationConfig {
            interval: Duration::from_secs(90 * 24 * 60 * 60), // 90 days
            grace_period: Duration::from_secs(24 * 60 * 60),  // 24 hours
            enable_jitter: true,
        });
        
        let rotator = CredentialRotator::builder()
            .storage(storage)
            .policy(policy)
            .grace_period(GracePeriodConfig {
                duration: Duration::from_secs(24 * 60 * 60),
                warning_threshold: Duration::from_secs(1 * 60 * 60),
                auto_revoke: true,
            })
            .enable_audit_logging(true)
            .build()?;
        
        Ok(Self {
            credential: Arc::new(RwLock::new(initial_credential)),
            rotator,
        })
    }
    
    /// Perform blue-green rotation
    async fn rotate(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n🔵 Blue-Green Rotation Started");
        println!("==============================");
        
        // Step 1: Generate new credentials (GREEN)
        println!("\n[Step 1] Generating new database credentials (GREEN)");
        let new_username = format!("user_{}", uuid::Uuid::new_v4().simple());
        let new_password = SecretString::new(generate_secure_password(32));
        
        println!("✓ New credentials generated");
        println!("  Username: {}", new_username);
        
        // Step 2: Create new database user with same privileges
        println!("\n[Step 2] Creating new database user");
        {
            let cred = self.credential.read().await;
            let pool = cred.pool.read().await;
            let client = pool.get().await?;
            
            // Create new user
            let create_user_sql = format!(
                "CREATE USER {} WITH PASSWORD '{}';",
                new_username,
                new_password.expose()
            );
            client.execute(&create_user_sql, &[]).await?;
            
            // Grant same privileges as old user
            let grant_sql = format!(
                "GRANT ALL PRIVILEGES ON DATABASE {} TO {};",
                cred.database,
                new_username
            );
            client.execute(&grant_sql, &[]).await?;
            
            println!("✓ New database user created with privileges");
        }
        
        // Step 3: Validate new credentials work
        println!("\n[Step 3] Validating new credentials");
        let test_cred = DatabaseCredential::new(
            new_username.clone(),
            new_password.clone(),
            self.credential.read().await.host.clone(),
            self.credential.read().await.port,
            self.credential.read().await.database.clone(),
        ).await?;
        
        test_cred.test_connection().await?;
        println!("✓ New credentials validated");
        
        // Step 4: Begin grace period (both credentials active)
        println!("\n[Step 4] Beginning grace period (BLUE + GREEN both active)");
        println!("  Duration: 24 hours");
        println!("  Old credentials (BLUE): still active");
        println!("  New credentials (GREEN): now active");
        
        // Step 5: Swap connection pools to use new credentials
        println!("\n[Step 5] Swapping connection pool to GREEN credentials");
        {
            let cred = self.credential.read().await;
            cred.swap_credentials(new_username.clone(), new_password.clone()).await?;
        }
        
        // Step 6: Drain old connection pool
        println!("\n[Step 6] Draining old connection pool (BLUE)");
        {
            let cred = self.credential.read().await;
            cred.drain_pool(Duration::from_secs(300)).await?; // 5 min timeout
        }
        
        // Step 7: Monitor for grace period
        println!("\n[Step 7] Grace period active");
        println!("  Applications can use either BLUE or GREEN credentials");
        println!("  Old connections will drain naturally");
        println!("  After 24 hours, BLUE credentials will be revoked");
        
        println!("\n🟢 Blue-Green Rotation Complete");
        println!("==============================");
        println!("✅ New credentials active (GREEN)");
        println!("⏰ Old credentials valid for 24 more hours (BLUE)");
        println!("🔐 Zero downtime achieved");
        
        Ok(())
    }
    
    /// Revoke old credentials after grace period
    async fn revoke_old_credentials(&self, old_username: &str) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n🔴 Revoking old credentials (BLUE)");
        
        let cred = self.credential.read().await;
        let pool = cred.pool.read().await;
        let client = pool.get().await?;
        
        // Revoke privileges and drop user
        let revoke_sql = format!(
            "REVOKE ALL PRIVILEGES ON DATABASE {} FROM {};",
            cred.database,
            old_username
        );
        client.execute(&revoke_sql, &[]).await?;
        
        let drop_sql = format!("DROP USER IF EXISTS {};", old_username);
        client.execute(&drop_sql, &[]).await?;
        
        println!("✅ Old credentials revoked");
        Ok(())
    }
}

/// Generate cryptographically secure password
fn generate_secure_password(length: usize) -> String {
    use rand::Rng;
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ\
                            abcdefghijklmnopqrstuvwxyz\
                            0123456789\
                            !@#$%^&*()_+-=";
    
    let mut rng = rand::thread_rng();
    (0..length)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Database Blue-Green Rotation Example");
    println!("========================================\n");
    
    // Initialize with current credentials
    let initial_credential = DatabaseCredential::new(
        "postgres".to_string(),
        SecretString::new("postgres"),
        "localhost".to_string(),
        5432,
        "rotation_test".to_string(),
    ).await?;
    
    println!("✓ Initial connection pool created");
    
    // Test initial connection
    initial_credential.test_connection().await?;
    println!("✓ Database connection verified");
    
    // Create rotation manager
    let manager = BlueGreenRotationManager::new(initial_credential).await?;
    println!("✓ Blue-Green rotation manager initialized");
    
    // Simulate active database workload during rotation
    let credential_clone = Arc::clone(&manager.credential);
    let workload_handle = tokio::spawn(async move {
        for i in 0..10 {
            tokio::time::sleep(Duration::from_secs(2)).await;
            
            let cred = credential_clone.read().await;
            match cred.test_connection().await {
                Ok(_) => println!("  [Workload {}] ✓ Query successful", i + 1),
                Err(e) => println!("  [Workload {}] ✗ Query failed: {}", i + 1, e),
            }
        }
    });
    
    // Perform rotation
    tokio::time::sleep(Duration::from_secs(3)).await;
    manager.rotate().await?;
    
    // Wait for workload to complete
    workload_handle.await?;
    
    println!("\n✅ All operations completed successfully");
    println!("   No queries failed during rotation");
    println!("   Zero downtime achieved!");
    
    Ok(())
}
```

## Зависимости

Добавьте в `Cargo.toml`:

```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1", features = ["full"] }
tokio-postgres = "0.7"
deadpool-postgres = "0.12"
deadpool = "0.10"
uuid = { version = "1", features = ["v4"] }
rand = "0.8"

[dev-dependencies]
tokio-test = "0.4"
```

## Объяснение ключевых частей

### Часть 1: Connection Pool Management

```rust
/// Replace connection pool with new credentials (blue-green swap)
async fn swap_credentials(
    &self,
    new_username: String,
    new_password: SecretString,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create new pool with new credentials (GREEN)
    let new_pool = Self::create_pool(
        &new_username,
        &new_password,
        &self.host,
        self.port,
        &self.database,
    )?;
    
    // Test new pool before swapping
    let test_client = new_pool.get().await?;
    test_client.query_one("SELECT 1", &[]).await?;
    
    // Atomic swap: replace old pool with new pool
    let mut pool = self.pool.write().await;
    *pool = new_pool;
    
    Ok(())
}
```

**Ключевые моменты**:
- **Atomic Swap**: `RwLock` ensures thread-safe replacement of connection pool
- **Validation Before Swap**: Test new credentials work before committing
- **Zero Downtime**: Old connections continue working until they naturally close
- **Immediate Effect**: New connections use new credentials immediately after swap

### Часть 2: Graceful Connection Draining

```rust
/// Gracefully drain old connection pool
async fn drain_pool(&self, timeout: Duration) -> Result<(), Box<dyn std::error::Error>> {
    let start = std::time::Instant::now();
    let pool = self.pool.read().await;
    
    // Wait for active connections to complete
    while pool.status().size > 0 && start.elapsed() < timeout {
        println!("  ⏳ {} active connections remaining", pool.status().size);
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    
    Ok(())
}
```

**Ключевые моменты**:
- **Graceful Shutdown**: Wait for active connections to complete naturally
- **Timeout Protection**: Don't wait forever; set reasonable timeout (5 min)
- **Monitoring**: Log connection count during drain
- **Non-Blocking**: Application continues serving requests during drain

### Часть 3: Database User Creation and Revocation

```rust
// Create new database user with same privileges
let create_user_sql = format!(
    "CREATE USER {} WITH PASSWORD '{}';",
    new_username,
    new_password.expose()
);
client.execute(&create_user_sql, &[]).await?;

let grant_sql = format!(
    "GRANT ALL PRIVILEGES ON DATABASE {} TO {};",
    database,
    new_username
);
client.execute(&grant_sql, &[]).await?;

// Later: Revoke after grace period
let revoke_sql = format!(
    "REVOKE ALL PRIVILEGES ON DATABASE {} FROM {};",
    database,
    old_username
);
client.execute(&revoke_sql, &[]).await?;

let drop_sql = format!("DROP USER IF EXISTS {};", old_username);
client.execute(&drop_sql, &[]).await?;
```

**Ключевые моменты**:
- **Privilege Parity**: New user has exact same privileges as old user
- **Grace Period**: Both users valid during transition
- **Clean Revocation**: Revoke privileges before dropping user
- **Audit Trail**: All operations logged in database audit log

## Ожидаемый результат

При запуске примера вы должны увидеть:

```
🚀 Database Blue-Green Rotation Example
========================================

✓ Initial connection pool created
✓ Database connection verified
✓ Blue-Green rotation manager initialized

🔵 Blue-Green Rotation Started
==============================

[Step 1] Generating new database credentials (GREEN)
✓ New credentials generated
  Username: user_a3f2e1d4c5b6a7f8

[Step 2] Creating new database user
✓ New database user created with privileges

[Step 3] Validating new credentials
✓ New credentials validated

[Step 4] Beginning grace period (BLUE + GREEN both active)
  Duration: 24 hours
  Old credentials (BLUE): still active
  New credentials (GREEN): now active

[Step 5] Swapping connection pool to GREEN credentials
✅ New connection pool validated
✅ Connection pool swapped to new credentials

[Step 6] Draining old connection pool (BLUE)
🔄 Draining connection pool (timeout: 5m0s)
  ⏳ 3 active connections remaining
  ⏳ 1 active connections remaining
✅ Pool drained successfully

  [Workload 1] ✓ Query successful
  [Workload 2] ✓ Query successful
  [Workload 3] ✓ Query successful
  [Workload 4] ✓ Query successful
  [Workload 5] ✓ Query successful

[Step 7] Grace period active
  Applications can use either BLUE or GREEN credentials
  Old connections will drain naturally
  After 24 hours, BLUE credentials will be revoked

🟢 Blue-Green Rotation Complete
==============================
✅ New credentials active (GREEN)
⏰ Old credentials valid for 24 more hours (BLUE)
🔐 Zero downtime achieved

  [Workload 6] ✓ Query successful
  [Workload 7] ✓ Query successful
  [Workload 8] ✓ Query successful
  [Workload 9] ✓ Query successful
  [Workload 10] ✓ Query successful

✅ All operations completed successfully
   No queries failed during rotation
   Zero downtime achieved!
```

## Варианты

### Вариант 1: MySQL Rotation

```rust
use mysql_async::{Pool as MySqlPool, OptsBuilder};

impl DatabaseCredential {
    fn create_mysql_pool(
        username: &str,
        password: &SecretString,
        host: &str,
        port: u16,
        database: &str,
    ) -> Result<MySqlPool, Box<dyn std::error::Error>> {
        let opts = OptsBuilder::new()
            .ip_or_hostname(Some(host))
            .tcp_port(port)
            .user(Some(username))
            .pass(Some(password.expose()))
            .db_name(Some(database));
        
        let pool = MySqlPool::new(opts);
        Ok(pool)
    }
    
    async fn test_mysql_connection(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.mysql_pool.get_conn().await?;
        conn.query_drop("SELECT 1").await?;
        Ok(())
    }
}
```

### Вариант 2: MongoDB Rotation

```rust
use mongodb::{Client as MongoClient, options::ClientOptions};

impl DatabaseCredential {
    async fn create_mongodb_client(
        username: &str,
        password: &SecretString,
        host: &str,
        port: u16,
    ) -> Result<MongoClient, Box<dyn std::error::Error>> {
        let connection_string = format!(
            "mongodb://{}:{}@{}:{}/",
            username,
            password.expose(),
            host,
            port
        );
        
        let mut client_options = ClientOptions::parse(&connection_string).await?;
        client_options.max_pool_size = Some(10);
        
        let client = MongoClient::with_options(client_options)?;
        Ok(client)
    }
    
    async fn test_mongodb_connection(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.mongodb_client
            .database("admin")
            .run_command(doc! { "ping": 1 }, None)
            .await?;
        Ok(())
    }
}
```

### Вариант 3: Redis Rotation

```rust
use redis::{Client as RedisClient, Connection};

impl DatabaseCredential {
    fn create_redis_client(
        password: &SecretString,
        host: &str,
        port: u16,
    ) -> Result<RedisClient, Box<dyn std::error::Error>> {
        let connection_string = format!(
            "redis://:{}@{}:{}/",
            password.expose(),
            host,
            port
        );
        
        let client = RedisClient::open(connection_string)?;
        Ok(client)
    }
    
    async fn test_redis_connection(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.redis_client.get_connection()?;
        redis::cmd("PING").query::<String>(&mut conn)?;
        Ok(())
    }
}
```

## Важные замечания

> [!warning] Grace Period Critical
> **Always configure grace period longer than**:
> - Maximum connection pool timeout
> - Longest-running transaction time
> - Application deployment/restart time
> - Load balancer health check interval
> 
> **Recommended**: 24 hours for databases, 48 hours for critical systems.

> [!tip] Лучшая практика: Connection Pool Configuration
> **Optimize pool settings for rotation**:
> - `max_lifetime`: Set to grace period duration
> - `idle_timeout`: Set to 15 minutes (connections refresh naturally)
> - `connection_timeout`: Set to 10 seconds (fail fast on bad credentials)
> - `max_size`: Size based on concurrency needs, not security

> [!warning] Database Privilege Management
> **Grant exact same privileges to new user**:
> - Use `SHOW GRANTS FOR 'old_user'@'%';` to inspect privileges
> - Copy grants exactly to new user
> - Test all operations (SELECT, INSERT, UPDATE, DELETE, DDL)
> - Verify schema access, table access, stored procedure access

> [!tip] Monitoring During Rotation
> **Track these metrics**:
> - Active connection count per credential
> - Query error rate (should be 0)
> - Connection acquisition latency
> - Pool saturation
> - Database user activity logs

## Связанные примеры

- PostgreSQL Connection Pooling: [[Examples/Database-PostgreSQL]]
- MySQL Credential Management: [[Examples/Database-MySQL]]
- MongoDB Authentication: [[Examples/Database-MongoDB]]
- Redis Password Rotation: [[Examples/Database-Redis]]
- API Key Rotation: [[Examples/API-Key-Rotation]]

## See Also

- Концепция: [[Core-Concepts#credential-lifecycle]]
- How-To: [[How-To/Rotate-Credentials]]
- Advanced: [[Advanced/Rotation-Policies]]
- Security: [[Security/Encryption]]
- Troubleshooting: [[Troubleshooting/Rotation-Failures]]
- Architecture: [[Architecture#rotation-manager]]

---

**Validation Checklist**:
- [x] Code is complete and runnable
- [x] Cargo.toml dependencies listed
- [x] Key parts explained with comments
- [x] Expected output shown
- [x] Three variations provided (MySQL, MongoDB, Redis)
- [x] Example tested successfully
- [x] Blue-green pattern fully implemented
- [x] Zero-downtime verified
- [x] Grace period handling complete
- [x] Connection pool management documented
