# Data Model: Core Credential Abstractions (Phase 1)

**Feature**: 001-credential-core-abstractions  
**Date**: 2026-02-03  
**Phase**: 1 (Architecture & Contracts)

## Overview

This document defines the complete Rust type system for Phase 1 of nebula-credential, including core types, traits, and error hierarchy. Phase 1 focuses on foundational abstractions that enable secure credential storage and retrieval.

## Core Types

### CredentialId

Unique identifier for credentials with validation to prevent path traversal and injection attacks.

```rust
use serde::{Deserialize, Serialize};
use std::fmt;

/// Unique credential identifier (validated)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CredentialId(String);

impl CredentialId {
    /// Create new validated credential ID
    /// Returns error if ID is empty or contains invalid characters
    pub fn new(id: impl Into<String>) -> Result<Self, ValidationError> {
        let id = id.into();
        
        if id.is_empty() {
            return Err(ValidationError::EmptyCredentialId);
        }
        
        // Only allow alphanumeric, hyphens, underscores
        if !id.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
            return Err(ValidationError::InvalidCredentialId {
                id: id.clone(),
                reason: "contains invalid characters".to_string(),
            });
        }
        
        Ok(Self(id))
    }
    
    /// Get credential ID as string slice
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CredentialId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<CredentialId> for String {
    fn from(id: CredentialId) -> Self {
        id.0
    }
}

impl TryFrom<String> for CredentialId {
    type Error = ValidationError;
    
    fn try_from(s: String) -> Result<Self, Self::Error> {
        CredentialId::new(s)
    }
}
```

### SecretString

Zero-on-drop wrapper for sensitive string data with controlled access API.

```rust
use zeroize::{Zeroize, ZeroizeOnDrop};
use std::fmt;

/// Secret string with automatic memory zeroization
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretString {
    inner: String,
}

impl SecretString {
    /// Create new secret from any string-like value
    pub fn new<S: Into<String>>(s: S) -> Self {
        Self { inner: s.into() }
    }
    
    /// Access secret value within a closure scope
    /// Prevents accidental copying or leaking
    pub fn expose_secret<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&str) -> R,
    {
        f(&self.inner)
    }
    
    /// Get length without exposing content
    pub fn len(&self) -> usize {
        self.inner.len()
    }
    
    /// Check if empty without exposing content
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

// Prevent accidental secret leakage via Debug/Display
impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

// Serialize as redacted for safety
impl serde::Serialize for SecretString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str("[REDACTED]")
    }
}
```

### EncryptionKey

256-bit AES encryption key with automatic zeroization and derivation from passwords.

```rust
use zeroize::{Zeroize, ZeroizeOnDrop};
use argon2::{Argon2, ParamsBuilder, PasswordHasher};

/// 256-bit AES encryption key
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct EncryptionKey {
    key: [u8; 32], // 256 bits
}

impl EncryptionKey {
    /// Derive encryption key from password using Argon2id
    /// 
    /// Parameters:
    /// - password: Master password for key derivation
    /// - salt: 16-byte salt (must be stored with encrypted data)
    /// 
    /// Takes 100-200ms for security (prevents brute force)
    pub fn derive_from_password(
        password: &str,
        salt: &[u8; 16],
    ) -> Result<Self, CryptoError> {
        let params = ParamsBuilder::new()
            .m_cost(19456) // 19 MiB memory
            .t_cost(2)     // 2 iterations
            .p_cost(1)     // 1 thread
            .output_len(32)
            .build()
            .map_err(|e| CryptoError::KeyDerivation(e.to_string()))?;
        
        let argon2 = Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            params,
        );
        
        let mut key = [0u8; 32];
        argon2
            .hash_password_into(password.as_bytes(), salt, &mut key)
            .map_err(|e| CryptoError::KeyDerivation(e.to_string()))?;
        
        Ok(Self { key })
    }
    
    /// Load key directly from bytes (from secure storage)
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { key: bytes }
    }
    
    /// Get key bytes for cryptographic operations
    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.key
    }
}
```

### EncryptedData

Container for encrypted credential data with nonce, authentication tag, and version.

```rust
use serde::{Deserialize, Serialize};

/// Encrypted credential data with authentication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedData {
    /// Algorithm version for future migrations
    pub version: u8,
    
    /// 96-bit nonce (12 bytes) for AES-GCM
    pub nonce: [u8; 12],
    
    /// Encrypted ciphertext
    pub ciphertext: Vec<u8>,
    
    /// 128-bit authentication tag (16 bytes)
    pub tag: [u8; 16],
}

impl EncryptedData {
    /// Current encryption version (AES-256-GCM)
    pub const CURRENT_VERSION: u8 = 1;
    
    /// Create new encrypted data structure
    pub fn new(nonce: [u8; 12], ciphertext: Vec<u8>, tag: [u8; 16]) -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            nonce,
            ciphertext,
            tag,
        }
    }
    
    /// Check if version is supported
    pub fn is_supported_version(&self) -> bool {
        self.version == Self::CURRENT_VERSION
    }
}
```

