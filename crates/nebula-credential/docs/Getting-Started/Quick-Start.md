---
title: "Quick Start: Your First Credential in 5 Minutes"
tags: [getting-started, api-key, beginner, quick-start]
status: published
lang: en
created: 2026-02-03
last_updated: 2026-02-03
audience: [beginner]
estimated_reading: 10
priority: P1
---

# Quick Start: Your First Credential in 5 Minutes

> **TL;DR**: Store and retrieve an API key credential with automatic encryption in under 5 minutes using nebula-credential's secure storage.

## Overview

This guide walks you through the absolute basics of `nebula-credential`: creating a credential manager, storing an API key securely, and retrieving it for use. You'll see how nebula-credential handles encryption, zeroization, and secure storage automatically.

By the end of this tutorial, you'll have a working credential management system that stores API keys with AES-256-GCM encryption and automatic memory cleanup.

**What you'll learn**:
- How to initialize a credential manager with local storage
- How to store an API key with automatic encryption
- How to retrieve and use credentials securely with `SecretString`

**Who this guide is for**:
- Rust developers new to nebula-credential
- Anyone needing secure credential storage in their application
- Developers looking for a quick proof-of-concept

## Prerequisites

- Installed Rust 1.75+ and Cargo
- Basic understanding of async Rust (tokio)
- 5 minutes of your time

## 5-Minute Quick Start

### Step 1: Add Dependencies

Add to your `Cargo.toml`:

```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1.0", features = ["full"] }
```

### Step 2: Create a Credential Manager

Create a new file `examples/quickstart.rs`:

```rust
use nebula_credential::{CredentialManager, ApiKeyCredential, SecretString};
use nebula_credential::storage::LocalStorage;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize credential manager with local encrypted storage
    let storage = LocalStorage::new("./credentials.db").await?;
    let manager = CredentialManager::new(storage);
    
    println!("✓ Credential manager initialized");
    
    Ok(())
}
```

**What's happening**:
- `LocalStorage` creates an encrypted SQLite database at `./credentials.db`
- `CredentialManager` provides the high-level API for credential operations
- All credentials stored will be automatically encrypted with AES-256-GCM

### Step 3: Store an API Key

Add credential storage to your example:

```rust
use nebula_credential::{CredentialManager, ApiKeyCredential, SecretString};
use nebula_credential::storage::LocalStorage;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize manager
    let storage = LocalStorage::new("./credentials.db").await?;
    let manager = CredentialManager::new(storage);
    
    // Create an API key credential
    let api_key = ApiKeyCredential::new(
        "my-api-service",                          // Service identifier
        SecretString::from("sk_live_abc123xyz"),   // API key (will be encrypted)
    );
    
    // Store the credential with a unique ID
    manager.store("github-api", api_key).await?;
    
    println!("✓ API key stored securely");
    
    Ok(())
}
```

**Expected output**:
```
✓ Credential manager initialized
✓ API key stored securely
```

### Step 4: Retrieve and Use the Credential

Now retrieve the credential and use it:

```rust
use nebula_credential::{CredentialManager, ApiKeyCredential, SecretString};
use nebula_credential::storage::LocalStorage;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize manager
    let storage = LocalStorage::new("./credentials.db").await?;
    let manager = CredentialManager::new(storage);
    
    // Store credential (only needed first time)
    let api_key = ApiKeyCredential::new(
        "my-api-service",
        SecretString::from("sk_live_abc123xyz"),
    );
    manager.store("github-api", api_key).await?;
    
    // Retrieve the credential
    let credential: ApiKeyCredential = manager
        .retrieve("github-api")
        .await?
        .ok_or("Credential not found")?;
    
    // Use the API key (automatically redacted in logs)
    println!("✓ Retrieved credential for: {}", credential.service_name());
    println!("✓ API key: {}", credential.key()); // Shows: "SecretString([REDACTED])"
    
    // Access raw value when needed (use with caution)
    credential.key().expose_secret(|key| {
        println!("Making API call with key: {}...", &key[..8]);
        // Make your API call here
    });
    
    println!("✓ Credential used successfully");
    
    Ok(())
}
```

**Expected output**:
```
✓ Retrieved credential for: my-api-service
✓ API key: SecretString([REDACTED])
Making API call with key: sk_live_...
✓ Credential used successfully
```

### Step 5: Run the Example

Execute your quickstart example:

```bash
cargo run --example quickstart
```

You should see all checkmarks indicating successful credential storage and retrieval. The `credentials.db` file now contains your encrypted API key.

## Full Working Example

