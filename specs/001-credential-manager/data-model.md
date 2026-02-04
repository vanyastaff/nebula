# Data Model: Credential Manager

**Feature**: Credential Manager API  
**Date**: 2026-02-04  
**Phase**: 1 - Design

## Overview

This document defines all data structures for the Credential Manager, including the core manager struct, configuration types, builder pattern, error types, and supporting utilities. These types build on Phase 1 core abstractions (CredentialId, CredentialMetadata, CredentialContext) and Phase 2 storage providers (StorageProvider trait).

---

## Entity Diagram

```
┌─────────────────────────────┐
│   CredentialManager         │
│  ┌───────────────────────┐  │
│  │ storage: Arc<dyn      │  │──────► StorageProvider (Phase 2)
│  │   StorageProvider>    │  │
│  │ cache: Option<Cache>  │  │──────► CacheLayer (moka)
│  │ config: ManagerConfig │  │──┐
│  └───────────────────────┘  │  │
└─────────────────────────────┘  │
                                 │
                                 ▼
                    ┌────────────────────────┐
                    │   ManagerConfig        │
                    │  ┌──────────────────┐  │
                    │  │ cache_config     │  │──────► CacheConfig
                    │  │ batch_concurrency│  │
                    │  │ default_scope    │  │──────► ScopeId (Phase 1)
                    │  │ retry_policy     │  │──────► RetryPolicy (Phase 2)
                    │  └──────────────────┘  │
                    └────────────────────────┘

┌──────────────────────────────────────────┐
│   CredentialManagerBuilder<HasStorage>  │
│  ┌────────────────────────────────────┐  │
│  │ storage: Option<Arc<...>>          │  │
│  │ cache_config: Option<CacheConfig>  │  │
│  │ _marker: PhantomData<HasStorage>   │  │
│  └────────────────────────────────────┘  │
└──────────────────────────────────────────┘
```

---

## 1. Core Manager Type

### CredentialManager

**Purpose**: Central management interface for all credential operations. Wraps storage provider with optional caching layer.

**Location**: `crates/nebula-credential/src/manager/manager.rs`

