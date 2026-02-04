# Research Findings: Core Credential Abstractions

**Feature**: 001-credential-core-abstractions  
**Date**: 2026-02-03  
**Phase**: 0 (Research & Discovery)

## Summary

This document consolidates research findings from cryptographic best practices, async Rust patterns, and credential management system design. All NEEDS CLARIFICATION items from Technical Context have been resolved through MCP tool research (Context7, DeepWiki) and existing nebula-credential documentation.

## Research Areas

### 1. AES-256-GCM Implementation Patterns

**Research Question**: What are the critical security considerations for implementing AES-256-GCM encryption, specifically regarding nonce management and timing attacks?

**Sources**:
- DeepWiki: RustCrypto/AEADs repository
- Context7: aes-gcm crate documentation
- nebula-credential/docs/Meta/TECHNICAL-DESIGN.md

**Findings**:

#### Nonce Management (Critical)
- **Requirement**: Nonces MUST be unique per message to maintain AES-GCM security
- **Standard size**: 96 bits (U12 in Rust type system)
- **Nonce reuse**: Catastrophic failure - compromises all messages encrypted with same key
- **Recommendation**: Counter-based nonce with atomic operations (AtomicU64)

**Rationale**: Pure random nonces have collision risk at scale (birthday paradox). Counter-based with 64-bit atomic integer provides 2^64 unique nonces (18 quintillion operations) before exhaustion - practically unlimited for credential storage use case.

#### Timing Attack Prevention
- **Requirement**: Constant-time tag comparison using `subtle` crate
- **Implementation**: RustCrypto AEADs use `expected_tag.ct_eq(tag).into()` for verification
- **Hardware acceleration**: AES-NI provides constant-time AES operations on modern CPUs
- **Warning**: Variable-time multiplication operations on older processors can leak timing information

**Rationale**: Timing attacks can reveal key bits by measuring decryption time. Constant-time comparison prevents this attack vector.

#### AEAD Trait System
- **KeyInit trait**: Provides `new()` method for key-based initialization
- **Aead trait**: Provides `encrypt()` and `decrypt()` methods
- **AeadCore trait**: Defines NonceSize, TagSize, CiphertextOverhead associated types
- **AeadInPlace trait**: In-place encryption for performance (optional)

**Decision**:
- **Chosen**: Counter-based nonce with AtomicU64, constant-time tag comparison
- **Implementation**: 
  ```rust
  use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
  use std::sync::atomic::{AtomicU64, Ordering};
  
  struct NonceGenerator {
      counter: AtomicU64,
  }
  
  impl NonceGenerator {
      fn next(&self) -> Nonce {
          let value = self.counter.fetch_add(1, Ordering::SeqCst);
          // Convert u64 to 12-byte nonce (8 bytes value + 4 bytes zeros)
          let mut nonce_bytes = [0u8; 12];
          nonce_bytes[0..8].copy_from_slice(&value.to_le_bytes());
          *Nonce::from_slice(&nonce_bytes)
      }
  }
  ```

---

### 2. Argon2id Key Derivation Parameters

**Research Question**: What are the recommended Argon2id parameters for password-based key derivation that balance security with usability?

**Sources**:
- Context7: argon2 crate documentation (docs.rs/argon2)
- OWASP Password Storage Cheat Sheet 2024
- nebula-credential spec.md requirements

**Findings**:

#### OWASP 2024 Recommendations
- **Algorithm**: Argon2id (hybrid of Argon2i and Argon2d)
- **Memory cost (m_cost)**: 19 MiB minimum (19456 KB)
- **Time cost (t_cost)**: 2 iterations minimum
- **Parallelism (p_cost)**: 1 thread (default)
- **Output length**: 32 bytes (256 bits for AES-256 key)

#### Performance Impact
- **Target derivation time**: 100-200ms on standard hardware
- **Security rationale**: Slows down brute-force attacks (100ms per attempt = 10 attempts/sec maximum)
- **Usability**: Acceptable for one-time initialization, not interactive operations

#### Argon2id Variants
- **Argon2d**: Maximizes resistance to GPU cracking (data-dependent memory access)
- **Argon2i**: Optimized to resist side-channel attacks (data-independent memory access)
- **Argon2id**: Hybrid combining both approaches (RECOMMENDED)

