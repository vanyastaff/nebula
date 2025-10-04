# nebula-credential Crate Structure

## Overview

The `nebula-credential` crate provides secure, extensible credential management for the Nebula ecosystem. It implements a sophisticated architecture with multi-level caching, automatic token refresh, distributed locking, and pluggable authentication strategies.

## Module Organization

```
nebula-credential/
├── src/
│   ├── lib.rs                    # Crate root with prelude
│   ├── authenticator/            # Authentication strategies
│   │   ├── mod.rs
│   │   ├── traits.rs             # ClientAuthenticator trait
│   │   ├── chain.rs              # ChainAuthenticator composition
│   │   └── common.rs             # Common authentication utilities
│   ├── core/                     # Core types and primitives
│   │   ├── mod.rs
│   │   ├── context.rs            # CredentialContext for operations
│   │   ├── ephemeral.rs          # Ephemeral<T> zero-copy wrapper
│   │   ├── error.rs              # CredentialError type
│   │   ├── id.rs                 # CredentialId wrapper
│   │   ├── key.rs                # Encryption key management
│   │   ├── metadata.rs           # CredentialMetadata
│   │   ├── secure.rs             # SecureString with zeroization
│   │   ├── state.rs              # CredentialState trait
│   │   ├── time.rs               # Time utilities
│   │   └── token.rs              # AccessToken type
│   ├── manager/                  # Credential management
│   │   ├── mod.rs
│   │   ├── builder.rs            # ManagerBuilder pattern
│   │   ├── manager.rs            # CredentialManager implementation
│   │   ├── negative_cache.rs     # Failure caching
│   │   └── policy.rs             # RefreshPolicy configuration
│   ├── migration/                # State migration support
│   │   ├── mod.rs
│   │   └── migrator.rs           # Version migration logic
│   ├── registry/                 # Type registration
│   │   ├── mod.rs
│   │   └── factory.rs            # CredentialFactory trait & registry
│   ├── testing/                  # Testing utilities (private)
│   │   ├── mod.rs
│   │   ├── assertions.rs         # Test assertions
│   │   ├── fixtures.rs           # Test fixtures
│   │   ├── helpers.rs            # Helper functions
│   │   └── mocks.rs              # Mock implementations
│   └── traits/                   # Public traits
│       ├── mod.rs
│       ├── cache.rs              # TokenCache trait
│       ├── credential.rs         # Credential trait
│       ├── lock.rs               # DistributedLock trait
│       └── storage.rs            # StateStore trait
├── examples/                     # (Currently empty)
├── tests/                        # (Currently empty)
└── docs/                         # Comprehensive documentation
    ├── README.md                 # Main documentation
    ├── Architecture.md           # Architecture deep dive
    └── ... (12+ additional docs)
```

## Core Components

### 1. Credential Trait (`traits/credential.rs`)

The main trait for implementing custom credential types:

```rust
pub trait Credential: Send + Sync + 'static {
    type Input: Serialize + DeserializeOwned + Send + Sync;
    type State: CredentialState;
    const TYPE_NAME: &'static str;

    fn metadata(&self) -> CredentialMetadata;
    async fn initialize(&self, input: &Self::Input, ctx: &mut CredentialContext)
        -> Result<(Self::State, Option<AccessToken>), CredentialError>;
    async fn refresh(&self, state: &mut Self::State, ctx: &mut CredentialContext)
        -> Result<AccessToken, CredentialError>;
    async fn revoke(&self, state: &mut Self::State, ctx: &mut CredentialContext)
        -> Result<(), CredentialError>;
    async fn validate(&self, state: &Self::State, ctx: &CredentialContext)
        -> Result<bool, CredentialError>;
}
```

### 2. CredentialManager (`manager/manager.rs`)

Main entry point for credential operations with multi-level caching:

**Key Features:**
- L1/L2 cache integration via `TokenCache` trait
- Distributed locking via `DistributedLock` trait
- Negative caching for failed operations
- Automatic token refresh based on `RefreshPolicy`
- Registry-based credential type dispatching

**Primary Methods:**
- `get_token(credential_id)` - Get token with automatic refresh
- `create_credential(type, input)` - Create new credential
- `refresh_credential(credential_id)` - Force refresh
- `delete_credential(credential_id)` - Delete credential

**Builder Pattern:**
```rust
CredentialManager::builder()
    .with_store(store)
    .with_lock(lock)
    .with_cache(cache)
    .with_policy(policy)
    .build()
```

### 3. CredentialRegistry (`registry/factory.rs`)

Type-safe registry for credential implementations:

```rust
pub trait CredentialFactory: Send + Sync {
    fn type_name(&self) -> &'static str;
    async fn create_and_init(&self, input_json: Value, cx: &mut CredentialContext)
        -> Result<(Box<dyn Serialize>, Option<AccessToken>), CredentialError>;
    async fn refresh(&self, state_json: Value, cx: &mut CredentialContext)
        -> Result<(Box<dyn Serialize>, AccessToken), CredentialError>;
}
```