```rust
use std::sync::Arc;
use moka::future::Cache;

/// Central credential manager with caching and storage abstraction
pub struct CredentialManager {
    /// Underlying storage provider (LocalStorage, AWS, Vault, K8s)
    storage: Arc<dyn StorageProvider>,
    
    /// Optional in-memory cache with LRU + TTL
    cache: Option<Arc<CacheLayer>>,
    
    /// Manager configuration
    config: ManagerConfig,
}

impl CredentialManager {
    /// Create builder for constructing manager instance
    pub fn builder() -> CredentialManagerBuilder<No> {
        CredentialManagerBuilder::new()
    }
    
    /// Store a credential with encryption and optional caching
    ///
    /// # Arguments
    ///
    /// * `id` - Unique credential identifier
    /// * `credential` - Credential data (will be encrypted before storage)
    ///
    /// # Errors
    ///
    /// Returns `ManagerError::StorageError` if storage operation fails
    pub async fn store(
        &self,
        id: &CredentialId,
        credential: &Credential,
    ) -> ManagerResult<()> {
        // Implementation: store to provider, invalidate cache
    }
    
    /// Retrieve a credential by ID with cache-aside pattern
    ///
    /// # Cache Behavior
    ///
    /// - Cache hit: Returns cached credential (<10ms)
    /// - Cache miss: Fetches from storage, populates cache (<100ms)
    ///
    /// # Errors
    ///
    /// Returns `ManagerError::NotFound` if credential doesn't exist
    pub async fn retrieve(
        &self,
        id: &CredentialId,
    ) -> ManagerResult<Option<Credential>> {
        // Implementation: check cache, on miss fetch + populate
    }
    
    /// Delete a credential and invalidate cache
    pub async fn delete(&self, id: &CredentialId) -> ManagerResult<()> {
        // Implementation: delete from storage, invalidate cache
    }
    
    /// List all credential IDs (no caching, always fresh)
    pub async fn list(&self) -> ManagerResult<Vec<CredentialId>> {
        // Implementation: delegate to storage provider
    }
    
    // Scope-based operations (multi-tenant)
    
    /// Retrieve credential within specific scope
    pub async fn retrieve_scoped(
        &self,
        id: &CredentialId,
        scope: &ScopeId,
    ) -> ManagerResult<Option<Credential>> {
        // Implementation: retrieve + validate scope matches
    }
    
    /// List credentials within scope (hierarchical)
    pub async fn list_scoped(&self, scope: &ScopeId) -> ManagerResult<Vec<CredentialId>> {
        // Implementation: filter by scope prefix
    }
    
    // Batch operations
    
    /// Store multiple credentials in parallel
    pub async fn store_batch(
        &self,
        credentials: Vec<(CredentialId, Credential)>,
    ) -> Vec<ManagerResult<()>> {
        // Implementation: JoinSet with bounded concurrency
    }
    
    /// Retrieve multiple credentials in parallel
    pub async fn retrieve_batch(
        &self,
        ids: Vec<CredentialId>,
    ) -> Vec<ManagerResult<Option<Credential>>> {
        // Implementation: check cache, batch fetch misses
    }
    
    /// Delete multiple credentials in parallel
    pub async fn delete_batch(
        &self,
        ids: Vec<CredentialId>,
    ) -> Vec<ManagerResult<()>> {
        // Implementation: parallel deletion + cache invalidation
    }
    
    // Validation operations
    
    /// Validate single credential (expiration, format)
    pub async fn validate(
        &self,
        id: &CredentialId,
    ) -> ManagerResult<ValidationResult> {
        // Implementation: retrieve + check expiration/format
    }
    
    /// Validate multiple credentials in batch
    pub async fn validate_batch(
        &self,
        ids: Vec<CredentialId>,
    ) -> Vec<ValidationResult> {
        // Implementation: parallel validation
    }
    
    // Cache management
    
    /// Clear all cached credentials
    pub async fn clear_cache(&self) -> ManagerResult<()> {
        // Implementation: cache.invalidate_all()
    }
    
    /// Clear cache for specific credential
    pub async fn clear_cache_for(&self, id: &CredentialId) -> ManagerResult<()> {
        // Implementation: cache.invalidate(id)
    }
    
    /// Get cache performance statistics
    pub fn cache_stats(&self) -> Option<CacheStats> {
        // Implementation: return hit/miss/size metrics
    }
}
```

**Relationships**:
- **Uses**: `StorageProvider` (dependency injection via builder)
- **Uses**: `CacheLayer` (optional, created from CacheConfig)
- **Contains**: `ManagerConfig` (configuration settings)
- **Returns**: `Credential` types (from Phase 1)
- **Accepts**: `CredentialId`, `ScopeId` (from Phase 1)

**Lifecycle**:
1. Constructed via `CredentialManagerBuilder`
2. Long-lived (application lifetime, Arc-cloned for sharing)
3. Thread-safe (Send + Sync, internal Arc for cache/storage)

---

## 2. Configuration Types

### ManagerConfig

**Purpose**: Configuration for CredentialManager behavior

**Location**: `crates/nebula-credential/src/manager/config.rs`

```rust
use std::time::Duration;

/// Configuration for credential manager
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    /// Cache configuration (if caching enabled)
    pub cache_config: Option<CacheConfig>,
    
    /// Default scope for operations without explicit scope
    pub default_scope: Option<ScopeId>,
    
    /// Maximum concurrent operations in batch
    pub batch_concurrency: usize,
    
    /// Retry policy for storage operations
    pub retry_policy: RetryPolicy,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            cache_config: None, // Caching disabled by default
            default_scope: None,
            batch_concurrency: 10,
            retry_policy: RetryPolicy::default(),
        }
    }
}
```

