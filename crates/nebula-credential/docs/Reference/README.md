---
title: README
tags: [nebula, nebula-credential, docs]
status: published
created: 2025-08-24
---

# API Reference

Complete API reference documentation for nebula-credential.

## Core Components

### [Credential Trait](https://claude.ai/chat/CredentialTrait.md)

The foundational trait that all credential types implement. Defines the interface for credential initialization, token retrieval, refresh, and validation.

### [Credential Types](https://claude.ai/chat/CredentialTypes.md)

Comprehensive reference for all built-in credential types including API keys, OAuth2, JWT, AWS, certificates, and more.

### [Credential Manager](https://claude.ai/chat/CredentialManager.md)

Central management interface for all credential operations including creation, retrieval, refresh, rotation, and deletion.

## Storage & Persistence

### [Storage Backends](https://claude.ai/chat/StorageBackends.md)

Reference for all supported storage backends including memory, file, database, HashiCorp Vault, AWS Secrets Manager, and more.

### [Configuration](https://claude.ai/chat/Configuration.md)

Complete configuration options for credential manager, storage, caching, security, and monitoring.

## Security

### [Security Features](https://claude.ai/chat/SecurityFeatures.md)

Detailed reference for security features including encryption, access control, audit logging, and compliance.

### [Error Types](https://claude.ai/chat/ErrorTypes.md)

Complete reference of error types, error handling patterns, and recovery strategies.

## Monitoring & Metrics

### [Metrics](https://claude.ai/chat/Metrics.md)

Available metrics, monitoring endpoints, and observability features for credential operations.

## Quick Reference Card

### Common Types

```rust
use nebula_credential::prelude::*;

// Core types
CredentialId        // Unique credential identifier
CredentialManager   // Main management interface
Token              // Authentication token
SecureString       // Encrypted string in memory

// Credential types
ApiKeyCredential   // API key authentication
OAuth2Credential   // OAuth 2.0 flow
JwtCredential      // JWT tokens
AwsCredential      // AWS credentials
BasicCredential    // Basic auth
CertificateCredential // Client certificates
```

### Common Operations

```rust
// Create
let id = manager.create_credential(type, input, context).await?;

// Retrieve
let token = manager.get_token(&id).await?;

// Refresh
let new_token = manager.refresh_credential(&id).await?;

// Rotate
let new_id = manager.rotate_credential(&id).await?;

// Delete
manager.delete_credential(&id).await?;
```

### Common Patterns

```rust
// With retry
let token = retry::retry(Fixed::from_millis(100).take(3), || {
    manager.get_token(&id)
}).await?;

// With timeout
let token = timeout(Duration::from_secs(5), 
    manager.get_token(&id)
).await??;

// With fallback
let token = manager.get_token(&primary_id).await
    .or_else(|_| manager.get_token(&fallback_id)).await?;
```

## API Stability

|Component|Stability|Since|
|---|---|---|
|Credential Trait|Stable|1.0.0|
|CredentialManager|Stable|1.0.0|
|Storage Backends|Stable|1.0.0|
|OAuth2 Support|Stable|1.1.0|
|MFA Support|Beta|1.2.0|
|ZKP Support|Experimental|1.3.0|

## Version Compatibility

|nebula-credential|nebula-action|nebula-resource|Rust|
|---|---|---|---|
|1.3.x|1.2+|1.1+|1.70+|
|1.2.x|1.1+|1.0+|1.65+|
|1.1.x|1.0+|1.0+|1.60+|
|1.0.x|1.0+|1.0+|1.60+|

## Related Documentation

- [Getting Started](https://claude.ai/Getting-Started/)
- [How-To Guides](https://claude.ai/How-To/)
- [Examples](https://claude.ai/Examples/)
- [Architecture](https://claude.ai/Architecture.md)