### 4. Security Primitives

#### SecureString (`core/secure.rs`)
- Zeroizes memory on drop
- Integration with `secrecy` crate
- Prevents accidental logging

#### Ephemeral<T> (`core/ephemeral.rs`)
- Zero-copy wrapper for sensitive data
- Automatic cleanup
- Time-bounded access

#### Key Management (`core/key.rs`)
- Encryption key derivation
- Secure key storage

### 5. Pluggable Traits

#### TokenCache (`traits/cache.rs`)
```rust
pub trait TokenCache: Send + Sync {
    async fn get(&self, key: &str) -> Result<Option<AccessToken>, CacheError>;
    async fn set(&self, key: &str, token: AccessToken, ttl: Duration) -> Result<(), CacheError>;
    async fn delete(&self, key: &str) -> Result<(), CacheError>;
}
```

#### StateStore (`traits/storage.rs`)
```rust
pub trait StateStore: Send + Sync {
    async fn load(&self, id: &str) -> Result<(Value, StateVersion), StorageError>;
    async fn save(&self, id: &str, state: Value, version: StateVersion) -> Result<(), StorageError>;
    async fn delete(&self, id: &str) -> Result<(), StorageError>;
}
```

#### DistributedLock (`traits/lock.rs`)
```rust
pub trait DistributedLock: Send + Sync {
    type Guard: LockGuard;
    async fn acquire(&self, key: &str, ttl: Duration) -> Result<Self::Guard, LockError>;
}
```

### 6. Authenticator System (`authenticator/`)

Composable authentication strategies:

```rust
pub trait ClientAuthenticator: Send + Sync {
    async fn authenticate(&self, request: &mut http::Request<Vec<u8>>)
        -> Result<(), AuthError>;
}

pub struct ChainAuthenticator {
    authenticators: Vec<Box<dyn ClientAuthenticator>>,
}
```

Allows composing multiple authentication methods (OAuth2, API keys, mTLS, etc.)

## Data Flow

### Token Retrieval Flow

1. **Check negative cache** - Fast-fail for known failures
2. **Check L1 cache** (in-memory) - Fastest path
3. **Check L2 cache** (Redis/external) - Still fast
4. **Acquire distributed lock** - Prevent thundering herd
5. **Re-check caches** - Someone else might have refreshed
6. **Load state from StateStore**
7. **Call credential.refresh()** via registry
8. **Save updated state**
9. **Update L1 and L2 caches**
10. **Release lock**
11. **Return token**

### Credential Creation Flow

1. **Validate input** via credential type
2. **Get factory from registry**
3. **Call credential.initialize()**
4. **Handle interactive flows** (e.g., OAuth2 redirect)
5. **Save state to StateStore**
6. **Optionally cache initial token**
7. **Audit log creation event**
8. **Return credential ID**

## Testing Infrastructure

The crate includes comprehensive testing utilities (in private `testing` module):

- **Mocks** (`testing/mocks.rs`) - Mock implementations of all traits
- **Fixtures** (`testing/fixtures.rs`) - Common test data
- **Assertions** (`testing/assertions.rs`) - Custom assertions
- **Helpers** (`testing/helpers.rs`) - Test helper functions

**Note**: Currently there are 0 integration tests in `tests/` directory.

## Feature Flags

Currently no optional features defined in `Cargo.toml`.

## Dependencies

### Core Dependencies
- **async-trait** - Async trait support
- **tokio** - Async runtime (time, sync, rt, fs)
- **dashmap** - Concurrent HashMap
- **parking_lot** - Fast synchronization primitives
- **arc-swap** - Atomic Arc swapping

### Security
- **aes-gcm** - AES-GCM encryption
- **argon2** - Password hashing
- **secrecy** - Secret handling
- **zeroize** - Memory zeroization
- **sha2** - SHA-256 hashing
- **subtle** - Constant-time operations

### Serialization
- **serde** / **serde_json** - Serialization
- **erased-serde** - Type-erased serialization

### Utilities
- **chrono** - Time handling
- **uuid** - Unique identifiers
- **regex** - Pattern matching
- **base64** - Encoding

## Known Issues

1. **Cyclic dependency** in nebula-core prevents tests from running
2. **No integration tests** - `tests/` directory is empty
3. **No examples** - `examples/` directory is empty
4. **Limited documentation examples** - Docs have diagrams but few code examples

## API Stability

The crate uses:
- `#![warn(missing_docs)]` - Documentation warnings
- `#![deny(unsafe_code)]` - No unsafe code allowed
- `#![forbid(unsafe_code)]` - Double enforcement

Public API is exported via `prelude` module for convenience.