### CacheConfig

**Purpose**: Cache-specific configuration (TTL, size, eviction)

**Location**: `crates/nebula-credential/src/manager/config.rs`

```rust
/// Configuration for credential caching
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Enable/disable caching
    pub enabled: bool,
    
    /// Time-to-live for cache entries
    pub ttl: Option<Duration>,
    
    /// Time-to-idle (evict if not accessed)
    pub idle_timeout: Option<Duration>,
    
    /// Maximum number of cached credentials
    pub max_capacity: usize,
    
    /// Cache eviction strategy
    pub eviction_strategy: EvictionStrategy,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ttl: Some(Duration::from_secs(300)), // 5 minutes
            idle_timeout: None,
            max_capacity: 1000,
            eviction_strategy: EvictionStrategy::Lru,
        }
    }
}

/// Cache eviction strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvictionStrategy {
    /// Least-recently-used (default)
    Lru,
    /// Least-frequently-used
    Lfu,
}
```

---

## 3. Builder Pattern

### CredentialManagerBuilder

**Purpose**: Typestate builder for safe CredentialManager construction

**Location**: `crates/nebula-credential/src/manager/builder.rs`

```rust
use std::marker::PhantomData;
use std::sync::Arc;

/// Builder for CredentialManager with typestate pattern
pub struct CredentialManagerBuilder<HasStorage> {
    storage: Option<Arc<dyn StorageProvider>>,
    config: ManagerConfig,
    _marker: PhantomData<HasStorage>,
}

// Type-level flags for compile-time validation
#[doc(hidden)]
pub struct Yes;
#[doc(hidden)]
pub struct No;

impl CredentialManagerBuilder<No> {
    /// Create new builder
    pub fn new() -> Self {
        Self {
            storage: None,
            config: ManagerConfig::default(),
            _marker: PhantomData,
        }
    }
}

impl CredentialManagerBuilder<No> {
    /// Set storage provider (required)
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_credential::{CredentialManager, LocalStorageProvider};
    ///
    /// let storage = LocalStorageProvider::new("./creds.db").await?;
    /// let manager = CredentialManager::builder()
    ///     .storage(Arc::new(storage))
    ///     .build();
    /// ```
    pub fn storage(
        self,
        storage: Arc<dyn StorageProvider>,
    ) -> CredentialManagerBuilder<Yes> {
        CredentialManagerBuilder {
            storage: Some(storage),
            config: self.config,
            _marker: PhantomData,
        }
    }
}

impl<S> CredentialManagerBuilder<S> {
    /// Enable caching with TTL (optional)
    pub fn cache_ttl(mut self, ttl: Duration) -> Self {
        self.config.cache_config.get_or_insert_with(CacheConfig::default).ttl = Some(ttl);
        self.config.cache_config.as_mut().unwrap().enabled = true;
        self
    }
    
    /// Set cache maximum size (optional)
    pub fn cache_max_size(mut self, size: usize) -> Self {
        self.config.cache_config.get_or_insert_with(CacheConfig::default).max_capacity = size;
        self.config.cache_config.as_mut().unwrap().enabled = true;
        self
    }
    
    /// Set default scope for operations (optional)
    pub fn default_scope(mut self, scope: ScopeId) -> Self {
        self.config.default_scope = Some(scope);
        self
    }
    
    /// Set batch concurrency limit (optional, default: 10)
    pub fn batch_concurrency(mut self, limit: usize) -> Self {
        self.config.batch_concurrency = limit;
        self
    }
}