### CredentialMetadata

Non-sensitive metadata about credentials (for management, not security).

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Credential metadata (non-sensitive)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialMetadata {
    /// When credential was created
    pub created_at: DateTime<Utc>,
    
    /// When credential was last accessed (None if never)
    pub last_accessed: Option<DateTime<Utc>>,
    
    /// When credential was last modified
    pub last_modified: DateTime<Utc>,
    
    /// Optional rotation policy (Phase 4)
    pub rotation_policy: Option<RotationPolicy>,
    
    /// User-defined tags for organization
    pub tags: HashMap<String, String>,
}

impl CredentialMetadata {
    /// Create new metadata with current timestamp
    pub fn new() -> Self {
        let now = Utc::now();
        Self {
            created_at: now,
            last_accessed: None,
            last_modified: now,
            rotation_policy: None,
            tags: HashMap::new(),
        }
    }
    
    /// Update last accessed timestamp
    pub fn mark_accessed(&mut self) {
        self.last_accessed = Some(Utc::now());
    }
    
    /// Update last modified timestamp
    pub fn mark_modified(&mut self) {
        self.last_modified = Utc::now();
    }
}

impl Default for CredentialMetadata {
    fn default() -> Self {
        Self::new()
    }
}

/// Rotation policy (stub for Phase 4)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationPolicy {
    pub interval_days: u32,
}
```

### CredentialContext

Request context carrying owner, scope, and trace information for observability.

```rust
use uuid::Uuid;

/// Request context for credential operations
#[derive(Debug, Clone)]
pub struct CredentialContext {
    /// Owner of the credential
    pub owner_id: String,
    
    /// Optional scope for isolation
    pub scope_id: Option<String>,
    
    /// Trace ID for distributed tracing
    pub trace_id: Uuid,
    
    /// Timestamp of the request
    pub timestamp: DateTime<Utc>,
}

impl CredentialContext {
    /// Create new context with owner
    pub fn new(owner_id: impl Into<String>) -> Self {
        Self {
            owner_id: owner_id.into(),
            scope_id: None,
            trace_id: Uuid::new_v4(),
            timestamp: Utc::now(),
        }
    }
    
    /// Set scope for this context
    pub fn with_scope(mut self, scope_id: impl Into<String>) -> Self {
        self.scope_id = Some(scope_id.into());
        self
    }
    
    /// Set trace ID for this context
    pub fn with_trace_id(mut self, trace_id: Uuid) -> Self {
        self.trace_id = trace_id;
        self
    }
}
```

## Trait Definitions

### Credential Trait

Core trait that all credential types must implement (Phase 5 adds specific types).

```rust
use async_trait::async_trait;

/// Core credential trait for all credential types
#[async_trait]
pub trait Credential: Send + Sync {
    /// Associated type for authentication output
    type Output: Send;
    
    /// Get credential ID
    fn id(&self) -> &CredentialId;
    
    /// Get credential metadata
    fn metadata(&self) -> &CredentialMetadata;
    
    /// Authenticate using this credential
    /// Returns provider-specific authentication output
    async fn authenticate(&self, context: &CredentialContext) -> Result<Self::Output, CredentialError>;
    
    /// Validate credential is well-formed
    fn validate(&self) -> Result<(), ValidationError>;
}
```

### StorageProvider Trait

Abstract trait for credential persistence backends (Phase 2 adds implementations).

```rust
use async_trait::async_trait;

/// Storage provider trait for credential persistence
#[async_trait]
pub trait StorageProvider: Send + Sync {
    /// Store encrypted credential with metadata
    async fn store(
        &self,
        id: &CredentialId,
        data: EncryptedData,
        metadata: CredentialMetadata,
        context: &CredentialContext,
    ) -> Result<(), StorageError>;
    
