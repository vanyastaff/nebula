use crate::manager::core::AnyLock;
use crate::manager::{CredentialManager, RefreshPolicy};
use crate::registry::CredentialRegistry;
use crate::traits::{DistributedLock, StateStore, TokenCache};
use dashmap::DashMap;
use std::sync::Arc;

/// Builder for `CredentialManager`
pub struct ManagerBuilder {
    store: Option<Arc<dyn StateStore>>,
    lock: Option<AnyLock>,
    cache: Option<Arc<dyn TokenCache>>,
    policy: RefreshPolicy,
    registry: Option<Arc<CredentialRegistry>>,
}

impl ManagerBuilder {
    /// Create new builder
    pub fn new() -> Self {
        Self {
            store: None,
            lock: None,
            cache: None,
            policy: RefreshPolicy::default(),
            registry: None,
        }
    }

    /// Set state store
    pub fn with_store(mut self, store: Arc<dyn StateStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Set distributed lock
    pub fn with_lock<L: DistributedLock + 'static>(mut self, lock: L) -> Self {
        self.lock = Some(AnyLock::new(lock));
        self
    }

    /// Set token cache
    pub fn with_cache(mut self, cache: Arc<dyn TokenCache>) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Set refresh policy
    pub fn with_policy(mut self, policy: RefreshPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Set credential registry
    pub fn with_registry(mut self, registry: Arc<CredentialRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Build the manager
    pub fn build(self) -> Result<CredentialManager, anyhow::Error> {
        let store = self
            .store
            .ok_or_else(|| anyhow::anyhow!("StateStore is required"))?;
        let lock = self
            .lock
            .ok_or_else(|| anyhow::anyhow!("DistributedLock is required"))?;
        let registry = self
            .registry
            .unwrap_or_else(|| Arc::new(CredentialRegistry::new()));
        let negative_cache = Arc::new(DashMap::new());
        Ok(CredentialManager::new(
            store,
            lock,
            self.cache,
            self.policy,
            registry,
            negative_cache,
        ))
    }
}

impl Default for ManagerBuilder {
    fn default() -> Self {
        Self::new()
    }
}
