use crate::core::{AccessToken, CredentialContext, CredentialError, CredentialMetadata};
use crate::traits::{bridge::CredentialAdapter, Credential};
use async_trait::async_trait;
use dashmap::DashMap;
use serde::de::DeserializeOwned;
use serde::Serialize;
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

    /// Get metadata about this credential type (optional)
    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata {
            id: self.type_name(),
            name: self.type_name(),
            description: "",
            supports_refresh: true,
            requires_interaction: false,
        }
    }
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

    /// Register a Credential directly (auto-wraps in CredentialAdapter)
    ///
    /// This is a convenience method for registering type-safe `Credential` implementations.
    /// The credential is automatically wrapped in `CredentialAdapter` to make it compatible
    /// with the factory registry.
    ///
    /// # Example
    /// ```ignore
    /// use nebula_credential::{CredentialRegistry, Credential};
    ///
    /// struct MyCredential;
    /// impl Credential for MyCredential { /* ... */ }
    ///
    /// let registry = CredentialRegistry::new();
    /// registry.register_credential(MyCredential);
    /// ```
    pub fn register_credential<C>(&self, credential: C)
    where
        C: Credential,
        C::Input: Serialize + DeserializeOwned + Send + Sync + 'static,
        C::State: Serialize + DeserializeOwned + Send + Sync + 'static,
    {
        let adapter = CredentialAdapter::new(credential);
        self.register(Arc::new(adapter));
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

    /// Get metadata for all registered credential types
    pub fn list_metadata(&self) -> Vec<CredentialMetadata> {
        self.factories
            .iter()
            .map(|entry| entry.value().metadata())
            .collect()
    }

    /// Get metadata for a specific credential type
    pub fn get_metadata(&self, type_name: &str) -> Option<CredentialMetadata> {
        self.factories.get(type_name).map(|f| f.metadata())
    }
}

impl Default for CredentialRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock factory for testing
    struct MockFactory {
        type_name: &'static str,
    }

    #[async_trait]
    impl CredentialFactory for MockFactory {
        fn type_name(&self) -> &'static str {
            self.type_name
        }

        async fn create_and_init(
            &self,
            _input_json: Value,
            _cx: &mut CredentialContext,
        ) -> Result<(Box<dyn erased_serde::Serialize>, Option<AccessToken>), CredentialError>
        {
            Err(CredentialError::Internal("mock factory".to_string()))
        }

        async fn refresh(
            &self,
            _state_json: Value,
            _cx: &mut CredentialContext,
        ) -> Result<(Box<dyn erased_serde::Serialize>, AccessToken), CredentialError> {
            Err(CredentialError::Internal("mock factory".to_string()))
        }
    }

    #[test]
    fn test_registry_creation() {
        let registry = CredentialRegistry::new();
        assert_eq!(registry.list_types().len(), 0);
    }

    #[test]
    fn test_registry_default() {
        let registry = CredentialRegistry::default();
        assert_eq!(registry.list_types().len(), 0);
    }

    #[test]
    fn test_registry_register_factory() {
        let registry = CredentialRegistry::new();
        let factory = Arc::new(MockFactory {
            type_name: "test_type",
        });

        registry.register(factory);
        assert_eq!(registry.list_types().len(), 1);
        assert!(registry.has_type("test_type"));
    }

    #[test]
    fn test_registry_get_factory_success() {
        let registry = CredentialRegistry::new();
        let factory = Arc::new(MockFactory {
            type_name: "oauth2",
        });

        registry.register(factory.clone());
        let retrieved = registry.get("oauth2");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().type_name(), "oauth2");
    }

    #[test]
    fn test_registry_get_factory_not_found() {
        let registry = CredentialRegistry::new();
        let retrieved = registry.get("nonexistent");
        assert!(retrieved.is_none());
    }

    #[test]
    fn test_registry_has_type() {
        let registry = CredentialRegistry::new();
        let factory = Arc::new(MockFactory { type_name: "api_key" });

        assert!(!registry.has_type("api_key"));
        registry.register(factory);
        assert!(registry.has_type("api_key"));
    }

    #[test]
    fn test_registry_list_types() {
        let registry = CredentialRegistry::new();

        registry.register(Arc::new(MockFactory {
            type_name: "type_a",
        }));
        registry.register(Arc::new(MockFactory {
            type_name: "type_b",
        }));
        registry.register(Arc::new(MockFactory {
            type_name: "type_c",
        }));

        let types = registry.list_types();
        assert_eq!(types.len(), 3);
        assert!(types.contains(&"type_a"));
        assert!(types.contains(&"type_b"));
        assert!(types.contains(&"type_c"));
    }

    #[test]
    fn test_registry_multiple_registrations_same_type() {
        let registry = CredentialRegistry::new();

        let factory1 = Arc::new(MockFactory {
            type_name: "duplicate",
        });
        let factory2 = Arc::new(MockFactory {
            type_name: "duplicate",
        });

        registry.register(factory1);
        registry.register(factory2); // Should overwrite

        let types = registry.list_types();
        assert_eq!(types.len(), 1); // Should only have one entry
    }

    #[test]
    fn test_registry_concurrent_access() {
        use std::thread;

        let registry = Arc::new(CredentialRegistry::new());
        let mut handles = vec![];

        for i in 0..10 {
            let registry = Arc::clone(&registry);
            let handle = thread::spawn(move || {
                let factory = Arc::new(MockFactory {
                    type_name: match i {
                        0..=3 => "type_a",
                        4..=6 => "type_b",
                        _ => "type_c",
                    },
                });
                registry.register(factory);
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let types = registry.list_types();
        assert_eq!(types.len(), 3); // Should have 3 unique types
    }

    #[test]
    fn test_registry_empty_list() {
        let registry = CredentialRegistry::new();
        let types = registry.list_types();
        assert!(types.is_empty());
    }
}
