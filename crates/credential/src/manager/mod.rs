//! Credential Manager - High-level API for credential operations.
//!
//! This module provides the [`crate::manager::CredentialManager`] type and supporting infrastructure
//! for CRUD operations, caching, validation, and multi-tenant credential isolation.
//!
//! # Overview
//!
//! The Credential Manager is the primary interface for interacting with the credential
//! management system. It provides:
//!
//! - **CRUD Operations**: Store, retrieve, update, delete credentials
//! - **Multi-Tenant Isolation**: Scope-based credential separation
//! - **Validation**: Credential health checks and rotation recommendations
//! - **Caching**: Optional in-memory cache for performance
//! - **Batch Operations**: Parallel operations for multiple credentials
//! - **Builder Pattern**: Fluent API for configuration
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │              CredentialManager (High-Level API)         │
//! ├─────────────────────────────────────────────────────────┤
//! │  • store()   • retrieve()   • delete()   • validate()  │
//! │  • list()    • update()     • batch operations          │
//! └─────────────────────────────────────────────────────────┘
//!                           │
//!           ┌───────────────┴───────────────┐
//!           │                               │
//! ┌─────────▼────────────┐      ┌──────────▼───────────┐
//! │   CacheLayer         │      │  ValidationDetails   │
//! │  (Optional Moka)     │      │  (Health Checks)     │
//! └──────────────────────┘      └──────────────────────┘
//!           │
//! ┌─────────▼────────────────────────────────────────────┐
//! │         StorageProvider Trait                        │
//! ├──────────────────────────────────────────────────────┤
//! │  • MockStorage  • LocalStorage  • AWS Secrets        │
//! │  • Azure KeyVault  • HashiCorp Vault  • Kubernetes   │
//! └──────────────────────────────────────────────────────┘
//! ```
//!
//! # Examples
//!
//! ## Basic CRUD Operations
//!
//! ```no_run
//! use nebula_credential::prelude::*;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create manager with storage backend
//! let storage = Arc::new(MockStorageProvider::new());
//! let manager = CredentialManager::builder()
//!     .storage(storage)
//!     .build();
//!
//! // Store credential
//! let id = CredentialId::new("api-key")?;
//! let key = EncryptionKey::from_bytes([0u8; 32]);
//! let data = encrypt(&key, b"secret-value")?;
//! let context = CredentialContext::new("user-123");
//!
//! manager.store(&id, data, CredentialMetadata::new(), &context).await?;
//!
//! // Retrieve credential
//! if let Some((data, metadata)) = manager.retrieve(&id, &context).await? {
//!     println!("Retrieved credential created at: {}", metadata.created_at);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Multi-Tenant Isolation
//!
//! ```no_run
//! # use nebula_credential::prelude::*;
//! # use std::sync::Arc;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let storage = Arc::new(MockStorageProvider::new());
//! # let manager = CredentialManager::builder().storage(storage).build();
//! // Tenant A context with scope
//! let tenant_a = CredentialContext::new("org-1")
//!     .with_scope("tenant-a")?;
//!
//! // Tenant B context with scope
//! let tenant_b = CredentialContext::new("org-1")
//!     .with_scope("tenant-b")?;
//!
//! // Credentials are isolated by scope
//! let id = CredentialId::new("db-password")?;
//! # let key = EncryptionKey::from_bytes([0u8; 32]);
//! # let data = encrypt(&key, b"secret")?;
//! manager.store(&id, data, CredentialMetadata::new(), &tenant_a).await?;
//!
//! // Tenant B cannot access Tenant A's credentials
//! assert!(manager.retrieve(&id, &tenant_b).await?.is_none());
//! # Ok(())
//! # }
//! ```
//!
//! ## Validation and Health Checks
//!
//! ```no_run
//! # use nebula_credential::prelude::*;
//! # use std::sync::Arc;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let storage = Arc::new(MockStorageProvider::new());
//! # let manager = CredentialManager::builder().storage(storage).build();
//! # let id = CredentialId::new("cred")?;
//! # let context = CredentialContext::new("user");
//! // Validate credential health
//! let result = manager.validate(&id, &context).await?;
//!
//! if result.is_valid() {
//!     println!("Credential is valid");
//! }
//!
//! // Check rotation recommendation
//! use std::time::Duration;
//! if result.rotation_recommended(Duration::from_secs(30 * 24 * 3600)) {
//!     println!("Rotation recommended");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Performance with Caching
//!
//! ```no_run
//! # use nebula_credential::prelude::*;
//! # use std::sync::Arc;
//! use std::time::Duration;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let storage = Arc::new(MockStorageProvider::new());
//!
//! // Enable caching for performance
//! let manager = CredentialManager::builder()
//!     .storage(storage)
//!     .cache_ttl(Duration::from_secs(300))      // 5 minutes
//!     .cache_max_size(1000)                      // 1000 entries
//!     .build();
//!
//! // Check cache performance
//! if let Some(stats) = manager.cache_stats() {
//!     println!("Hit rate: {:.1}%", stats.hit_rate() * 100.0);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Builder Pattern Configuration
//!
//! ```no_run
//! # use nebula_credential::prelude::*;
//! # use std::sync::Arc;
//! use std::time::Duration;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let storage = Arc::new(MockStorageProvider::new());
//!
//! // Fluent builder API
//! let manager = CredentialManager::builder()
//!     .storage(storage)
//!     .cache_ttl(Duration::from_secs(600))
//!     .cache_max_size(5000)
//!     .build();
//!
//! // Or use CacheConfig struct
//! let cache_config = CacheConfig {
//!     ttl: Duration::from_secs(600),
//!     max_size: 5000,
//!     eviction_strategy: EvictionStrategy::LRU,
//! };
//!
//! let manager = CredentialManager::builder()
//!     .storage(storage)
//!     .cache_config(cache_config)
//!     .build();
//! # Ok(())
//! # }
//! ```
//!
//! # Performance
//!
//! ## Cache Performance
//!
//! - **Cache Hit Latency**: p99 < 10ms (in-memory)
//! - **Cache Miss**: Falls through to storage provider
//! - **TTL**: Configurable per-manager (default: disabled)
//! - **Eviction**: LRU strategy with configurable max size
//!
//! ## Batch Operations
//!
//! - **Parallel Execution**: Uses `tokio::task::JoinSet` for concurrency
//! - **Performance Gain**: 50%+ improvement with I/O backends (AWS, Azure, Vault)
//! - **Mock Storage**: Minimal overhead due to in-memory operations
//! - **Error Handling**: Per-item results with partial failure support
//!
//! # Thread Safety
//!
//! All types in this module are thread-safe:
//!
//! - [`crate::manager::CredentialManager`] implements `Clone` and uses `Arc` internally
//! - Cache operations are lock-free (Moka handles concurrency)
//! - Storage providers must implement thread-safe operations
//!
//! # See Also
//!
//! - [`crate::manager::CredentialManager`]: Primary API for credential operations
//! - [`crate::manager::CredentialManagerBuilder`]: Builder for configuring managers
//! - [`crate::manager::CacheConfig`]: Cache configuration options
//! - [`crate::manager::ValidationResult`]: Validation and health check results

pub mod cache;
pub mod config;
#[allow(clippy::module_inception)]
pub mod manager;
pub mod validation;

// Re-export public types
pub use cache::{CacheLayer, CacheStats, CachedCredential};
pub use config::{CacheConfig, EvictionStrategy, ManagerConfig};
pub use manager::{CredentialManager, CredentialManagerBuilder};
pub use validation::{ValidationDetails, ValidationResult};
