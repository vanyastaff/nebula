//! Helper functions for testing

use crate::core::*;
use crate::manager::*;
use crate::registry::*;
use std::sync::Arc;
use std::time::Duration;
use crate::testing::{MockLock, MockStateStore, MockTokenCache};

/// Create a test manager with mock components
pub async fn test_manager() -> CredentialManager {
    let store = Arc::new(MockStateStore::new());
    let lock = MockLock::new();
    let cache = Arc::new(MockTokenCache::new());
    let registry = Arc::new(CredentialRegistry::new());

    // Register test credential
    register_credential!(registry, TestCredential);

    CredentialManager::builder()
        .with_store(store)
        .with_lock(lock)
        .with_cache(cache)
        .with_registry(registry)
        .build()
        .expect("Failed to build test manager")
}

/// Create a manager without cache
pub async fn test_manager_no_cache() -> CredentialManager {
    let store = Arc::new(MockStateStore::new());
    let lock = MockLock::new();
    let registry = Arc::new(CredentialRegistry::new());

    register_credential!(registry, TestCredential);

    CredentialManager::builder()
        .with_store(store)
        .with_lock(lock)
        .with_registry(registry)
        .build()
        .expect("Failed to build test manager")
}

/// Run a test with timeout
pub async fn with_timeout<F, Fut, T>(duration: Duration, f: F) -> T
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    tokio::time::timeout(duration, f())
        .await
        .expect("Test timed out")
}

/// Create test credential and return its ID
pub async fn create_test_credential(manager: &CredentialManager) -> Result<CredentialId> {
    let input = serde_json::json!({
        "value": "test-value",
        "should_fail": false
    });

    manager.create_credential("test_credential", input).await
}