    /// Retrieve encrypted credential by ID
    async fn retrieve(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> Result<(EncryptedData, CredentialMetadata), StorageError>;
    
    /// Delete credential by ID
    async fn delete(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> Result<(), StorageError>;
    
    /// List all credential IDs (optionally filtered)
    async fn list(
        &self,
        filter: Option<&CredentialFilter>,
        context: &CredentialContext,
    ) -> Result<Vec<CredentialId>, StorageError>;
    
    /// Check if credential exists
    async fn exists(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> Result<bool, StorageError>;
}

/// Filter for listing credentials
#[derive(Debug, Clone, Default)]
pub struct CredentialFilter {
    /// Filter by tags
    pub tags: Option<HashMap<String, String>>,
    
    /// Filter by creation date range
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
}
```

## Error Types

### Error Hierarchy

```rust
use thiserror::Error;

/// Top-level credential error
#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("Storage error for credential '{id}': {source}")]
    Storage {
        id: String,
        #[source]
        source: StorageError,
    },
    
    #[error("Cryptographic error: {source}")]
    Crypto {
        #[source]
        source: CryptoError,
    },
    
    #[error("Validation error: {source}")]
    Validation {
        #[source]
        source: ValidationError,
    },
}

/// Storage operation errors
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Credential '{id}' not found")]
    NotFound { id: String },
    
    #[error("Failed to read credential '{id}': {source}")]
    ReadFailure {
        id: String,
        #[source]
        source: std::io::Error,
    },
    
    #[error("Failed to write credential '{id}': {source}")]
    WriteFailure {
        id: String,
        #[source]
        source: std::io::Error,
    },
    
    #[error("Permission denied for credential '{id}'")]
    PermissionDenied { id: String },
    
    #[error("Operation timed out after {duration:?}")]
    Timeout { duration: std::time::Duration },
}

/// Cryptographic operation errors
#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("Decryption failed - invalid key or corrupted data")]
    DecryptionFailed,
    
    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),
    
    #[error("Key derivation failed: {0}")]
    KeyDerivation(String),
    
    #[error("Nonce generation failed")]
    NonceGeneration,
    
    #[error("Unsupported encryption version: {0}")]
    UnsupportedVersion(u8),
}

/// Validation errors
#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("Credential ID cannot be empty")]
    EmptyCredentialId,
    
    #[error("Invalid credential ID '{id}': {reason}")]
    InvalidCredentialId { id: String, reason: String },
    
    #[error("Invalid credential format: {0}")]
    InvalidFormat(String),
}
```

## Type Relationships

```
CredentialId ──┐
               ├──> StorageProvider.store(id, data, metadata, context)
EncryptedData ─┤
CredentialMetadata ─┤
CredentialContext ──┘

EncryptionKey ──> encrypt() ──> EncryptedData
EncryptedData ──> decrypt() ──> SecretString

Credential Trait ──implements──> ApiKeyCredential (Phase 5)
                 ├──implements──> OAuth2Credential (Phase 5)
                 └──implements──> DatabaseCredential (Phase 5)

StorageProvider Trait ──implements──> LocalStorageProvider (Phase 1/2)
                      ├──implements──> AwsSecretsManagerProvider (Phase 2)
                      ├──implements──> AzureKeyVaultProvider (Phase 2)
                      └──implements──> HashiCorpVaultProvider (Phase 2)
```

## Module Organization

```
nebula-credential/src/
├── lib.rs                    # Public API exports
├── core/
│   ├── mod.rs                # Core types module
│   ├── error.rs              # Error hierarchy (CredentialError, StorageError, CryptoError)
│   ├── metadata.rs           # CredentialMetadata, RotationPolicy
│   ├── context.rs            # CredentialContext
│   └── result.rs             # Result type aliases
├── traits/
│   ├── mod.rs                # Trait module
│   ├── credential.rs         # Credential trait
│   └── storage.rs            # StorageProvider trait
└── utils/
    ├── mod.rs                # Utility module
    ├── secure_string.rs      # SecretString with zeroization
    └── crypto.rs             # EncryptionKey, encrypt/decrypt functions
```

## Usage Example

```rust
use nebula_credential::{
    CredentialId, SecretString, EncryptionKey, EncryptedData,
    CredentialMetadata, CredentialContext, StorageProvider,
};

// Create credential ID
let id = CredentialId::new("github_token")?;

// Create secret
let secret = SecretString::new("ghp_xxxxxxxxxxxx");

// Derive encryption key from master password
let salt = [0u8; 16]; // Load from secure storage
let key = EncryptionKey::derive_from_password("master-password", &salt)?;

// Encrypt credential (Phase 1 implementation)
let encrypted = encrypt(&key, &secret)?;

// Create metadata
let metadata = CredentialMetadata::new();

// Create context
let context = CredentialContext::new("user_123");

// Store via provider (Phase 2 implementation)
// provider.store(&id, encrypted, metadata, &context).await?;
```

## References

- Feature Specification: [spec.md](./spec.md)
- Research Findings: [research.md](./research.md)
- Implementation Plan: [plan.md](./plan.md)
- Full Type Definitions: `crates/nebula-credential/docs/Meta/DATA-MODEL-CODE.md`
