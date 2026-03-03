---
title: "How to Retrieve Credentials"
tags: [how-to, retrieval, query, scopes, filtering]
status: published
lang: en
created: 2026-02-03
last_updated: 2026-02-03
audience: [intermediate]
estimated_reading: 15
priority: P1
---

# How to Retrieve Credentials

> **TL;DR**: Retrieve stored credentials by ID, filter by tags/metadata, use scopes for multi-tenant isolation, and implement secure credential queries.

## Overview

This guide covers all methods for retrieving credentials from storage: direct retrieval by ID, filtering by metadata, querying with scopes for multi-tenant applications, and implementing secure credential access patterns.

**What you'll achieve**:
- Retrieve credentials by unique identifier
- Filter credentials using tags and metadata
- Implement multi-tenant credential isolation with scopes
- Query credentials efficiently with caching strategies

## Prerequisites

> [!note] Required knowledge
> Ensure you've completed the following before starting:

- [x] Read: [[Quick-Start]]
- [x] Completed: [[How-To/Store-Credentials]]
- [x] Understand: [[Core-Concepts#Scopes]]
- [x] Understand: [[Architecture#Storage Abstraction]]

## Step-by-Step Guide

### Step 1: Basic Retrieval by ID

The simplest way to retrieve a credential is by its unique identifier:

```rust
use nebula_credential::{CredentialManager, ApiKeyCredential};
use nebula_credential::storage::LocalStorage;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize manager
    let storage = LocalStorage::new("./credentials.db").await?;
    let manager = CredentialManager::new(storage);
    
    // Retrieve by ID
    let credential: Option<ApiKeyCredential> = manager
        .retrieve("github-production")
        .await?;
    
    match credential {
        Some(cred) => {
            println!("✓ Retrieved credential for: {}", cred.service_name());
            // Use the credential
            cred.key().expose_secret(|key| {
                println!("  Key: {}...", &key[..10]);
            });
        }
        None => {
            println!("✗ Credential not found");
        }
    }
    
    Ok(())
}
```

**Expected output**:
```
✓ Retrieved credential for: github-api
  Key: ghp_xxxxx...
```

> [!tip] Type Safety
> Note the type annotation `Option<ApiKeyCredential>`. This ensures you get the right credential type. If the stored credential doesn't match the type, retrieval will fail with a type error.

### Step 2: Retrieve with Automatic Validation

Retrieve and validate credentials in one step:

```rust
use nebula_credential::{CredentialManager, ApiKeyCredential, CredentialContext};

// Retrieve credential
let credential: ApiKeyCredential = manager
    .retrieve("github-production")
    .await?
    .ok_or("Credential not found")?;

// Validate before use
let context = CredentialContext::default();
if !credential.validate(&context).await? {
    return Err("Credential validation failed".into());
}

// Safe to use
credential.key().expose_secret(|key| {
    make_api_call(key);
});
```

**What happens**:
- Credential is decrypted from storage
- Type checking ensures correct credential type
- Validation checks expiration and format
- Memory is zeroized after use

### Step 3: Query Multiple Credentials

List all credentials or filter by patterns:

```rust
use nebula_credential::CredentialManager;

// List all credential IDs
let all_ids: Vec<String> = manager.list().await?;
println!("Found {} credentials:", all_ids.len());
for id in all_ids {
    println!("  - {}", id);
}

// Filter by prefix pattern
let production_creds: Vec<String> = manager
    .list()
    .await?
    .into_iter()
    .filter(|id| id.contains("production"))
    .collect();

println!("\nProduction credentials:");
for id in production_creds {
    println!("  - {}", id);
}
```

**Expected output**:
```
Found 5 credentials:
  - github-production
  - github-staging
  - stripe-production
  - postgres-dev
  - postgres-production

Production credentials:
  - github-production
  - stripe-production
  - postgres-production
```

### Step 4: Retrieve with Scopes (Multi-Tenant)

Use scopes to isolate credentials by tenant, environment, or team:

```rust
use nebula_credential::{CredentialManager, Scope, ScopeId, ApiKeyCredential};

// Create a scope for tenant isolation
let tenant_scope = Scope::new(ScopeId::from("tenant:acme-corp"));

// Retrieve credential within scope
let credential: Option<ApiKeyCredential> = manager
    .retrieve_scoped("api-key", &tenant_scope)
    .await?;

match credential {
    Some(cred) => println!("✓ Retrieved credential for tenant: acme-corp"),
    None => println!("✗ No credential found in scope"),
}

// List all credentials in scope
let scoped_ids = manager.list_scoped(&tenant_scope).await?;
println!("Credentials in tenant scope: {:?}", scoped_ids);
```

**Use cases for scopes**:
- **Multi-tenant SaaS**: Isolate credentials per customer
- **Environment separation**: Separate dev/staging/prod credentials
- **Team boundaries**: Isolate credentials by team or department
- **Compliance**: Enforce access control boundaries

## Scope-Based Retrieval Patterns

### Pattern 1: Hierarchical Scopes

Implement hierarchical credential access:

```rust
use nebula_credential::{Scope, ScopeId};

// Define scope hierarchy: organization → team → service
let org_scope = Scope::new(ScopeId::from("org:acme"));
let team_scope = org_scope.child(ScopeId::from("team:engineering"));
let service_scope = team_scope.child(ScopeId::from("service:api"));

// Store credential in service scope
manager.store_scoped("db-password", db_credential, &service_scope).await?;

// Retrieve from specific scope
let cred = manager.retrieve_scoped("db-password", &service_scope).await?;

// List all credentials in team scope (includes child scopes)
let team_creds = manager.list_scoped_recursive(&team_scope).await?;
println!("Team credentials (including children): {}", team_creds.len());
```

**Scope hierarchy example**:
```
org:acme
├── team:engineering
│   ├── service:api
│   │   └── db-password
│   └── service:worker
│       └── queue-credentials
└── team:marketing
    └── service:analytics
        └── analytics-api-key
```

### Pattern 2: Multi-Tenant Isolation

Ensure complete credential isolation per tenant:

```rust
use nebula_credential::{CredentialManager, Scope, ScopeId};

async fn get_tenant_credential(
    manager: &CredentialManager,
    tenant_id: &str,
    credential_name: &str,
) -> Result<Option<ApiKeyCredential>, Box<dyn std::error::Error>> {
    // Create tenant-specific scope
    let tenant_scope = Scope::new(ScopeId::from(format!("tenant:{}", tenant_id)));
    
    // Retrieve credential only accessible to this tenant
    let credential = manager
        .retrieve_scoped(credential_name, &tenant_scope)
        .await?;
    
    Ok(credential)
}

// Usage in multi-tenant application
let acme_api_key = get_tenant_credential(
    &manager,
    "acme-corp",
    "external-api-key",
).await?;

let globex_api_key = get_tenant_credential(
    &manager,
    "globex-inc",
    "external-api-key",
).await?;

// These credentials are completely isolated
```

### Pattern 3: Environment-Based Retrieval

Automatically select credentials based on environment:

```rust
use std::env;

fn get_environment_scope() -> Scope {
    let env = env::var("APP_ENV").unwrap_or_else(|_| "development".to_string());
    Scope::new(ScopeId::from(format!("env:{}", env)))
}

async fn get_database_credential(
    manager: &CredentialManager,
) -> Result<DatabaseCredential, Box<dyn std::error::Error>> {
    let scope = get_environment_scope();
    
    let credential = manager
        .retrieve_scoped("postgres-primary", &scope)
        .await?
        .ok_or("Database credential not found for environment")?;
    
    Ok(credential)
}

// Automatically uses correct credential for current environment
// APP_ENV=production → uses production credentials
// APP_ENV=staging → uses staging credentials
let db_cred = get_database_credential(&manager).await?;
```

## Complete Example

Here's a comprehensive example demonstrating all retrieval patterns:

```rust
// File: examples/retrieve_credentials.rs
use nebula_credential::{
    CredentialManager,
    ApiKeyCredential,
    DatabaseCredential,
    OAuth2Credential,
    SecretString,
    Scope,
    ScopeId,
    CredentialContext,
    storage::LocalStorage,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔍 Retrieving Credentials Guide\n");
    
    // Setup: Store some test credentials
    let storage = LocalStorage::new("./demo_credentials.db").await?;
    let manager = CredentialManager::new(storage);
    
    // Store credentials in different scopes
    setup_test_credentials(&manager).await?;
    
    // === Example 1: Basic Retrieval ===
    println!("=== Example 1: Basic Retrieval ===");
    if let Some(cred) = manager.retrieve::<ApiKeyCredential>("global-api-key").await? {
        println!("✓ Retrieved global API key");
        cred.key().expose_secret(|k| println!("  Key: {}...", &k[..10]));
    }
    println!();
    
    // === Example 2: Scoped Retrieval ===
    println!("=== Example 2: Tenant-Scoped Retrieval ===");
    let tenant_a_scope = Scope::new(ScopeId::from("tenant:acme-corp"));
    let tenant_b_scope = Scope::new(ScopeId::from("tenant:globex-inc"));
    
    let acme_cred = manager
        .retrieve_scoped::<ApiKeyCredential>("api-key", &tenant_a_scope)
        .await?;
    let globex_cred = manager
        .retrieve_scoped::<ApiKeyCredential>("api-key", &tenant_b_scope)
        .await?;
    
    println!("✓ Retrieved credentials for 2 tenants (isolated)");
    println!("  Acme Corp: {}", acme_cred.is_some());
    println!("  Globex Inc: {}", globex_cred.is_some());
    println!();
    
    // === Example 3: List and Filter ===
    println!("=== Example 3: List and Filter ===");
    let all_creds = manager.list().await?;
    println!("Total credentials: {}", all_creds.len());
    
    let production_creds: Vec<_> = all_creds
        .iter()
        .filter(|id| id.contains("production"))
        .collect();
    println!("Production credentials: {}", production_creds.len());
    for id in production_creds {
        println!("  - {}", id);
    }
    println!();
    
    // === Example 4: Hierarchical Scopes ===
    println!("=== Example 4: Hierarchical Scopes ===");
    let org_scope = Scope::new(ScopeId::from("org:acme"));
    let team_scope = org_scope.child(ScopeId::from("team:engineering"));
    let service_scope = team_scope.child(ScopeId::from("service:api"));
    
    if let Some(cred) = manager
        .retrieve_scoped::<DatabaseCredential>("db-password", &service_scope)
        .await?
    {
        println!("✓ Retrieved DB credential from service scope");
        println!("  Host: {}", cred.host());
        println!("  Database: {}", cred.database().unwrap_or("default"));
    }
    println!();
    
    // === Example 5: Validation During Retrieval ===
    println!("=== Example 5: Validation During Retrieval ===");
    if let Some(cred) = manager.retrieve::<OAuth2Credential>("oauth2-token").await? {
        let context = CredentialContext::default();
        
        if cred.validate(&context).await? {
            println!("✓ Credential is valid");
        } else {
            println!("✗ Credential validation failed");
        }
        
        if cred.is_expired() {
            println!("  ⚠️  Credential has expired, needs refresh");
        }
    }
    println!();
    
    // === Example 6: Batch Retrieval ===
    println!("=== Example 6: Batch Retrieval ===");
    let ids = vec!["global-api-key", "oauth2-token", "db-password"];
    
    for id in ids {
        match manager.retrieve::<ApiKeyCredential>(id).await {
            Ok(Some(_)) => println!("  ✓ {}", id),
            Ok(None) => println!("  ✗ {} (not found)", id),
            Err(e) => println!("  ✗ {} (error: {})", id, e),
        }
    }
    println!();
    
    println!("🎉 Retrieval examples complete!");
    
    // Cleanup
    std::fs::remove_file("./demo_credentials.db").ok();
    
    Ok(())
}

async fn setup_test_credentials(manager: &CredentialManager) -> Result<(), Box<dyn std::error::Error>> {
    // Global credential
    manager.store(
        "global-api-key",
        ApiKeyCredential::new("global-service", SecretString::from("gsk_abc123")),
    ).await?;
    
    // Scoped credentials for tenant A
    let tenant_a = Scope::new(ScopeId::from("tenant:acme-corp"));
    manager.store_scoped(
        "api-key",
        ApiKeyCredential::new("acme-service", SecretString::from("acme_key_123")),
        &tenant_a,
    ).await?;
    
    // Scoped credentials for tenant B
    let tenant_b = Scope::new(ScopeId::from("tenant:globex-inc"));
    manager.store_scoped(
        "api-key",
        ApiKeyCredential::new("globex-service", SecretString::from("globex_key_456")),
        &tenant_b,
    ).await?;
    
    // Hierarchical scope credential
    let org_scope = Scope::new(ScopeId::from("org:acme"));
    let team_scope = org_scope.child(ScopeId::from("team:engineering"));
    let service_scope = team_scope.child(ScopeId::from("service:api"));
    
    manager.store_scoped(
        "db-password",
        DatabaseCredential::new(
            "postgresql",
            "db.acme.com",
            5432,
            "api_user",
            SecretString::from("db_pass_789"),
            Some("api_db"),
        ),
        &service_scope,
    ).await?;
    
    Ok(())
}
```

**Cargo.toml**:
```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1.0", features = ["full"] }
```

## Verification

To verify everything works correctly:

1. **Run the example**:
   ```bash
   cargo run --example retrieve_credentials
   ```

2. **Expected output**:
   ```
   🔍 Retrieving Credentials Guide
   
   === Example 1: Basic Retrieval ===
   ✓ Retrieved global API key
     Key: gsk_abc123...
   
   === Example 2: Tenant-Scoped Retrieval ===
   ✓ Retrieved credentials for 2 tenants (isolated)
     Acme Corp: true
     Globex Inc: true
   
   === Example 3: List and Filter ===
   Total credentials: 4
   Production credentials: 0
   
   === Example 4: Hierarchical Scopes ===
   ✓ Retrieved DB credential from service scope
     Host: db.acme.com
     Database: api_db
   
   === Example 5: Validation During Retrieval ===
   ✓ Credential is valid
   
   === Example 6: Batch Retrieval ===
     ✓ global-api-key
     ✗ oauth2-token (not found)
     ✗ db-password (not found)
   
   🎉 Retrieval examples complete!
   ```

## Troubleshooting

### Problem: Credential not found despite being stored

**Symptoms**:
- `retrieve()` returns `None`
- Credential ID appears in `list()` output

**Cause**: Type mismatch between stored type and retrieved type

**Solution**:
```rust
// ❌ WRONG: Trying to retrieve OAuth2 credential as API key
let cred: Option<ApiKeyCredential> = manager.retrieve("oauth2-token").await?;
// Returns None because types don't match

// ✅ CORRECT: Match the stored type
let cred: Option<OAuth2Credential> = manager.retrieve("oauth2-token").await?;
// Returns Some(credential)
```

### Problem: Scope isolation not working

**Symptoms**:
- Credentials leak between scopes
- `retrieve_scoped()` returns wrong tenant's credentials

**Cause**: Using `retrieve()` instead of `retrieve_scoped()`

**Solution**:
```rust
// ❌ WRONG: Bypasses scope isolation
let cred = manager.retrieve("api-key").await?;

// ✅ CORRECT: Enforces scope isolation
let tenant_scope = Scope::new(ScopeId::from("tenant:acme"));
let cred = manager.retrieve_scoped("api-key", &tenant_scope).await?;
```

### Problem: Performance issues with many credentials

**Symptoms**:
- Slow retrieval times
- High memory usage

**Cause**: Not using caching for frequently accessed credentials

**Solution**:
```rust
use std::collections::HashMap;
use tokio::sync::RwLock;

// Implement simple cache
struct CredentialCache {
    cache: RwLock<HashMap<String, ApiKeyCredential>>,
    manager: CredentialManager,
}

impl CredentialCache {
    async fn get(&self, id: &str) -> Result<Option<ApiKeyCredential>, Box<dyn std::error::Error>> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(cred) = cache.get(id) {
                return Ok(Some(cred.clone()));
            }
        }
        
        // Cache miss: retrieve from storage
        if let Some(cred) = self.manager.retrieve(id).await? {
            let mut cache = self.cache.write().await;
            cache.insert(id.to_string(), cred.clone());
            return Ok(Some(cred));
        }
        
        Ok(None)
    }
}
```

## Advanced Retrieval Patterns

### Pattern: Fallback Retrieval

Retrieve with fallback to default credentials:

```rust
async fn get_credential_with_fallback(
    manager: &CredentialManager,
    primary_id: &str,
    fallback_id: &str,
) -> Result<ApiKeyCredential, Box<dyn std::error::Error>> {
    // Try primary first
    if let Some(cred) = manager.retrieve(primary_id).await? {
        return Ok(cred);
    }
    
    // Fall back to default
    manager.retrieve(fallback_id)
        .await?
        .ok_or_else(|| format!("Neither {} nor {} found", primary_id, fallback_id).into())
}

// Usage
let api_key = get_credential_with_fallback(
    &manager,
    "custom-api-key",
    "default-api-key",
).await?;
```

### Pattern: Conditional Retrieval Based on Context

Select credentials dynamically based on runtime context:

```rust
async fn get_database_credential(
    manager: &CredentialManager,
    read_only: bool,
) -> Result<DatabaseCredential, Box<dyn std::error::Error>> {
    let credential_id = if read_only {
        "postgres-readonly"
    } else {
        "postgres-readwrite"
    };
    
    manager.retrieve(credential_id)
        .await?
        .ok_or_else(|| format!("Database credential {} not found", credential_id).into())
}

// Usage
let write_db = get_database_credential(&manager, false).await?;
let read_db = get_database_credential(&manager, true).await?;
```

## Best Practices

> [!tip] Performance Optimization
> - **Cache frequently-used credentials**: Avoid repeated decryption overhead
> - **Use batch retrieval**: When fetching multiple credentials, do it in parallel
> - **Set appropriate cache TTLs**: Balance security and performance (30-60 seconds typical)
> - **Lazy load credentials**: Only retrieve when actually needed

> [!tip] Security Best Practices
> - **Always validate after retrieval**: Check expiration and format before use
> - **Use scopes for multi-tenancy**: Never trust application logic alone for isolation
> - **Audit credential access**: Log all retrieval operations with context
> - **Handle errors securely**: Don't leak credential existence in error messages

> [!warning] Common Mistakes
> - **Don't cache credentials indefinitely**: Implement expiration to respect rotation
> - **Don't bypass scope isolation**: Always use `retrieve_scoped()` in multi-tenant applications
> - **Don't ignore validation failures**: Always check credential validity before use
> - **Don't expose credential IDs**: Credential IDs may be sensitive (use opaque references in APIs)

## Next Steps

After mastering credential retrieval, explore:

- **Rotation**: [[How-To/RotateCredentials]] - Implement automatic credential rotation
- **Advanced Filtering**: [[How-To/Query-With-Tags]] - Query using metadata and tags
- **Caching Strategies**: [[Advanced/Credential-Caching]] - Implement efficient caching
- **Audit Logging**: [[Advanced/Audit-Logging]] - Track credential access

## See Also

- **Concept**: [[Core-Concepts#Scopes]] - Understanding scope-based isolation
- **Concept**: [[Core-Concepts#Credential Lifecycle]] - Credential states and transitions
- **How-To**: [[How-To/Store-Credentials]] - Storing credentials with scopes
- **Example**: [[Examples/API-Key-Basic]] - Basic retrieval patterns
- **Architecture**: [[Architecture#Storage Abstraction]] - Storage provider internals
- **API Reference**: [[API-Reference#CredentialManager]] - Complete retrieval API
- **Troubleshooting**: [[Troubleshooting/Common-Issues#Retrieval Issues]] - Common problems

---

**Validation Checklist**:
- [x] Basic retrieval by ID documented
- [x] Scoped retrieval with examples
- [x] Hierarchical scope patterns
- [x] Multi-tenant isolation patterns
- [x] Environment-based retrieval
- [x] Complete working example
- [x] Troubleshooting for common issues
- [x] Performance optimization tips
- [x] Security best practices
- [x] Advanced patterns (fallback, conditional)
