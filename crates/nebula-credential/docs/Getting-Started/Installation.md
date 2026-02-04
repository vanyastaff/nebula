---
title: Installation
tags: [getting-started, installation, setup, cargo, dependencies]
status: published
lang: en
created: 2026-02-03
last_updated: 2026-02-03
audience: [beginner]
estimated_reading: 8
priority: P1
---

# Installation

This guide walks you through installing and configuring `nebula-credential` for your Rust project, including all dependencies and optional features.

## Prerequisites

Before installing nebula-credential, ensure you have:

- **Rust 1.75 or higher**: Check with `rustc --version`
  ```bash
  rustc --version
  # Should show: rustc 1.75.0 or higher
  ```
  
- **Cargo**: Rust's package manager (included with Rust)
  ```bash
  cargo --version
  ```

If you don't have Rust installed, visit [rustup.rs](https://rustup.rs/) to install the latest stable version.

## Basic Installation

Add `nebula-credential` to your project's `Cargo.toml`:

```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1.0", features = ["full"] }
```

**Why tokio?** Nebula-credential is built on async Rust and requires an async runtime. Tokio is the most widely-used and well-tested runtime.

### Verify Installation

Create a simple test to verify everything works:

```rust
// examples/verify_install.rs
use nebula_credential::CredentialManager;

#[tokio::main]
async fn main() {
    println!("✓ nebula-credential installed successfully");
}
```

Run it:
```bash
cargo run --example verify_install
```

You should see: `✓ nebula-credential installed successfully`

## Feature Flags

Nebula-credential provides optional features for different storage backends and credential types. Enable only what you need to minimize dependencies and compile time.

### Storage Providers

```toml
[dependencies]
nebula-credential = { version = "0.1.0", features = ["local-storage"] }
```

Available storage features:

| Feature | Description | Additional Dependencies |
|---------|-------------|------------------------|
| `local-storage` | SQLite-based local encrypted storage (default) | `rusqlite`, `tokio` |
| `aws-storage` | AWS Secrets Manager integration | `aws-sdk-secretsmanager`, `aws-config` |
| `vault-storage` | HashiCorp Vault integration | `vaultrs` |
| `azure-storage` | Azure Key Vault integration | `azure_security_keyvault` |
| `k8s-storage` | Kubernetes Secrets integration | `kube`, `k8s-openapi` |

**Example with AWS Secrets Manager**:
```toml
[dependencies]
nebula-credential = { version = "0.1.0", features = ["aws-storage"] }
aws-config = "1.0"
aws-sdk-secretsmanager = "1.0"
tokio = { version = "1.0", features = ["full"] }
```

### Credential Types

```toml
[dependencies]
nebula-credential = { version = "0.1.0", features = ["oauth2"] }
```

Available credential type features:

| Feature | Description | Use Case |
|---------|-------------|----------|
| `oauth2` | OAuth 2.0 support (default) | GitHub, Google, custom OAuth providers |
| `saml` | SAML 2.0 authentication | Enterprise SSO, Active Directory |
| `ldap` | LDAP/Active Directory | Corporate directory services |
| `jwt` | JWT token handling | API authentication, microservices |
| `mtls` | Mutual TLS (client certificates) | Service-to-service auth |
| `kerberos` | Kerberos authentication | Windows domain environments |

**Example with OAuth2 and JWT**:
```toml
[dependencies]
nebula-credential = { version = "0.1.0", features = ["oauth2", "jwt"] }
tokio = { version = "1.0", features = ["full"] }
```

### All Features

To enable all features (useful for development):

```toml
[dependencies]
nebula-credential = { version = "0.1.0", features = ["full"] }
tokio = { version = "1.0", features = ["full"] }
```

**Warning**: This includes all storage providers and credential types, significantly increasing compile time and binary size. Only use in development.

## Platform-Specific Setup

### Linux

No additional setup required. SQLite is included.

For production deployments with AWS/Vault/Azure, ensure credentials are configured:

```bash
# AWS credentials
export AWS_ACCESS_KEY_ID="your-access-key"
export AWS_SECRET_ACCESS_KEY="your-secret-key"
export AWS_REGION="us-east-1"

# HashiCorp Vault
export VAULT_ADDR="https://vault.example.com"
export VAULT_TOKEN="your-vault-token"
```

### macOS

No additional setup required. SQLite is included with macOS.

For Vault or AWS, install CLI tools:

```bash
# Install AWS CLI
brew install awscli

# Install Vault CLI
brew install vault
```

### Windows

SQLite is included with the `rusqlite` dependency.

For Windows environments using Active Directory/LDAP:

```toml
[dependencies]
nebula-credential = { version = "0.1.0", features = ["ldap"] }
tokio = { version = "1.0", features = ["full"] }
```

## Docker Setup

If running in Docker, include these in your `Dockerfile`:

```dockerfile
# Use Rust slim image
FROM rust:1.75-slim as builder

# Install system dependencies for SQLite and OpenSSL
RUN apt-get update && apt-get install -y \
    libsqlite3-dev \
    libssl-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Build with release optimizations
RUN cargo build --release

# Runtime image
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y \
    libsqlite3-0 \
    libssl3 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/your-app /usr/local/bin/

CMD ["your-app"]
```

## Minimal Configuration Example

Here's a minimal `Cargo.toml` for a typical web service using API keys:

