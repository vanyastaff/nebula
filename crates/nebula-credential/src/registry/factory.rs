use crate::core::{AccessToken, CredentialContext, CredentialError};
use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::Value;
use std::sync::Arc;

/// Factory for creating credentials
#[async_trait]
pub trait CredentialFactory: Send + Sync {
    /// Get the type name
    fn type_name(&self) -> &'static str;

    /// Create and initialize credential
    async fn create_and_init(
        &self,
        input_json: Value,
        cx: &mut CredentialContext,
    ) -> Result<(Box<dyn erased_serde::Serialize>, Option<AccessToken>), CredentialError>;

    /// Refresh existing credential
    async fn refresh(
        &self,
        state_json: Value,
        cx: &mut CredentialContext,
    ) -> Result<(Box<dyn erased_serde::Serialize>, AccessToken), CredentialError>;
}

/// Registry for credential types
pub struct CredentialRegistry {
    factories: DashMap<&'static str, Arc<dyn CredentialFactory>>,
}

impl CredentialRegistry {
    /// Create new registry
    pub fn new() -> Self {
        Self {
            factories: DashMap::new(),
        }
    }

    /// Register a credential factory
    pub fn register(&self, factory: Arc<dyn CredentialFactory>) {
        self.factories.insert(factory.type_name(), factory);
    }

    /// Get factory by type name
    pub fn get(&self, type_name: &str) -> Option<Arc<dyn CredentialFactory>> {
        self.factories.get(type_name).map(|f| f.clone())
    }

    /// List all registered types
    pub fn list_types(&self) -> Vec<&'static str> {
        self.factories.iter().map(|e| *e.key()).collect()
    }

    /// Check if type is registered
    pub fn has_type(&self, type_name: &str) -> bool {
        self.factories.contains_key(type_name)
    }
}

impl Default for CredentialRegistry {
    fn default() -> Self {
        Self::new()
    }
}