```rust
// File: examples/quickstart.rs
// Description: Complete example showing credential storage and retrieval

use nebula_credential::{CredentialManager, ApiKeyCredential, SecretString};
use nebula_credential::storage::LocalStorage;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Nebula Credential Quick Start\n");
    
    // Step 1: Initialize credential manager with local encrypted storage
    let storage = LocalStorage::new("./credentials.db").await?;
    let manager = CredentialManager::new(storage);
    println!("✓ Credential manager initialized");
    
    // Step 2: Create and store an API key credential
    let api_key = ApiKeyCredential::new(
        "my-api-service",
        SecretString::from("sk_live_abc123xyz"),
    );
    manager.store("github-api", api_key).await?;
    println!("✓ API key stored securely (AES-256-GCM encrypted)");
    
    // Step 3: Retrieve the credential
    let credential: ApiKeyCredential = manager
        .retrieve("github-api")
        .await?
        .ok_or("Credential not found")?;
    println!("✓ Retrieved credential for: {}", credential.service_name());
    
    // Step 4: Use the credential (automatically redacted in logs)
    println!("✓ API key: {}", credential.key()); // Shows [REDACTED]
    
    // Step 5: Access raw value only when needed for API calls
    credential.key().expose_secret(|key| {
        println!("✓ Making API call with key: {}...", &key[..10]);
        // Your API call would go here
    });
    
    // Step 6: Credential is automatically zeroized when dropped
    drop(credential);
    println!("✓ Credential securely removed from memory\n");
    
    println!("🎉 Quick start complete! Your API key is stored encrypted on disk.");
    println!("   Run this program again to see retrieval from storage.");
    
    Ok(())
}
```

**Security features you get automatically**:
- ✅ AES-256-GCM encryption for stored credentials
- ✅ Argon2id key derivation (19 MiB memory cost)
- ✅ Automatic memory zeroization when credentials are dropped
- ✅ Redacted logging (secrets never appear in logs)
- ✅ Constant-time comparison to prevent timing attacks

## What's Next?

Now you know the basics of storing and retrieving credentials. Explore these topics:

- **Installation & Setup**: [[Installation]] - Comprehensive setup with all storage providers
- **Core Concepts**: [[Core-Concepts]] - Deep dive into credential types, lifecycle, and security model
- **API Keys in Production**: [[Examples/API-Key-Basic]] - Production-ready API key management patterns
- **Secure String Handling**: [[Examples/SecretString-Usage]] - Advanced SecretString usage and best practices
- **Storage Providers**: [[How-To/Store-Credentials]] - Using AWS Secrets Manager, HashiCorp Vault, etc.
- **Credential Rotation**: [[How-To/RotateCredentials]] - Implementing automatic credential rotation

## Frequently Asked Questions

**Q: Where is my API key actually stored?**

A: In a SQLite database at `./credentials.db`, encrypted with AES-256-GCM. The encryption key is derived from a master password using Argon2id with 19 MiB memory cost, making brute-force attacks computationally expensive.

**Q: Can I use this in production?**

A: Yes! For production, consider using a production-grade storage backend like [[Integrations/AWS-Secrets-Manager]], [[Integrations/HashiCorp-Vault]], or [[Integrations/Azure-Key-Vault]] instead of LocalStorage.

**Q: What happens if I forget to call `expose_secret()`?**

A: The `SecretString` type prevents accidental exposure. You must explicitly call `expose_secret()` to access the raw value, ensuring credentials don't leak into logs or error messages.

**Q: How do I delete a credential?**

A: Use `manager.delete("credential-id").await?` to permanently remove a credential from storage. The memory is automatically zeroized.

**Q: Can I store multiple types of credentials?**

A: Absolutely! Use `OAuth2Credential`, `DatabaseCredential`, `JWTToken`, `SAMLAssertion`, and more. See [[API-Reference]] for all credential types.

## Troubleshooting

**Problem: "Permission denied" when creating credentials.db**

Solution: Ensure your application has write permissions in the current directory. Try specifying an absolute path: `LocalStorage::new("/path/to/credentials.db")`.

**Problem: "Credential not found" when retrieving**

Solution: Make sure you're using the same credential ID for both store and retrieve. Check available credentials with `manager.list().await?`.

**Problem: Dependencies won't compile**

Solution: Verify Rust version with `rustc --version`. Nebula-credential requires Rust 1.75+. Update with `rustup update`.

## See Also

- **Core Concept**: [[Core-Concepts]] - Understand credential lifecycle and security model
- **How-To**: [[How-To/Store-Credentials]] - Store credentials with different providers
- **How-To**: [[How-To/Retrieve-Credentials]] - Retrieve with scopes and filters
- **Example**: [[Examples/API-Key-Basic]] - Production API key patterns
- **Example**: [[Examples/SecretString-Usage]] - Secure string handling
- **Reference**: [[API-Reference]] - Complete API documentation
- **Troubleshooting**: [[Troubleshooting/Common-Issues]] - Solutions to common problems

---

**Validation Checklist**:
- [x] TL;DR is one sentence
- [x] Can be completed in <10 minutes (5 minutes target)
- [x] Code examples are copy-paste runnable
- [x] No advanced concepts mentioned (scopes, rotation, events deferred)
- [x] "What's Next" section has 6 links
- [x] All wikilinks resolve to planned pages
- [x] Example includes expected output
- [x] Security features explained simply
- [x] FAQ answers common beginner questions