impl CredentialManagerBuilder<Yes> {
    /// Build CredentialManager (requires storage provider)
    pub fn build(self) -> CredentialManager {
        let storage = self.storage.unwrap(); // Safe: type system guarantees Some
        
        let cache = self.config.cache_config
            .as_ref()
            .filter(|cfg| cfg.enabled)
            .map(|cfg| Arc::new(CacheLayer::new(cfg)));
        
        CredentialManager {
            storage,
            cache,
            config: self.config,
        }
    }
}
```

**Type Safety**:
- Cannot call `.build()` on `CredentialManagerBuilder<No>` (missing storage)
- Compiler error guides user to add `.storage()` call
- Optional parameters have fluent API (method chaining)
- Zero runtime overhead (PhantomData is ZST)

---

## 4. Cache Layer

### CacheLayer

**Purpose**: Wrapper around moka::future::Cache with credential-specific logic

**Location**: `crates/nebula-credential/src/manager/cache.rs`

```rust
use moka::future::Cache;
use std::sync::atomic::{AtomicU64, Ordering};

/// In-memory credential cache with LRU + TTL eviction
pub struct CacheLayer {
    /// moka cache instance
    cache: Cache<CredentialId, Credential>,
    
    /// Cache hit counter
    hits: AtomicU64,
    
    /// Cache miss counter
    misses: AtomicU64,
    
    /// Configuration
    config: CacheConfig,
}

impl CacheLayer {
    /// Create new cache from configuration
    pub fn new(config: &CacheConfig) -> Self {
        let mut builder = Cache::builder()
            .max_capacity(config.max_capacity as u64);
        
        if let Some(ttl) = config.ttl {
            builder = builder.time_to_live(ttl);
        }
        
        if let Some(idle) = config.idle_timeout {
            builder = builder.time_to_idle(idle);
        }
        
        Self {
            cache: builder.build(),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            config: config.clone(),
        }
    }
    
    /// Get credential from cache (increments hit/miss counters)
    pub async fn get(&self, id: &CredentialId) -> Option<Credential> {
        match self.cache.get(id).await {
            Some(credential) => {
                self.hits.fetch_add(1, Ordering::Relaxed);
                Some(credential)
            }
            None => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }
    
    /// Insert credential into cache
    pub async fn insert(&self, id: CredentialId, credential: Credential) {
        self.cache.insert(id, credential).await;
    }
    
    /// Invalidate single entry
    pub async fn invalidate(&self, id: &CredentialId) {
        self.cache.invalidate(id).await;
    }
    
    /// Invalidate all entries
    pub async fn invalidate_all(&self) {
        self.cache.invalidate_all();
    }
    
    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            size: self.cache.entry_count(),
            max_capacity: self.config.max_capacity,
        }
    }
}
```

---

## 5. Supporting Types

### ValidationResult

**Purpose**: Result of credential validation

**Location**: `crates/nebula-credential/src/manager/validation.rs`

```rust
use chrono::{DateTime, Utc};

/// Result of credential validation
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationResult {
    /// Credential identifier
    pub credential_id: CredentialId,
    
    /// Is credential valid?
    pub valid: bool,
    
    /// Validation details
    pub details: ValidationDetails,
}

/// Detailed validation information
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationDetails {
    /// Credential is valid
    Valid {
        expires_at: Option<DateTime<Utc>>,
    },
    
    /// Credential expired
    Expired {
        expired_at: DateTime<Utc>,
        now: DateTime<Utc>,
    },
    
    /// Credential not found
    NotFound,
    
    /// Credential malformed
    Invalid {
        reason: String,
    },
}

impl ValidationResult {
    /// Check if rotation recommended based on age
    pub fn rotation_recommended(&self, max_age: Duration) -> bool {
        match &self.details {
            ValidationDetails::Valid { expires_at: Some(exp) } => {
                let remaining = *exp - Utc::now();
                remaining < max_age / 4 // Rotate when 25% lifetime remaining
            }
            _ => false,
        }
    }
}
```

### CacheStats

**Purpose**: Cache performance metrics

**Location**: `crates/nebula-credential/src/manager/cache.rs`

```rust
/// Cache performance statistics
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    /// Total cache hits
    pub hits: u64,
    
    /// Total cache misses
    pub misses: u64,
    
    /// Current cache size (number of entries)
    pub size: u64,
    
    /// Maximum cache capacity
    pub max_capacity: usize,
}

