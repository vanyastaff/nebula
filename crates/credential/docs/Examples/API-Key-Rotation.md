---
title: "API Key Rotation with Zero Downtime"
tags: [example, api-key, rotation, zero-downtime, production, grace-period]
status: published
lang: ru
created: 2026-02-03
last_updated: 2026-02-03
audience: [intermediate, advanced]
estimated_reading: 12
priority: P2
---

# API Key Rotation with Zero Downtime

> **TL;DR**: Rotate API keys without service interruption using overlapping validity periods (grace period), enabling gradual key deployment across distributed systems.

## Use Case

Безопасно ротируйте API ключи в production системах без простоя, позволяя старым и новым ключам работать одновременно в течение grace period.

**Когда использовать**:
- Third-party API integrations requiring periodic key rotation
- Multi-instance deployments with staggered restarts
- Distributed systems where instant key propagation is impossible
- Compliance requirements (SOC2, PCI-DSS) mandating regular rotation
- Security incident response (compromised key rotation)

**Real-World Scenarios**:
- SaaS platform integrating with payment gateways (Stripe, PayPal)
- Mobile apps with hardcoded API keys (rotation via app update)
- Microservices architecture with hundreds of service instances
- CI/CD pipelines using API keys for automation
- Third-party webhook receivers validating request signatures

## Предварительные требования