**Decision**:
- **Chosen**: Argon2id with 19 MiB memory, 2 iterations, 32-byte output
- **Implementation**:
  ```rust
  use argon2::{Argon2, ParamsBuilder, PasswordHasher};
  
  let params = ParamsBuilder::new()
      .m_cost(19456)  // 19 MiB
      .t_cost(2)      // 2 iterations
      .p_cost(1)      // 1 thread
      .output_len(32) // 256 bits
      .build()?;
  
  let argon2 = Argon2::new(
      argon2::Algorithm::Argon2id,
      argon2::Version::V0x13,
      params,
  );
  
  let mut key_bytes = [0u8; 32];
  argon2.hash_password_into(password.as_bytes(), salt, &mut key_bytes)?;
  ```

---

### 3. Memory Zeroization Patterns

**Research Question**: How do we ensure sensitive data (encryption keys, passwords, secrets) is securely erased from memory when no longer needed?

**Sources**:
- Context7: zeroize crate documentation
- nebula-credential existing SecretString implementation
- Rust Drop trait semantics

**Findings**:

#### Zeroize Crate Features
- **Zeroize trait**: Provides `zeroize()` method to securely overwrite memory
- **ZeroizeOnDrop derive**: Automatically calls `zeroize()` in Drop implementation
- **Compiler guarantees**: Prevents optimization from removing zeroization (volatile writes)
- **Usage**: Apply to structs containing sensitive byte arrays

#### SecretString Pattern
- **Problem**: String/Vec<u8> do not automatically zero memory on drop
- **Solution**: Wrapper type with ZeroizeOnDrop derive
- **API design**: Controlled access via closure to prevent copying

**Existing Implementation** (nebula-credential/src/utils/secure_string.rs):
```rust
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SecretString {
    inner: String,
}

impl SecretString {
    pub fn new<S: Into<String>>(s: S) -> Self {
        Self { inner: s.into() }
    }
    
    pub fn expose_secret<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&str) -> R,
    {
        f(&self.inner)
    }
}

// Prevent accidental leaking via Debug
impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}
```

**Decision**:
- **Chosen**: Use existing SecretString pattern with ZeroizeOnDrop
- **Extend to**: EncryptionKey (256-bit key material), any other sensitive buffers
- **Rationale**: Proven pattern, compiler-verified zeroization, prevents accidental logging

---

### 4. Async Storage Trait Design

**Research Question**: What is the best pattern for defining async trait methods in Rust for the StorageProvider trait?

**Sources**:
- Context7: tokio docs (tokio::sync patterns)
- Context7: async-trait crate documentation
- nebula-credential existing trait definitions

**Findings**:

#### Async Trait Options
1. **async-trait crate**: Macro that transforms `async fn` to return `Pin<Box<dyn Future>>`
2. **Manual Future**: Return `impl Future` or boxed Future manually
3. **RPITIT** (Rust 1.75+): Return position impl Trait in traits (native async fn support)

