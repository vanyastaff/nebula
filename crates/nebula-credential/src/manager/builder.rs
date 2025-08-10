use crate::traits::{StateStore, DistributedLock, TokenCache};
use crate::manager::{CredentialManager, RefreshPolicy};
use crate::registry::CredentialRegistry;
use dashmap::DashMap;
use std::sync::Arc;

/// Builder for CredentialManager
pub struct ManagerBuilder {
    store: Option<Arc<dyn StateStore>>,
    lock: Option<Arc<dyn DistributedLock>>,
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
    pub fn with_lock(mut self, lock: Arc<dyn DistributedLock>) -> Self {
        self.lock = Some(lock);
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
        Ok(CredentialManager {
            store: self.store
                .ok_or_else(|| anyhow::anyhow!("StateStore is required"))?,
            lock: self.lock
                .ok_or_else(|| anyhow::anyhow!("DistributedLock is required"))?,
            cache: self.cache,
            policy: self.policy,
            registry: self.registry
                .unwrap_or_else(|| Arc::new(CredentialRegistry::new())),
            negative_cache: Arc::new(DashMap::new()),
        })
    }
}

impl Default for ManagerBuilder {
    fn default() -> Self {
        Self::new()
    }
}