impl CacheStats {
    /// Calculate cache hit rate (0.0 to 1.0)
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
    
    /// Check if cache is full
    pub fn is_full(&self) -> bool {
        self.size >= self.max_capacity as u64
    }
    
    /// Calculate cache utilization percentage
    pub fn utilization(&self) -> f64 {
        self.size as f64 / self.max_capacity as f64
    }
}
```

---

## 6. Error Types

### ManagerError

**Purpose**: Isolated error type for credential manager operations

**Location**: `crates/nebula-credential/src/core/error.rs` (add variants to existing CredentialError)

```rust
use thiserror::Error;

/// Errors from credential manager operations
#[derive(Debug, Error)]
pub enum ManagerError {
    /// Credential not found in storage
    #[error("Credential not found: {credential_id}")]
    NotFound {
        credential_id: CredentialId,
    },
    
    /// Storage provider error
    #[error("Storage error for credential {credential_id}: {source}")]
    StorageError {
        credential_id: CredentialId,
        #[source]
        source: StorageError,
    },
    
    /// Cache operation error
    #[error("Cache error: {0}")]
    CacheError(String),
    
    /// Credential validation failed
    #[error("Validation failed for {credential_id}: {reason}")]
    ValidationError {
        credential_id: CredentialId,
        reason: String,
    },
    
    /// Scope isolation violation
    #[error("Scope violation: credential {credential_id} in scope {actual_scope}, requested {requested_scope}")]
    ScopeViolation {
        credential_id: CredentialId,
        actual_scope: ScopeId,
        requested_scope: ScopeId,
    },
    
    /// Batch operation partial failure
    #[error("Batch operation failed: {successful} succeeded, {failed} failed")]
    BatchError {
        successful: usize,
        failed: usize,
        errors: Vec<(CredentialId, Box<ManagerError>)>,
    },
}

/// Result type for manager operations
pub type ManagerResult<T> = Result<T, ManagerError>;

impl ManagerError {
    /// Add credential_id context to error
    pub fn with_credential_id(self, id: CredentialId) -> Self {
        match self {
            Self::StorageError { source, .. } => {
                Self::StorageError {
                    credential_id: id,
                    source,
                }
            }
            other => other,
        }
    }
}
```

---

## 7. Type Relationships

```
CredentialManager
├── storage: Arc<dyn StorageProvider> ───► Phase 2 providers
│   ├── LocalStorageProvider
│   ├── AwsSecretsManagerProvider
│   ├── HashiCorpVaultProvider
│   └── KubernetesSecretsProvider
│
├── cache: Option<Arc<CacheLayer>> ───► moka::future::Cache wrapper
│   └── CacheConfig ───► TTL, max size, eviction strategy
│
└── config: ManagerConfig
    ├── batch_concurrency: usize
    ├── default_scope: Option<ScopeId> ───► Phase 1 core type
    └── retry_policy: RetryPolicy ───► Phase 2 utility

CredentialManagerBuilder<HasStorage>
├── storage: Option<Arc<dyn StorageProvider>>
├── config: ManagerConfig
└── _marker: PhantomData<HasStorage> ───► Compile-time type safety
```

---

## Validation Rules

### CredentialId
- MUST be unique within a storage provider
- Format: UUID v4 or custom string (user-defined)
- Cannot be empty string

### ScopeId
- Format: `{level}:{value}/{level}:{value}/...`
- Each level MUST contain colon separator
- No leading/trailing slashes
- No empty levels
- Example: `"org:acme/team:eng/service:api"`

### CacheConfig
- `max_capacity` MUST be > 0
- `ttl` SHOULD be reasonable (1 second - 1 hour recommended)
- `idle_timeout` SHOULD be >= `ttl` if both set
- `enabled = false` disables all caching (config ignored)

### ManagerConfig
- `batch_concurrency` MUST be > 0 and <= 100 (prevent overwhelming backends)
- `default_scope` optional (None = no default scope)

---

## State Transitions

### Cache Entry Lifecycle

```
┌─────────┐
│ Missing │
└────┬────┘
     │ insert()
     ▼
