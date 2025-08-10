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

pub mod core;
pub mod traits;
pub mod authenticator;
pub mod manager;
pub mod registry;
pub mod migration;
mod testing;

/// Commonly used types and traits
pub mod prelude {
    pub use crate::core::{
        AccessToken, CredentialError, CredentialContext, CredentialMetadata,
        SecureString, CredentialState, Ephemeral,
    };
    pub use crate::traits::{Credential, StateStore, TokenCache, DistributedLock, LockGuard, LockError};
    pub use crate::authenticator::{ClientAuthenticator, ChainAuthenticator};
    pub use crate::manager::{CredentialManager, ManagerBuilder, RefreshPolicy};
    pub use async_trait::async_trait;
    pub use serde::{Serialize, Deserialize};
}

// Re-export commonly used external types
pub use uuid::Uuid;
pub use chrono::{DateTime, Utc};