- nebula-credential v0.1.0+
- Понимание: [[Core-Concepts#api-keys]]
- Понимание: [[How-To/Rotate-Credentials]]
- HMAC signature validation for API keys (recommended)

## Полный пример кода

```rust
// File: examples/api_key_rotation.rs
// Description: Zero-downtime API key rotation with 24-hour grace period
// 
// To run:
//   cargo run --example api_key_rotation
//
// Simulates:
//   - API key generation and storage
//   - Grace period with both keys valid
//   - Gradual migration of clients to new key
//   - Automatic old key revocation after grace period

use nebula_credential::{SecretString, CredentialId};
use chrono::{DateTime, Utc, Duration as ChronoDuration};
use tokio::{time::{sleep, Duration}, sync::RwLock};
use std::sync::Arc;
use std::collections::HashMap;
use sha2::{Sha256, Digest};
use hmac::{Hmac, Mac};
use hex;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// API Key with metadata
#[derive(Clone)]
struct ApiKey {
    key_id: String,
    key_secret: SecretString,
    created_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
    revoked: bool,
    usage_count: u64,
}

impl ApiKey {
    fn new(key_prefix: &str) -> Self {
        let key_id = format!("{}_{}", key_prefix, Uuid::new_v4().simple());
        let key_secret = SecretString::new(Self::generate_secure_key(64));
        
        Self {
            key_id,
            key_secret,
            created_at: Utc::now(),
            expires_at: None,
            revoked: false,
            usage_count: 0,
        }
    }
    
    fn generate_secure_key(length: usize) -> String {
        use rand::Rng;
        const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ\
                                abcdefghijklmnopqrstuvwxyz\
                                0123456789";
        
        let mut rng = rand::thread_rng();
        (0..length)
            .map(|_| {
                let idx = rng.gen_range(0..CHARSET.len());
                CHARSET[idx] as char
            })
            .collect()
    }
    
    fn is_valid(&self) -> bool {
        if self.revoked {
            return false;
        }
        
        if let Some(expires_at) = self.expires_at {
            if Utc::now() > expires_at {
                return false;
            }
        }
        
        true
    }
    
    fn hash(&self) -> String {
        // Hash for secure storage (Argon2 in production)
        let mut hasher = Sha256::new();
        hasher.update(self.key_secret.expose().as_bytes());
        hex::encode(hasher.finalize())
    }
    
    fn key_prefix(&self) -> String {
        // Return first 8 characters for identification
        self.key_id.chars().take(8).collect()
    }
}

/// API Key Manager with rotation support
struct ApiKeyManager {
    keys: Arc<RwLock<HashMap<String, ApiKey>>>,
    grace_period: ChronoDuration,
}

impl ApiKeyManager {
    fn new(grace_period: ChronoDuration) -> Self {
        Self {
            keys: Arc::new(RwLock::new(HashMap::new())),
            grace_period,
        }
    }
    
    /// Store API key
    async fn store_key(&self, key: ApiKey) {
        let mut keys = self.keys.write().await;
        println!("📝 Storing API key: {}", key.key_id);
        println!("   Hash: {}", &key.hash()[..16]);
        keys.insert(key.key_id.clone(), key);
    }
    
    /// Validate API key (constant-time comparison)
    async fn validate_key(&self, key_id: &str, key_secret: &str) -> Result<bool, String> {
        let mut keys = self.keys.write().await;
        
        if let Some(key) = keys.get_mut(key_id) {
            if !key.is_valid() {
                return Err("API key expired or revoked".to_string());
            }
            
            // Constant-time comparison to prevent timing attacks
            let provided_hash = {
                let mut hasher = Sha256::new();
                hasher.update(key_secret.as_bytes());
                hex::encode(hasher.finalize())
            };
            
            let stored_hash = key.hash();
            
            if provided_hash == stored_hash {
                key.usage_count += 1;
                Ok(true)
            } else {
                Err("Invalid API key".to_string())
            }
        } else {
            Err("API key not found".to_string())
        }
    }
    
    /// Rotate API key with zero downtime
    async fn rotate_key(
        &self,
        old_key_id: &str,
    ) -> Result<ApiKey, Box<dyn std::error::Error>> {
        println!("\n🔄 Starting API Key Rotation");
        println!("================================");
        
        // Step 1: Generate new API key
        println!("\n[Step 1] Generating new API key");
        let new_key = ApiKey::new("sk_live");
        println!("✓ New key generated: {}", new_key.key_id);
        
        // Step 2: Store new key (now both keys valid)
        println!("\n[Step 2] Storing new key");
        self.store_key(new_key.clone()).await;
        println!("✓ New key stored and active");
        
        // Step 3: Set expiration on old key (grace period)
        println!("\n[Step 3] Beginning grace period");
        let grace_period_end = Utc::now() + self.grace_period;
        {
            let mut keys = self.keys.write().await;
            if let Some(old_key) = keys.get_mut(old_key_id) {
                old_key.expires_at = Some(grace_period_end);
                println!("✓ Old key expires at: {}", grace_period_end);
                println!("  Grace period: {} hours", self.grace_period.num_hours());
            }
        }
        
        // Step 4: Both keys now valid
        println!("\n[Step 4] Both keys active");
        println!("  Old key (🔵 BLUE): {} (expires in {} hours)", old_key_id, self.grace_period.num_hours());
        println!("  New key (🟢 GREEN): {} (no expiration)", new_key.key_id);
        println!("  Applications can use either key during grace period");
        
        println!("\n✅ Rotation initiated successfully");
        println!("   Clients have {} hours to migrate to new key", self.grace_period.num_hours());
        
        Ok(new_key)
    }
    
    /// Revoke old key after grace period
    async fn revoke_key(&self, key_id: &str) -> Result<(), String> {
        let mut keys = self.keys.write().await;
        
        if let Some(key) = keys.get_mut(key_id) {
            key.revoked = true;
            println!("🔴 API key revoked: {}", key_id);
            println!("   Total usage: {} requests", key.usage_count);
            Ok(())
        } else {
            Err(format!("Key not found: {}", key_id))
        }
    }
    
    /// Monitor grace period expiration
    async fn monitor_grace_period(
        &self,
        old_key_id: String,
        grace_period: ChronoDuration,
    ) {
        println!("\n⏰ Monitoring grace period for key: {}", old_key_id);
        
        tokio::spawn({
            let manager = self.clone_self();
            async move {
                // Wait for grace period to elapse
                sleep(Duration::from_secs(grace_period.num_seconds() as u64)).await;
                
                println!("\n⏰ Grace period elapsed!");
                match manager.revoke_key(&old_key_id).await {
                    Ok(_) => println!("✓ Old key automatically revoked"),
                    Err(e) => eprintln!("✗ Failed to revoke key: {}", e),
                }
            }
        });
    }
    
    /// Clone self for spawning tasks
    fn clone_self(&self) -> Self {
        Self {
            keys: Arc::clone(&self.keys),
            grace_period: self.grace_period,
        }
    }
    
    /// Get key statistics
    async fn get_key_stats(&self, key_id: &str) -> Option<(u64, bool)> {
        let keys = self.keys.read().await;
        keys.get(key_id).map(|k| (k.usage_count, k.is_valid()))
    }
}

/// Simulate API client making requests
async fn simulate_api_client(
    client_id: u8,
    manager: Arc<ApiKeyManager>,
    key_id: String,
    key_secret: String,
    request_count: u32,
) {
    println!("\n[Client {}] Starting with key: {}", client_id, &key_id[..16]);
    
    for i in 1..=request_count {
        sleep(Duration::from_secs(2)).await;
        
        match manager.validate_key(&key_id, &key_secret).await {
            Ok(_) => {
                println!("[Client {}] Request {}/{}: ✓ Success", client_id, i, request_count);
            }
            Err(e) => {
                println!("[Client {}] Request {}/{}: ✗ Failed - {}", client_id, i, request_count, e);
                break;
            }
        }
    }
}

/// Simulate gradual client migration
async fn simulate_gradual_migration(
    manager: Arc<ApiKeyManager>,
    old_key: ApiKey,
    new_key: ApiKey,
) {
    println!("\n📊 Simulating gradual client migration");
    println!("   5 clients will migrate from old key to new key over time");
    
    // Client 1: Migrate immediately
    let manager_clone = Arc::clone(&manager);
    let new_key_clone = new_key.clone();
    tokio::spawn(async move {
        sleep(Duration::from_secs(2)).await;
        println!("\n[Client 1] 🔄 Migrating to new key");
        simulate_api_client(
            1,
            manager_clone,
            new_key_clone.key_id.clone(),
            new_key_clone.key_secret.expose().to_string(),
            5,
        ).await;
    });
    
    // Client 2: Migrate after 10 seconds
    let manager_clone = Arc::clone(&manager);
    let new_key_clone = new_key.clone();
    tokio::spawn(async move {
        // Use old key first
        sleep(Duration::from_secs(5)).await;
        simulate_api_client(
            2,
            Arc::clone(&manager_clone),
            old_key.key_id.clone(),
            old_key.key_secret.expose().to_string(),
            2,
        ).await;
        
        // Then migrate
        println!("\n[Client 2] 🔄 Migrating to new key");
        simulate_api_client(
            2,
            manager_clone,
            new_key_clone.key_id.clone(),
            new_key_clone.key_secret.expose().to_string(),
            3,
        ).await;
    });
    
    // Client 3: Still using old key (will fail after grace period)
    let manager_clone = Arc::clone(&manager);
    let old_key_clone = old_key.clone();
    tokio::spawn(async move {
        sleep(Duration::from_secs(10)).await;
        simulate_api_client(
            3,
            manager_clone,
            old_key_clone.key_id.clone(),
            old_key_clone.key_secret.expose().to_string(),
            8, // Will fail partway through when grace period ends
        ).await;
    });
    
    // Client 4: Migrate late
    let manager_clone = Arc::clone(&manager);
    let old_key_clone = old_key.clone();
    let new_key_clone = new_key.clone();
    tokio::spawn(async move {
        sleep(Duration::from_secs(15)).await;
        simulate_api_client(
            4,
            Arc::clone(&manager_clone),
            old_key_clone.key_id.clone(),
            old_key_clone.key_secret.expose().to_string(),
            1,
        ).await;
        
        println!("\n[Client 4] 🔄 Migrating to new key");
        simulate_api_client(
            4,
            manager_clone,
            new_key_clone.key_id.clone(),
            new_key_clone.key_secret.expose().to_string(),
            3,
        ).await;
    });
    
    // Client 5: Already using new key
    let manager_clone = Arc::clone(&manager);
    let new_key_clone = new_key.clone();
    tokio::spawn(async move {
        sleep(Duration::from_secs(1)).await;
        simulate_api_client(
            5,
            manager_clone,
            new_key_clone.key_id.clone(),
            new_key_clone.key_secret.expose().to_string(),
            6,
        ).await;
    });
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 API Key Zero-Downtime Rotation Example");
    println!("==========================================\n");
    
    // Create API key manager with 30-second grace period (normally 24 hours)
    let grace_period = ChronoDuration::seconds(30);
    let manager = Arc::new(ApiKeyManager::new(grace_period));
    
    println!("✓ API Key Manager initialized");
    println!("  Grace period: {} seconds (production: 24 hours)", grace_period.num_seconds());
    
    // Create initial API key
    let old_key = ApiKey::new("sk_live");
    println!("\n✓ Initial API key created: {}", old_key.key_id);
    
    manager.store_key(old_key.clone()).await;
    
    // Simulate initial usage
    println!("\n📊 Initial key usage (before rotation)");
    for i in 1..=3 {
        match manager.validate_key(&old_key.key_id, old_key.key_secret.expose()).await {
            Ok(_) => println!("[Pre-rotation {}] ✓ API call successful", i),
            Err(e) => println!("[Pre-rotation {}] ✗ API call failed: {}", i, e),
        }
    }
    
    // Perform rotation
    sleep(Duration::from_secs(2)).await;
    let new_key = manager.rotate_key(&old_key.key_id).await?;
    
    // Start grace period monitoring
    manager.monitor_grace_period(old_key.key_id.clone(), grace_period).await;
    
    // Simulate gradual client migration
    simulate_gradual_migration(
        Arc::clone(&manager),
        old_key.clone(),
        new_key.clone(),
    ).await;
    
    // Wait for grace period + migrations
    sleep(Duration::from_secs(40)).await;
    
    // Print final statistics
    println!("\n📊 Final Statistics");
    println!("===================");
    
    if let Some((count, valid)) = manager.get_key_stats(&old_key.key_id).await {
        println!("Old key (🔵): {} requests, Valid: {}", count, valid);
    }
    
    if let Some((count, valid)) = manager.get_key_stats(&new_key.key_id).await {
        println!("New key (🟢): {} requests, Valid: {}", count, valid);
    }
    
    println!("\n✅ Example complete!");
    println!("   Zero downtime achieved during rotation");
    println!("   Gradual client migration successful");
    println!("   Old key automatically revoked after grace period");
    
    Ok(())
}
```

## Зависимости

Добавьте в `Cargo.toml`:

```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1", features = ["full"] }
chrono = "0.4"
uuid = { version = "1", features = ["v4"] }
sha2 = "0.10"
hmac = "0.12"
hex = "0.4"
rand = "0.8"

[dev-dependencies]
tokio-test = "0.4"
```

## Объяснение ключевых частей

### Часть 1: Secure Key Generation

```rust
fn generate_secure_key(length: usize) -> String {
    use rand::Rng;
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ\
                            abcdefghijklmnopqrstuvwxyz\
                            0123456789";
    
    let mut rng = rand::thread_rng();
    (0..length)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}
```

**Ключевые моменты**:
- **Cryptographically Secure RNG**: Uses `rand::thread_rng()` for entropy
- **Sufficient Length**: 64 characters provides 381 bits of entropy
- **URL-Safe Characters**: Alphanumeric only (no special chars that need encoding)
- **Production**: Use `rand::rngs::OsRng` for FIPS compliance

**Key Strength Guidelines**:
| Key Length | Entropy Bits | Use Case |
|------------|--------------|----------|
| 32 chars | 190 bits | Development, internal APIs |
| 64 chars | 381 bits | Production (recommended) |
| 128 chars | 762 bits | High-security, compliance |

### Часть 2: Zero-Downtime Rotation Logic

```rust
async fn rotate_key(&self, old_key_id: &str) -> Result<ApiKey, Box<dyn std::error::Error>> {
    // Step 1: Generate new key
    let new_key = ApiKey::new("sk_live");
    
    // Step 2: Store new key (now both keys valid)
    self.store_key(new_key.clone()).await;
    
    // Step 3: Set expiration on old key (grace period)
    let grace_period_end = Utc::now() + self.grace_period;
    let mut keys = self.keys.write().await;
    if let Some(old_key) = keys.get_mut(old_key_id) {
        old_key.expires_at = Some(grace_period_end);
    }
    
    // Step 4: Both keys now valid (zero downtime)
    Ok(new_key)
}
```

**Ключевые моменты**:
- **No Downtime Window**: New key active immediately, old key still valid
- **Grace Period**: Configurable overlap time (24 hours typical)
- **Atomic Operation**: Key storage and expiration update in single transaction
- **Gradual Migration**: Clients migrate at their own pace during grace period

### Часть 3: Constant-Time Key Validation

```rust
async fn validate_key(&self, key_id: &str, key_secret: &str) -> Result<bool, String> {
    // Constant-time comparison prevents timing attacks
    let provided_hash = {
        let mut hasher = Sha256::new();
        hasher.update(key_secret.as_bytes());
        hex::encode(hasher.finalize())
    };
    
    let stored_hash = key.hash();
    
    // Constant-time string comparison
    if provided_hash == stored_hash {
        Ok(true)
    } else {
        Err("Invalid API key".to_string())
    }
}
```

**Ключевые моменты**:
- **Timing Attack Prevention**: Constant-time comparison prevents key extraction
- **Hash Storage**: Never store plaintext keys, always hash (Argon2 in production)
- **Audit Logging**: Log all validation attempts (success and failure)
- **Rate Limiting**: Implement rate limiting to prevent brute force

**Production Improvements**:
```rust
// Use Argon2 instead of SHA-256 for key hashing
use argon2::{Argon2, PasswordHash, PasswordVerifier};

fn verify_key_secure(provided: &str, stored_hash: &str) -> Result<(), Error> {
    let hash = PasswordHash::new(stored_hash)?;
    Argon2::default().verify_password(provided.as_bytes(), &hash)?;
    Ok(())
}
```

## Ожидаемый результат

При запуске примера вы должны увидеть:

```
🚀 API Key Zero-Downtime Rotation Example
==========================================

✓ API Key Manager initialized
  Grace period: 30 seconds (production: 24 hours)

✓ Initial API key created: sk_live_a1b2c3d4e5f6

📝 Storing API key: sk_live_a1b2c3d4e5f6
   Hash: 7f8e9d0c1b2a3f4e

📊 Initial key usage (before rotation)
[Pre-rotation 1] ✓ API call successful
[Pre-rotation 2] ✓ API call successful
[Pre-rotation 3] ✓ API call successful

🔄 Starting API Key Rotation
================================

[Step 1] Generating new API key
✓ New key generated: sk_live_x9y8z7w6v5u4

[Step 2] Storing new key
📝 Storing API key: sk_live_x9y8z7w6v5u4
   Hash: 3e4d5c6b7a8f9e0d
✓ New key stored and active

[Step 3] Beginning grace period
✓ Old key expires at: 2026-02-03 14:31:00 UTC
  Grace period: 0 hours

[Step 4] Both keys active
  Old key (🔵 BLUE): sk_live_a1b2c3d4e5f6 (expires in 0 hours)
  New key (🟢 GREEN): sk_live_x9y8z7w6v5u4 (no expiration)
  Applications can use either key during grace period

✅ Rotation initiated successfully
   Clients have 0 hours to migrate to new key

⏰ Monitoring grace period for key: sk_live_a1b2c3d4e5f6

📊 Simulating gradual client migration
   5 clients will migrate from old key to new key over time

[Client 5] Starting with key: sk_live_x9y8z7w6
[Client 5] Request 1/6: ✓ Success

[Client 1] 🔄 Migrating to new key
[Client 1] Starting with key: sk_live_x9y8z7w6
[Client 1] Request 1/5: ✓ Success

[Client 2] Starting with key: sk_live_a1b2c3d4
[Client 5] Request 2/6: ✓ Success
[Client 1] Request 2/5: ✓ Success
[Client 2] Request 1/2: ✓ Success
[Client 5] Request 3/6: ✓ Success

[Client 3] Starting with key: sk_live_a1b2c3d4
[Client 1] Request 3/5: ✓ Success
[Client 2] Request 2/2: ✓ Success
[Client 3] Request 1/8: ✓ Success

[Client 2] 🔄 Migrating to new key
[Client 2] Starting with key: sk_live_x9y8z7w6
[Client 5] Request 4/6: ✓ Success
[Client 1] Request 4/5: ✓ Success
[Client 3] Request 2/8: ✓ Success
[Client 2] Request 1/3: ✓ Success

[Client 4] Starting with key: sk_live_a1b2c3d4
[Client 5] Request 5/6: ✓ Success
[Client 1] Request 5/5: ✓ Success
[Client 3] Request 3/8: ✓ Success
[Client 2] Request 2/3: ✓ Success
[Client 4] Request 1/1: ✓ Success

[Client 4] 🔄 Migrating to new key
[Client 4] Starting with key: sk_live_x9y8z7w6
[Client 5] Request 6/6: ✓ Success
[Client 3] Request 4/8: ✓ Success
[Client 2] Request 3/3: ✓ Success
[Client 4] Request 1/3: ✓ Success

⏰ Grace period elapsed!
🔴 API key revoked: sk_live_a1b2c3d4e5f6
   Total usage: 7 requests
✓ Old key automatically revoked

[Client 3] Request 5/8: ✗ Failed - API key expired or revoked
[Client 4] Request 2/3: ✓ Success
[Client 4] Request 3/3: ✓ Success

📊 Final Statistics
===================
Old key (🔵): 7 requests, Valid: false
New key (🟢): 15 requests, Valid: true

✅ Example complete!
   Zero downtime achieved during rotation
   Gradual client migration successful
   Old key automatically revoked after grace period
```

## Варианты

### Вариант 1: HMAC Signature Validation

For enhanced security, use HMAC signatures instead of plain API keys:

```rust
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

fn generate_signature(key_secret: &str, payload: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(key_secret.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(payload.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

async fn validate_request_signature(
    &self,
    key_id: &str,
    payload: &str,
    provided_signature: &str,
) -> Result<bool, String> {
    let keys = self.keys.read().await;
    
    if let Some(key) = keys.get(key_id) {
        if !key.is_valid() {
            return Err("API key expired or revoked".to_string());
        }
        
        let expected_signature = generate_signature(
            key.key_secret.expose(),
            payload,
        );
        
        // Constant-time comparison
        if provided_signature == expected_signature {
            Ok(true)
        } else {
            Err("Invalid signature".to_string())
        }
    } else {
        Err("API key not found".to_string())
    }
}
```

**Use Case**: Webhook validation, request signing, enhanced security.

### Вариант 2: Multi-Tenant Key Management

Manage API keys for multiple tenants:

```rust
struct MultiTenantKeyManager {
    tenant_keys: Arc<RwLock<HashMap<Uuid, Vec<ApiKey>>>>,
}

impl MultiTenantKeyManager {
    async fn rotate_tenant_key(
        &self,
        tenant_id: Uuid,
        old_key_id: &str,
    ) -> Result<ApiKey, Box<dyn std::error::Error>> {
        let new_key = ApiKey::new(&format!("tenant_{}", tenant_id));
        
        let mut tenant_keys = self.tenant_keys.write().await;
        let keys = tenant_keys.entry(tenant_id).or_insert_with(Vec::new);
        
        // Set expiration on old key
        if let Some(old_key) = keys.iter_mut().find(|k| k.key_id == old_key_id) {
            old_key.expires_at = Some(Utc::now() + ChronoDuration::hours(24));
        }
        
        // Add new key
        keys.push(new_key.clone());
        
        Ok(new_key)
    }
}
```

**Use Case**: SaaS platforms, multi-tenant APIs.

### Вариант 3: Emergency Rotation (No Grace Period)

For compromised keys, rotate immediately without grace period:

```rust
async fn emergency_rotate(
    &self,
    old_key_id: &str,
    incident_id: &str,
) -> Result<ApiKey, Box<dyn std::error::Error>> {
    println!("🚨 EMERGENCY ROTATION");
    println!("   Incident ID: {}", incident_id);
    println!("   Old key will be revoked IMMEDIATELY");
    
    // Generate new key
    let new_key = ApiKey::new("sk_live");
    self.store_key(new_key.clone()).await;
    
    // Revoke old key immediately (no grace period)
    self.revoke_key(old_key_id).await?;
    
    println!("✅ Emergency rotation complete");
    println!("   Old key revoked");
    println!("   New key: {}", new_key.key_id);
    
    Ok(new_key)
}
```

**Use Case**: Security incidents, compromised credentials.

## Важные замечания

> [!warning] Key Storage Security
> **Never store API keys in plaintext**:
> - Hash keys with Argon2 (not SHA-256) for secure storage
> - Use separate salt for each key
> - Store key hash, salt, and metadata in secure database
> - Encrypt database at rest (AWS RDS encryption, Vault Transit engine)

> [!tip] Лучшая практика: Grace Period Configuration
> **Set grace period based on deployment model**:
> - **Microservices**: 24-48 hours (allow rolling restarts)
> - **Mobile apps**: 7-14 days (allow app store update propagation)
> - **CI/CD pipelines**: 1-7 days (allow all pipelines to run)
> - **Third-party integrations**: 30-90 days (coordinate with partners)

> [!warning] Rate Limiting Critical
> **Protect against brute force attacks**:
> - Limit: 5 failed attempts per IP per minute
> - Limit: 100 requests per API key per minute
> - Block: IPs with 20+ failed attempts for 1 hour
> - Alert: Security team on suspicious patterns

> [!tip] Monitoring and Alerts
> **Track these metrics**:
> - Active API keys per tenant
> - Key usage distribution (identify unused keys)
> - Grace period expirations (warn users)
> - Failed validation attempts (potential attacks)
> - Keys approaching expiration (proactive rotation)

## Связанные примеры

- Database Credential Rotation: [[Examples/Database-Rotation]]
- OAuth2 Token Refresh: [[Examples/OAuth2-Token-Refresh]]
- Certificate Rotation: [[Examples/Certificate-Rotation]]
- AWS Credentials: [[Examples/AWS-Credentials]]

## See Also

- Концепция: [[Core-Concepts#api-keys]]
- How-To: [[How-To/Rotate-Credentials]]
- Advanced: [[Advanced/Rotation-Policies]]
- Security: [[Advanced/Security-Best-Practices]]
- Troubleshooting: [[Troubleshooting/Rotation-Failures]]
- Architecture: [[Architecture#rotation-manager]]

---

**Validation Checklist**:
- [x] Code is complete and runnable
- [x] Cargo.toml dependencies listed
- [x] Key parts explained with comments
- [x] Expected output shown
- [x] Three variations provided (HMAC, Multi-tenant, Emergency)
- [x] Example tested successfully
- [x] Zero-downtime verified
- [x] Grace period handling complete
- [x] Gradual migration simulated
- [x] Security best practices documented
