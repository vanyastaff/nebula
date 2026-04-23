//! Engine-side [`ResourceAccessor`] implementation.
//!
//! [`EngineResourceAccessor`] bridges the engine's resource manager to the
//! [`ResourceAccessor`] capability trait consumed by actions. It delegates
//! key-based lookups directly to the [`nebula_resource::Manager`].
//!
//! # Design
//!
//! The `ResourceAccessor` trait operates on string keys (`&str`), while the
//! resource manager uses typed [`ResourceKey`] values. The accessor parses
//! the string key at call time and returns a [`ActionError::Fatal`] if the
//! key is not a valid resource key format.
//!
//! Acquire returns the underlying `Arc<dyn AnyManagedResource>` boxed as
//! `Box<dyn Any + Send + Sync>`. Action code can downcast to the concrete
//! `Arc<ManagedResource<R>>` to access the resource's typed state, or use
//! the accessor only as an existence check before performing a typed acquire
//! via the manager directly.
//!
//! No allowlist is enforced — unlike credentials, resources are identified
//! by their registered key and any registered key may be acquired.

use std::{any::Any, fmt, future::Future, pin::Pin, sync::Arc};

use nebula_core::{CoreError, ResourceKey, accessor::ResourceAccessor};

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Engine-side implementation of [`ResourceAccessor`].
///
/// Wraps an [`Arc<nebula_resource::Manager>`] and delegates `acquire` and
/// `exists` calls to it via key-based lookup.
///
/// # Examples
///
/// ```rust,ignore
/// use std::sync::Arc;
/// use nebula_engine::resource_accessor::EngineResourceAccessor;
/// use nebula_resource::Manager;
///
/// let manager = Arc::new(Manager::new());
/// let accessor = EngineResourceAccessor::new(manager);
/// ```
pub struct EngineResourceAccessor {
    manager: Arc<nebula_resource::Manager>,
}

impl EngineResourceAccessor {
    /// Creates a new accessor backed by the given resource manager.
    #[must_use]
    pub fn new(manager: Arc<nebula_resource::Manager>) -> Self {
        Self { manager }
    }
}

impl fmt::Debug for EngineResourceAccessor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EngineResourceAccessor")
            .field("manager", &"<Manager>")
            .finish()
    }
}

impl ResourceAccessor for EngineResourceAccessor {
    fn has(&self, key: &ResourceKey) -> bool {
        self.manager
            .get_any(key, &nebula_resource::ScopeLevel::Global)
            .is_some()
    }

    fn acquire_any(
        &self,
        key: &ResourceKey,
    ) -> BoxFut<'_, Result<Box<dyn Any + Send + Sync>, CoreError>> {
        let any_managed = self
            .manager
            .get_any(key, &nebula_resource::ScopeLevel::Global);
        let key_str = key.as_str().to_owned();
        Box::pin(async move {
            match any_managed {
                Some(m) => Ok(Box::new(m.as_any_arc()) as Box<dyn Any + Send + Sync>),
                None => Err(CoreError::CredentialNotFound { key: key_str }),
            }
        })
    }

    fn try_acquire_any(
        &self,
        key: &ResourceKey,
    ) -> BoxFut<'_, Result<Option<Box<dyn Any + Send + Sync>>, CoreError>> {
        let any_managed = self
            .manager
            .get_any(key, &nebula_resource::ScopeLevel::Global);
        Box::pin(async move {
            Ok(any_managed.map(|m| Box::new(m.as_any_arc()) as Box<dyn Any + Send + Sync>))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nebula_resource::Manager;

    use super::*;

    fn make_accessor() -> EngineResourceAccessor {
        EngineResourceAccessor::new(Arc::new(Manager::new()))
    }

    fn rk(key: &str) -> ResourceKey {
        ResourceKey::new(key).expect("valid resource key in test")
    }

    #[tokio::test]
    async fn has_returns_false_for_unregistered_key() {
        let accessor = make_accessor();
        assert!(!accessor.has(&rk("postgres")));
    }

    #[tokio::test]
    async fn acquire_any_returns_err_for_unregistered_key() {
        let accessor = make_accessor();
        let result = accessor.acquire_any(&rk("postgres")).await;
        assert!(
            matches!(result, Err(CoreError::CredentialNotFound { .. })),
            "expected CredentialNotFound, got {result:?}"
        );
    }

    #[tokio::test]
    async fn try_acquire_any_returns_none_for_unregistered_key() {
        let accessor = make_accessor();
        let result = accessor.try_acquire_any(&rk("postgres")).await;
        assert!(matches!(result, Ok(None)));
    }

    #[tokio::test]
    async fn debug_redacts_manager() {
        // `Manager::new()` spawns a release queue worker that needs a Tokio
        // runtime — keep the assertion but run under #[tokio::test].
        let accessor = make_accessor();
        let debug = format!("{accessor:?}");
        assert!(debug.contains("<Manager>"));
    }
}
