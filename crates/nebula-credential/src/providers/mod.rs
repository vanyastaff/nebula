//! Storage provider implementations for credential persistence
//!
//! This module contains all storage backend implementations that implement
//! the `StorageProvider` trait from Phase 1.

// Mock provider for testing (always available)
pub mod mock;

// Local filesystem storage (default feature)
#[cfg(feature = "storage-local")]
pub mod local;

// AWS Secrets Manager provider
#[cfg(feature = "storage-aws")]
pub mod aws;

// Azure Key Vault provider - SKIPPED (Phase 5)
// #[cfg(feature = "storage-azure")]
// pub mod azure;

// HashiCorp Vault provider
#[cfg(feature = "storage-vault")]
pub mod vault;

// Kubernetes Secrets provider
#[cfg(feature = "storage-k8s")]
pub mod kubernetes;

// Provider configuration and metrics
pub mod config;
pub mod metrics;

// Re-exports
pub use mock::MockStorageProvider;

#[cfg(feature = "storage-local")]
pub use local::{LocalStorageConfig, LocalStorageProvider};

#[cfg(feature = "storage-aws")]
pub use aws::{AwsSecretsManagerConfig, AwsSecretsManagerProvider};

#[cfg(feature = "storage-vault")]
pub use vault::{HashiCorpVaultProvider, VaultAuthMethod, VaultConfig};

#[cfg(feature = "storage-k8s")]
pub use kubernetes::{KubernetesSecretsConfig, KubernetesSecretsProvider};

pub use config::{ConfigError, ProviderConfig};
pub use metrics::StorageMetrics;
