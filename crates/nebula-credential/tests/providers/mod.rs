//! Provider implementation tests
//!
//! Tests for storage provider implementations:
//! - MockStorageProvider - In-memory test provider
//! - LocalStorageProvider - Filesystem-based provider
//! - AwsSecretsManagerProvider - AWS Secrets Manager provider (unit + integration)
//! - HashiCorpVaultProvider - HashiCorp Vault provider (integration)

mod local_provider_tests;
mod mock_provider_tests;

// AWS provider tests
#[cfg(feature = "storage-aws")]
mod aws_tests; // Unit tests

#[cfg(feature = "storage-aws")]
mod aws_integration_tests; // Integration tests with LocalStack (requires Docker)

// Vault provider tests
#[cfg(feature = "storage-vault")]
mod vault_integration_tests; // Integration tests with HashiCorp Vault (requires Docker)
