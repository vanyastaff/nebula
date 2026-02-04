# Quickstart: Credential Manager

**Time to complete**: 3 minutes  
**Prerequisites**: Rust 1.92+, nebula-credential dependency

## Step 1: Basic Usage (30 seconds)

```rust
use nebula_credential::prelude::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Create storage provider
    let storage = LocalStorageProvider::new("./credentials.db").await?;
    
    // 2. Build manager
    let manager = CredentialManager::builder()
        .storage(Arc::new(storage))
        .build();
    
    // 3. Store credential
    let id = CredentialId::from("github-prod");
    let cred = Credential::api_key("ghp_abc123", "X-API-Key");
    manager.store(&id, &cred).await?;
    
    // 4. Retrieve credential
    let retrieved = manager.retrieve(&id).await?;
    println!("Retrieved: {:?}", retrieved);
    
    Ok(())
}
```

**Output**: `Retrieved: Some(ApiKeyCredential { ... })`

## Step 2: Enable Caching (1 minute)

```rust
use std::time::Duration;

// Build with cache (5-minute TTL, 1000 entry max)
let manager = CredentialManager::builder()
    .storage(Arc::new(storage))
    .cache_ttl(Duration::from_secs(300))
    .cache_max_size(1000)
    .build();

// First retrieval: cache miss (~50ms)
let cred1 = manager.retrieve(&id).await?;

// Second retrieval: cache hit (<5ms)
let cred2 = manager.retrieve(&id).await?;

// Check cache performance
if let Some(stats) = manager.cache_stats() {
    println!("Hit rate: {:.1}%", stats.hit_rate() * 100.0);
}
```

**Output**: `Hit rate: 50.0%` (1 hit, 1 miss)

## Step 3: Multi-Tenant Scopes (1 minute)

```rust
// Store credentials in different scopes
let tenant_a = ScopeId::new("org:acme/team:eng");
let tenant_b = ScopeId::new("org:acme/team:sales");

// Store in tenant A's scope
let cred_a = Credential::api_key("key-a", "Authorization");
manager.store_scoped(&id, &cred_a, &tenant_a).await?;

// Cannot retrieve from tenant B's scope
let result = manager.retrieve_scoped(&id, &tenant_b).await?;
assert!(result.is_none()); // Scope isolation enforced

// List all credentials in tenant A
let ids = manager.list_scoped(&tenant_a).await?;
println!("Tenant A has {} credentials", ids.len());
```

## Step 4: Batch Operations (30 seconds)

```rust
// Store 100 credentials in parallel
let credentials: Vec<_> = (0..100)
    .map(|i| {
        let id = CredentialId::from(format!("cred-{}", i));
        let cred = Credential::api_key(&format!("key-{}", i), "Authorization");
        (id, cred)
    })
    .collect();

let results = manager.store_batch(credentials).await;
let successful = results.iter().filter(|r| r.is_ok()).count();
println!("Stored {} credentials", successful);
```

**Performance**: ~1 second (10 concurrent) vs ~10 seconds (sequential)

## Next Steps

- **Production Setup**: See [How-To/Store-Credentials](../../../crates/nebula-credential/docs/How-To/Store-Credentials.md)
- **Cloud Storage**: Configure AWS/Vault/K8s providers
- **Validation**: Use `validate()` and `validate_batch()` for expiration checks
- **Error Handling**: Pattern match on `ManagerError` for precise handling

## Common Patterns

### With Cloud Storage
```rust
#[cfg(feature = "storage-aws")]
let storage = AwsSecretsManagerProvider::new("us-east-1").await?;

let manager = CredentialManager::builder()
    .storage(Arc::new(storage))
    .cache_ttl(Duration::from_secs(600)) // 10 min cache
    .build();
```

### Validation Workflow
```rust
// Validate before use
let result = manager.validate(&id).await?;
if !result.valid {
    match result.details {
        ValidationDetails::Expired { .. } => {
            // Credential expired, rotate or refresh
        }
        _ => {
            // Other validation failure
        }
    }
}
```

### Error Handling
```rust
match manager.retrieve(&id).await {
    Ok(Some(cred)) => { /* Use credential */ }
    Ok(None) => { /* Not found */ }
    Err(ManagerError::StorageError { source, .. }) => {
        // Storage provider failure, retry or fallback
    }
    Err(e) => { /* Other error */ }
}
```

That's it! You now have a fully functional credential manager with caching, multi-tenant isolation, and batch operations.