#### Trade-offs
- **async-trait**: Simple syntax, small heap allocation per call, widely used
- **Manual Future**: No allocations, complex implementation, verbose
- **RPITIT**: Native support, but requires Rust 1.75+ (we're on 1.92, so available)

#### Tokio Async Patterns (from research)
- **RwLock vs Mutex**: Use RwLock for read-heavy workloads (credential retrieval >> storage)
- **Mutex FIFO**: Tokio Mutex provides FIFO fairness (predictable lock acquisition order)
- **Cancellation**: All async operations should support `tokio::select!` for cancellation

**Decision**:
- **Chosen**: async-trait crate for StorageProvider trait (compatibility with Rust 1.92)
- **Rationale**: 
  - Simpler syntax than manual futures
  - Small allocation overhead acceptable for I/O-bound storage operations
  - Widely adopted in Rust async ecosystem
  - Future migration to RPITIT possible (minor refactor)
  
**Implementation**:
```rust
use async_trait::async_trait;

#[async_trait]
pub trait StorageProvider: Send + Sync {
    async fn store(
        &self,
        id: &CredentialId,
        data: EncryptedData,
        metadata: CredentialMetadata,
        context: &CredentialContext,
    ) -> Result<(), StorageError>;
    
    async fn retrieve(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> Result<EncryptedData, StorageError>;
    
    // ... other methods
}
```

---

### 5. Error Hierarchy Design

**Research Question**: How should we structure error types for nebula-credential to provide actionable context while maintaining clear separation of concerns?

**Sources**:
- Constitution Principle II (Isolated Error Handling)
- thiserror crate best practices
- nebula-credential existing error patterns

**Findings**:

#### Constitution Requirements
- Each crate defines its own error type (no shared nebula-error dependency)
- Use thiserror for error definitions
- Convert errors at crate boundaries with context
- Include actionable error messages with field details

#### Error Hierarchy Strategy
```
CredentialError (top-level)
├── Storage(StorageError)     - File I/O, permissions, not found
├── Crypto(CryptoError)        - Encryption, decryption, key derivation
├── Validation(ValidationError) - Invalid IDs, malformed data
└── Context(ContextError)      - Missing required context fields
```

#### Error Context Best Practices
- Include operation being performed (store, retrieve, delete)
- Include credential ID (but redact secret values)
- Include underlying cause with `#[source]` attribute
- Provide suggestions for resolution where applicable

**Decision**:
- **Chosen**: Three-tier hierarchy with thiserror
- **Implementation**:
  ```rust
  use thiserror::Error;
  
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
      
      #[error("Validation error for credential '{id}': {message}")]
      Validation {
          id: String,
          message: String,
      },
  }
  
  #[derive(Debug, Error)]
  pub enum StorageError {
      #[error("Credential '{id}' not found")]
      NotFound { id: String },
      
      #[error("Failed to write credential '{id}': {source}")]
      WriteFailure {
          id: String,
          #[source]
          source: std::io::Error,
      },
      
      #[error("Operation timed out after {duration:?}")]
      Timeout { duration: std::time::Duration },
  }
  
  #[derive(Debug, Error)]
  pub enum CryptoError {
      #[error("Decryption failed - invalid key or corrupted data")]
      DecryptionFailed,
      
      #[error("Key derivation failed: {0}")]
      KeyDerivation(String),
      
      #[error("Nonce generation failed")]
      NonceGeneration,
  }
  ```

---

## Technology Stack

### Cryptography
- **aes-gcm v0.10+**: AEAD encryption (AES-256-GCM)
- **argon2 latest**: Password-based key derivation (Argon2id)
- **zeroize v1.8+**: Memory zeroization for secrets
- **subtle v2.5+**: Constant-time comparisons

### Async Runtime
- **tokio v1.49+**: Async runtime (already in workspace)
- **async-trait v0.1+**: Async trait support

### Serialization
- **serde v1.0+**: Serialization framework (already in workspace)
- **serde_json v1.0+**: JSON format for local storage

### Error Handling
- **thiserror v1.0+**: Error derive macros

### Utilities
- **uuid v1.7+**: Unique identifiers
- **chrono v0.4+**: Timestamp handling

## Open Questions

### Resolved
- ✅ Nonce management strategy → Counter-based with AtomicU64
- ✅ Key derivation parameters → Argon2id, 19 MiB, 2 iterations
- ✅ Memory zeroization → ZeroizeOnDrop with SecretString pattern
- ✅ Async trait design → async-trait crate
- ✅ Error hierarchy → Three-tier with thiserror

### Outstanding
None - all technical decisions finalized for Phase 1 implementation.

## Security Considerations

### Critical Requirements
1. **Nonce Uniqueness**: MUST use AtomicU64 counter to prevent reuse
2. **Constant-Time Operations**: MUST use subtle::ConstantTimeEq for tag comparison
3. **Memory Zeroization**: MUST apply ZeroizeOnDrop to all sensitive types
4. **File Permissions**: MUST set 0600 (Unix) or equivalent ACLs (Windows) on credential files
5. **Error Message Hygiene**: MUST redact secrets in all error messages and logs

### Attack Mitigation
- **Timing attacks**: Constant-time comparisons, hardware AES acceleration
- **Nonce reuse**: Atomic counter prevents collision
- **Memory inspection**: Zeroization prevents secrets in memory dumps
- **Brute force**: Argon2id with 19 MiB memory makes attacks expensive

## Performance Expectations

### Benchmarks (target)
- Encryption (AES-256-GCM): <1ms per credential (<10KB)
- Decryption (AES-256-GCM): <1ms per credential
- Key derivation (Argon2id): 100-200ms (one-time cost)
- File I/O (local storage): <5ms read, <10ms write

### Scaling Considerations
- Nonce counter: 2^64 operations before exhaustion (practically unlimited)
- Memory per credential: ~10KB typical, 64KB limit recommended
- Concurrent operations: Thread-safe via Tokio sync primitives

## References

- [RustCrypto/AEADs Wiki](https://deepwiki.com/RustCrypto/AEADs)
- [Tokio async patterns](https://docs.rs/tokio/1.49.0/)
- [Argon2 crate docs](https://docs.rs/argon2/)
- [OWASP Password Storage](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html)
- nebula-credential/docs/Meta/TECHNICAL-DESIGN.md
- nebula-credential/docs/Meta/ARCHITECTURE-DESIGN.md