```toml
[package]
name = "my-service"
version = "0.1.0"
edition = "2021"

[dependencies]
# Core dependencies
nebula-credential = { version = "0.1.0", features = ["local-storage"] }
tokio = { version = "1.0", features = ["full"] }

# Web framework (example with Axum)
axum = "0.7"
tower = "0.4"

# Serialization
serde = { version = "1.0", features = ["derive"] }
```

## Production Configuration Example

For production with AWS Secrets Manager and OAuth2:

```toml
[package]
name = "production-service"
version = "1.0.0"
edition = "2021"

[dependencies]
# Credential management with AWS backend
nebula-credential = { version = "0.1.0", features = ["aws-storage", "oauth2", "jwt"] }
tokio = { version = "1.0", features = ["full"] }

# AWS SDK
aws-config = "1.0"
aws-sdk-secretsmanager = "1.0"

# Web framework
axum = "0.7"
tower = "0.4"
tower-http = { version = "0.5", features = ["trace"] }

# Observability
tracing = "0.1"
tracing-subscriber = "0.3"

# Serialization
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

## Verifying Your Setup

After installation, verify all features work:

```rust
// examples/verify_features.rs
use nebula_credential::{
    CredentialManager,
    ApiKeyCredential,
    SecretString,
    storage::LocalStorage,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔍 Verifying nebula-credential installation...\n");
    
    // Test 1: Local storage
    let storage = LocalStorage::new("./test_credentials.db").await?;
    println!("✓ Local storage initialized");
    
    // Test 2: Credential creation
    let api_key = ApiKeyCredential::new(
        "test-service",
        SecretString::from("test-key-123"),
    );
    println!("✓ Credential created");
    
    // Test 3: Manager operations
    let manager = CredentialManager::new(storage);
    manager.store("test", api_key).await?;
    println!("✓ Credential stored");
    
    let retrieved: Option<ApiKeyCredential> = manager.retrieve("test").await?;
    assert!(retrieved.is_some());
    println!("✓ Credential retrieved");
    
    manager.delete("test").await?;
    println!("✓ Credential deleted");
    
    // Cleanup
    std::fs::remove_file("./test_credentials.db").ok();
    
    println!("\n🎉 All features verified! Installation successful.");
    
    Ok(())
}
```

Run the verification:
```bash
cargo run --example verify_features
```

Expected output:
```
🔍 Verifying nebula-credential installation...

✓ Local storage initialized
✓ Credential created
✓ Credential stored
✓ Credential retrieved
✓ Credential deleted

🎉 All features verified! Installation successful.
```

## Troubleshooting

### Common Issues

**Problem**: `error: failed to compile nebula-credential`

**Solution**: Update Rust to the latest version:
```bash
rustup update stable
```

**Problem**: `linking with cc failed` on Linux

**Solution**: Install build dependencies:
```bash
# Ubuntu/Debian
sudo apt-get install build-essential libssl-dev pkg-config libsqlite3-dev

# Fedora/RHEL
sudo dnf install gcc openssl-devel sqlite-devel

# Arch Linux
sudo pacman -S base-devel openssl sqlite
```

**Problem**: `error: no default features` when using feature flags

**Solution**: If you disable default features, explicitly enable what you need:
```toml
nebula-credential = { version = "0.1.0", default-features = false, features = ["local-storage"] }
```

**Problem**: Binary size too large

**Solution**: Use `--release` mode and strip symbols:
```bash
cargo build --release
strip target/release/your-binary

# Or configure in Cargo.toml
[profile.release]
strip = true
lto = true
codegen-units = 1
```

### Getting Help

If you encounter issues not covered here:

- Check [[Troubleshooting/Common-Issues]] for detailed solutions
- See [[Troubleshooting/Storage-Provider-Issues]] for storage-specific problems
- Review [[Architecture]] to understand system requirements
- Search [GitHub Issues](https://github.com/nebula-rs/nebula-credential/issues)

## Next Steps

Now that you have nebula-credential installed:

1. **Quick Start**: Follow [[Quick-Start]] for a 5-minute tutorial
2. **Core Concepts**: Read [[Core-Concepts]] to understand the credential lifecycle
3. **Examples**: Browse [[Examples/API-Key-Basic]] for copy-paste patterns
4. **Storage Setup**: Configure your storage provider:
   - [[Integrations/AWS-Secrets-Manager]]
   - [[Integrations/HashiCorp-Vault]]
   - [[Integrations/Azure-Key-Vault]]
   - [[Integrations/Kubernetes-Secrets]]

## See Also

- **Quick Start**: [[Quick-Start]] - Build your first credential manager in 5 minutes
- **Core Concepts**: [[Core-Concepts]] - Understand credential types and lifecycle
- **Architecture**: [[Architecture]] - Deep dive into design and patterns
- **Configuration**: [[Configuration-Options]] - All configuration options reference
- **API Reference**: [[API-Reference]] - Complete API documentation
- **Troubleshooting**: [[Troubleshooting/Common-Issues]] - Solutions to common problems

---

**Installation Checklist**:
- [ ] Rust 1.75+ installed (`rustc --version`)
- [ ] nebula-credential added to Cargo.toml
- [ ] Appropriate features enabled for your use case
- [ ] Verification example runs successfully
- [ ] Storage provider configured (if not using local storage)
- [ ] Dependencies for your platform installed (Linux: libsqlite3-dev, libssl-dev)
