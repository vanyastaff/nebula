//! Nebula Credential Core
//!
//! A secure, extensible credential management system for Nebula.
//!
//! # Features
//!
//! - **Type-safe credential management** - Compile-time verification
//! - **Secure token storage** - Zero-copy secrets with automatic zeroization
//! - **Automatic token refresh** - With jitter and retry logic
//! - **Multi-level caching** - L1/L2 cache with TTL support
//! - **Distributed locking** - For safe concurrent operations
//! - **Pluggable authenticators** - Compose authentication strategies
//! - **State migrations** - Version-to-version upgrades
//! - **Comprehensive observability** - Metrics, tracing, and audit logs

#![warn(missing_docs)]
#![deny(unsafe_code)]
#![forbid(unsafe_code)]

/// Authentication strategies and composable authenticators
pub mod authenticator;
/// Core types, errors, and primitives
pub mod core;
/// Credential manager and refresh policies
pub mod manager;
/// State migration support
pub mod migration;
/// Credential type registry and factories
pub mod registry;
pub mod storage;
pub mod cache;
mod testing;
/// Core traits for credentials, storage, caching, and locking
pub mod traits;

/// Commonly used types and traits
pub mod prelude {
    pub use crate::authenticator::{ChainAuthenticator, ClientAuthenticator};
    pub use crate::core::{
        AccessToken, CredentialContext, CredentialError, CredentialId, CredentialMetadata,
        CredentialState, Ephemeral, SecureString,
    };
    pub use crate::manager::{CredentialManager, ManagerBuilder, RefreshPolicy};
    pub use crate::traits::{
        Credential, DistributedLock, LockError, LockGuard, StateStore, TokenCache,
    };
    pub use async_trait::async_trait;
    pub use serde::{Deserialize, Serialize};
}

// Re-export commonly used external types
pub use chrono::{DateTime, Utc};
pub use uuid::Uuid;
