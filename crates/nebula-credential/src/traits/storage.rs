use crate::core::CredentialError;
use async_trait::async_trait;
use serde_json::Value;

/// Version for CAS operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateVersion(pub u64);

/// Trait for persistent state storage
#[async_trait]
pub trait StateStore: Send + Sync {
    /// Load state by ID
    async fn load(&self, id: &str) -> Result<(Value, StateVersion), CredentialError>;

    /// Save state with CAS
    async fn save(
        &self,
        id: &str,
        version: StateVersion,
        state: &Value,
    ) -> Result<StateVersion, CredentialError>;

    /// Delete state
    async fn delete(&self, id: &str) -> Result<(), CredentialError>;

    /// Check if state exists
    async fn exists(&self, id: &str) -> Result<bool, CredentialError>;

    /// List all credential IDs
    async fn list(&self) -> Result<Vec<String>, CredentialError>;
}