┌─────────┐     TTL expires
│ Cached  │────────────────►┌──────────┐
└────┬────┘                 │ Evicted  │
     │                      └──────────┘
     │ access()
     │ (reset TTL)
     ▼
┌─────────┐
│ Cached  │
└─────────┘
```

### Credential Lifecycle (via Manager)

```
                ┌──────────┐
                │ Not      │
                │ Exists   │
                └────┬─────┘
                     │ store()
                     ▼
                ┌──────────┐
                │ Stored   │◄───┐
                │ (Active) │    │ update (re-store)
                └────┬─────┘    │
                     │          │
         ┌───────────┼──────────┘
         │           │
         │ retrieve()│ delete()
         ▼           ▼
    ┌─────────┐ ┌──────────┐
    │ Cached  │ │ Deleted  │
    └─────────┘ └──────────┘
```

---

## Performance Characteristics

| Operation | Cache Hit | Cache Miss | No Cache |
|-----------|-----------|------------|----------|
| `retrieve()` | <5ms | <100ms | <100ms |
| `store()` | N/A (invalidate) | N/A | <100ms |
| `delete()` | N/A (invalidate) | N/A | <50ms |
| `list()` | N/A (no cache) | N/A | <200ms |
| `retrieve_batch(100)` | <50ms (all hits) | <1000ms | <1000ms |

**Cache Memory Usage**:
- Per entry: ~32 bytes overhead + credential size
- Example: 1000 credentials × 1KB each = ~1MB total
- LRU eviction prevents unbounded growth

**Concurrency**:
- Cache: Lock-free reads, optimistic locking on writes
- Storage: Depends on provider (file locks for LocalStorage, connection pools for cloud)
- Batch operations: Bounded at `batch_concurrency` (default 10 concurrent)

---

## Summary

**Total New Types**: 8
- `CredentialManager` - Core manager struct
- `ManagerConfig` - Configuration
- `CacheConfig` - Cache configuration
- `CredentialManagerBuilder<S>` - Type-safe builder
- `CacheLayer` - moka wrapper
- `ValidationResult` - Validation outcome
- `CacheStats` - Performance metrics
- `ManagerError` - Error variants

**Reused Types from Phase 1**: 5
- `CredentialId` - Unique identifier
- `ScopeId` - Multi-tenant scope
- `Credential` - Credential data
- `CredentialMetadata` - Metadata with tags/expiration
- `CredentialContext` - Operation context

**Reused Types from Phase 2**: 2
- `StorageProvider` - Storage abstraction
- `RetryPolicy` - Retry configuration

**External Dependencies**:
- `moka::future::Cache` - LRU cache with TTL (new dependency)
- `tokio::sync::{Arc, RwLock, Semaphore}` - Concurrency primitives (existing)
- `thiserror::Error` - Error derive macro (existing)

**Files to Create**: 6
- `manager/mod.rs` - Module exports
- `manager/manager.rs` - CredentialManager implementation
- `manager/builder.rs` - Builder pattern
- `manager/config.rs` - Configuration types
- `manager/cache.rs` - Cache layer
- `manager/validation.rs` - Validation logic

**Files to Modify**: 2
- `lib.rs` - Add manager to prelude
- `core/error.rs` - Add ManagerError variants

All types follow Constitution principles: type safety (newtype patterns), isolated errors (ManagerError in this crate), observability (tracing in all operations), and simplicity (single manager struct with optional caching).
