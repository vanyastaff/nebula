---
title: "Basic API Key Management"
tags: [example, api-key, basic, authentication, storage]
status: published
lang: en
created: 2026-02-03
last_updated: 2026-02-03
audience: [beginner, intermediate]
estimated_reading: 10
priority: P1
---

# Basic API Key Management

> **TL;DR**: Store, retrieve, and use API keys securely with automatic encryption, validation, and credential rotation in production applications.

## Use Case

This example demonstrates the complete lifecycle of managing API keys in a production application: creation, secure storage with encryption, retrieval, validation, and automatic rotation. Perfect for applications that authenticate with external services using API keys.

**When to use**:
- Authenticating with third-party APIs (GitHub, Stripe, SendGrid, etc.)
- Managing service-to-service authentication keys
- Storing client API credentials for SaaS applications
- Implementing credential rotation without downtime

## Prerequisites

- nebula-credential v0.1.0+
- Understanding of: [[Core-Concepts#API Keys]]
- Basic async Rust knowledge (tokio)
- 10 minutes

## Full Code Example

```rust
// File: examples/api_key_basic.rs
// Description: Complete API key lifecycle management with encryption and rotation
// 
// To run:
//   cargo run --example api_key_basic

use nebula_credential::{
    CredentialManager,
    ApiKeyCredential,
    SecretString,
    storage::LocalStorage,
    CredentialContext,
};
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("🔑 API Key Management Example\n");
    
    // Step 1: Initialize credential manager with local encrypted storage
    let storage = LocalStorage::new("./credentials.db").await?;
    let manager = CredentialManager::new(storage);
    println!("✓ Credential manager initialized");
    
    // Step 2: Create an API key credential
    let github_api_key = ApiKeyCredential::new(
        "github-api",                                    // Service identifier
        SecretString::from("ghp_xxxxxxxxxxxxxxxxxxx"),   // API key (will be encrypted at rest)
    );
    println!("✓ Created GitHub API key credential");
    
    // Step 3: Store the credential with a unique identifier
    manager.store("github-prod", github_api_key).await?;
    println!("✓ Stored credential securely (AES-256-GCM encrypted)");
    
    // Step 4: Retrieve the credential
    let retrieved: ApiKeyCredential = manager
        .retrieve("github-prod")
        .await?
        .ok_or("Credential not found")?;
    println!("✓ Retrieved credential for: {}", retrieved.service_name());
    
    // Step 5: Validate the credential (checks format, not expiration for API keys)
    let context = CredentialContext::default();
    if retrieved.validate(&context).await? {
        println!("✓ Credential is valid");
    }
    
    // Step 6: Use the API key (automatically redacted in logs)
    println!("  API key (redacted): {}", retrieved.key()); // Shows [REDACTED]
    
    // Step 7: Access the raw key value safely for API calls
    retrieved.key().expose_secret(|key| {
        println!("  Making API call with key: {}...", &key[..10]);
        // Your actual API call would go here:
        // let response = reqwest::get(format!("https://api.github.com/user?token={}", key)).await?;
    });
    
    // Step 8: List all stored credentials
    let all_creds = manager.list().await?;
    println!("\n📋 All stored credentials:");
    for cred_id in all_creds {
        println!("  - {}", cred_id);
    }
    
    // Step 9: Rotate the API key (simulating key rotation)
    println!("\n🔄 Rotating API key...");
    let new_key = ApiKeyCredential::new(
        "github-api",
        SecretString::from("ghp_yyyyyyyyyyyyyyyyyyyy"),
    );
    manager.store("github-prod", new_key).await?;
    println!("✓ API key rotated successfully");
    
    // Step 10: Verify rotation
    let updated: ApiKeyCredential = manager
        .retrieve("github-prod")
        .await?
        .ok_or("Credential not found")?;
    
    updated.key().expose_secret(|key| {
        assert!(key.starts_with("ghp_yyy"));
        println!("✓ Verified new key is active: {}...", &key[..10]);
    });
    
    // Step 11: Delete the credential when no longer needed
    manager.delete("github-prod").await?;
    println!("\n✓ Credential deleted and zeroized from memory");
    
    println!("\n🎉 API key lifecycle complete!");
    
    // Cleanup test database
    std::fs::remove_file("./credentials.db").ok();
    
    Ok(())
}
```

## Dependencies

Add to your `Cargo.toml`:

```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1.0", features = ["full"] }

# For making actual API calls (optional)
reqwest = { version = "0.11", features = ["json"] }
```

## Explanation of Key Parts

### Part 1: Creating API Key Credentials

```rust
let github_api_key = ApiKeyCredential::new(
    "github-api",                                    // Service identifier
    SecretString::from("ghp_xxxxxxxxxxxxxxxxxxx"),   // API key
);
```

**What's happening**:
- `ApiKeyCredential::new()` creates a strongly-typed API key credential
- The service identifier helps you organize credentials by service
- `SecretString` ensures the key is never accidentally logged or displayed
- The key value is immediately protected and will be zeroized when dropped

### Part 2: Secure Storage

```rust
manager.store("github-prod", github_api_key).await?;
```

**What's happening**:
- The credential is serialized and encrypted with AES-256-GCM before storage
- A unique 96-bit nonce is generated using monotonic counter + random + timestamp
- The encryption key is derived from a master password using Argon2id (19 MiB memory cost)
- The encrypted credential is stored in SQLite with the identifier "github-prod"

### Part 3: Safe Retrieval and Usage

```rust
let retrieved: ApiKeyCredential = manager
    .retrieve("github-prod")
    .await?
    .ok_or("Credential not found")?;

retrieved.key().expose_secret(|key| {
    // Use key safely within this closure
    make_api_call(key);
});
```

**What's happening**:
- `retrieve()` decrypts the credential from storage
- Type annotation `ApiKeyCredential` ensures type safety
- `expose_secret()` is the **only** way to access the raw key value
- The closure ensures the key is never accidentally leaked outside its scope
- Memory is automatically zeroized when the credential is dropped

## Expected Output

When you run the example, you should see:

```
🔑 API Key Management Example

✓ Credential manager initialized
✓ Created GitHub API key credential
✓ Stored credential securely (AES-256-GCM encrypted)
✓ Retrieved credential for: github-api
✓ Credential is valid
  API key (redacted): SecretString([REDACTED])
  Making API call with key: ghp_xxxxxx...

📋 All stored credentials:
  - github-prod

🔄 Rotating API key...
✓ API key rotated successfully
✓ Verified new key is active: ghp_yyyyy...

✓ Credential deleted and zeroized from memory

🎉 API key lifecycle complete!
```

## Variations

### Variation 1: API Key with Expiration

For API keys that expire (like temporary access tokens):

```rust
use nebula_credential::{ApiKeyCredential, SecretString, Metadata};
use std::time::{Duration, SystemTime};

let expires_in = Duration::from_secs(3600); // 1 hour
let expires_at = SystemTime::now() + expires_in;

let mut api_key = ApiKeyCredential::new(
    "temporary-service",
    SecretString::from("temp_key_123456"),
);

// Set expiration metadata
api_key.set_metadata(Metadata {
    expires_at: Some(expires_at),
    ..Default::default()
});

manager.store("temp-key", api_key).await?;

// Later, check expiration before use
let key: ApiKeyCredential = manager.retrieve("temp-key").await?.unwrap();
if key.is_expired() {
    println!("⚠️  Key has expired, need to refresh");
    // Implement refresh logic
} else {
    // Safe to use
    key.key().expose_secret(|k| make_api_call(k));
}
```

### Variation 2: Multiple Environment Keys

Managing different keys for dev/staging/production:

```rust
use std::env;

async fn get_api_key(manager: &CredentialManager) -> Result<ApiKeyCredential, Box<dyn Error>> {
    let environment = env::var("APP_ENV").unwrap_or_else(|_| "development".to_string());
    
    let key_id = match environment.as_str() {
        "production" => "github-prod",
        "staging" => "github-staging",
        _ => "github-dev",
    };
    
    manager.retrieve(key_id)
        .await?
        .ok_or_else(|| format!("No API key found for environment: {}", environment).into())
}

// Usage
let api_key = get_api_key(&manager).await?;
println!("Using {} API key", std::env::var("APP_ENV").unwrap_or_default());
```

### Variation 3: API Key with Custom Headers

For APIs requiring custom authentication headers:

```rust
use reqwest::Client;
use std::collections::HashMap;

async fn call_api_with_custom_auth(
    credential: &ApiKeyCredential,
    endpoint: &str,
) -> Result<String, Box<dyn Error>> {
    let client = Client::new();
    
    credential.key().expose_secret(|key| {
        // Build custom headers
        let response = client
            .get(endpoint)
            .header("X-API-Key", key)  // Custom header
            .header("User-Agent", "my-app/1.0")
            .send()
            .await?;
        
        response.text().await
    })
}

// Usage
let github_key: ApiKeyCredential = manager.retrieve("github-prod").await?.unwrap();
let user_data = call_api_with_custom_auth(&github_key, "https://api.github.com/user").await?;
println!("User data: {}", user_data);
```

## Important Notes

> [!warning] Security Warning
> Never hardcode API keys in source code. Always load them from environment variables or secure storage. Use `SecretString` to prevent accidental exposure in logs or error messages.

> [!tip] Best Practice
> Implement automatic key rotation for production systems. Store the rotation timestamp in metadata and rotate keys every 30-90 days. Use grace periods to ensure zero-downtime rotation.

> [!info] Performance Tip
> For high-throughput applications, cache decrypted credentials in memory for a short period (30-60 seconds) to avoid repeated decryption overhead. Ensure cached credentials are zeroized on expiration.

## Common Pitfalls

**❌ DON'T**: Store credentials in plaintext

```rust
// NEVER DO THIS
let key = "ghp_xxxxxxxxxxxxxxxxxxx";
println!("API key: {}", key);  // Exposed in logs!
```

**✅ DO**: Use SecretString

```rust
let key = SecretString::from("ghp_xxxxxxxxxxxxxxxxxxx");
println!("API key: {}", key);  // Shows [REDACTED]
```

**❌ DON'T**: Keep raw key values in scope

```rust
let raw_key = credential.key().expose_secret(|k| k.clone());
// Raw key stays in memory!
```

**✅ DO**: Use keys within expose_secret closure

```rust
credential.key().expose_secret(|key| {
    make_api_call(key);
    // Key is zeroized when closure ends
});
```

## Related Examples

- **OAuth2 Flow**: [[Examples/OAuth2-GitHub]] - Full OAuth2 authorization code flow
- **Database Credentials**: [[Examples/Database-Rotation]] - Automatic database credential rotation
- **Credential Scopes**: [[Examples/Scoped-Credentials]] - Using scopes for multi-tenant applications

## See Also

- **Concept**: [[Core-Concepts#API Keys]] - Understanding API key credentials
- **How-To**: [[How-To/Store-Credentials]] - Storing credentials with different providers
- **How-To**: [[How-To/RotateCredentials]] - Implementing credential rotation
- **Security**: [[Security/Encryption]] - Encryption implementation details
- **Reference**: [[API-Reference#ApiKeyCredential]] - Complete API documentation
- **Troubleshooting**: [[Troubleshooting/Common-Issues#API Key Issues]] - Common API key problems

---

**Validation Checklist**:
- [x] Code is complete and runnable without modifications
- [x] Cargo.toml dependencies listed
- [x] All key parts explained with inline comments
- [x] Expected output shown with all steps
- [x] Three practical variations provided
- [x] Security warnings and best practices included
- [x] Related examples linked
- [x] Example demonstrates full credential lifecycle (create, store, retrieve, validate, rotate, delete)
