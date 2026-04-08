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

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use nebula_action::capability::ResourceAccessor;
use nebula_action::error::ActionError;
use nebula_core::ResourceKey;

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

#[async_trait]
impl ResourceAccessor for EngineResourceAccessor {
    /// Acquire the managed resource registered under `key`.
    ///
    /// Returns the type-erased `Arc<dyn AnyManagedResource>` boxed as
    /// `Box<dyn Any + Send + Sync>`. Action code can downcast to
    /// `Arc<nebula_resource::ManagedResource<R>>` for typed access.
    ///
    /// # Errors
    ///
    /// - [`ActionError::Fatal`] if `key` is not a valid [`ResourceKey`] format.
    /// - [`ActionError::Fatal`] if no resource with the given key is registered
    ///   in the global scope.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel-safe — it performs no async I/O.
    async fn acquire(&self, key: &str) -> Result<Box<dyn Any + Send + Sync>, ActionError> {
        let resource_key = ResourceKey::new(key)
            .map_err(|e| ActionError::fatal(format!("invalid resource key {key:?}: {e}")))?;

        let any_managed = self
            .manager
            .get_any(&resource_key, &nebula_resource::ScopeLevel::Global)
            .ok_or_else(|| ActionError::fatal(format!("resource not found: {key}")))?;

        Ok(Box::new(any_managed.as_any_arc()))
    }

    /// Check whether a resource with the given key is registered in the global scope.
    ///
    /// Delegates to [`acquire`](Self::acquire) for consistency — a resource exists if
    /// and only if it can be acquired from the global scope. Returns `false` for invalid
    /// key formats or unregistered resources.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel-safe — it performs no async I/O.
    async fn exists(&self, key: &str) -> bool {
        self.acquire(key).await.is_ok()
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

    #[tokio::test]
    async fn exists_returns_false_for_unregistered_key() {
        let accessor = make_accessor();
        assert!(!accessor.exists("postgres").await);
    }

    #[tokio::test]
    async fn exists_returns_false_for_invalid_key_format() {
        let accessor = make_accessor();
        // Keys with spaces or invalid characters are rejected.
        assert!(!accessor.exists("invalid key!").await);
    }

    #[tokio::test]
    async fn acquire_returns_fatal_for_unregistered_key() {
        let accessor = make_accessor();
        let result = accessor.acquire("postgres").await;
        assert!(
            matches!(result, Err(ActionError::Fatal { .. })),
            "expected Fatal for unregistered key, got {result:?}"
        );
    }

    #[tokio::test]
    async fn acquire_returns_fatal_for_invalid_key_format() {
        let accessor = make_accessor();
        let result = accessor.acquire("invalid key!").await;
        assert!(
            matches!(result, Err(ActionError::Fatal { .. })),
            "expected Fatal for invalid key format, got {result:?}"
        );
    }

    #[tokio::test]
    async fn debug_redacts_manager() {
        let accessor = make_accessor();
        let debug = format!("{accessor:?}");
        assert!(debug.contains("<Manager>"));
    }
}
