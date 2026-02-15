//! Core resource traits and types (bb8-style)
//!
//! The `Resource` trait defines how to create, validate, recycle, and clean up
//! resource instances. `ResourceGuard` provides RAII-based lifecycle management.

use async_trait::async_trait;

use crate::context::ResourceContext;
use crate::error::ResourceResult;

/// Configuration trait for resource types.
///
/// Implementations must be deserializable so the manager can construct them
/// from JSON config blobs.
pub trait ResourceConfig: Send + Sync + serde::de::DeserializeOwned + 'static {
    /// Validate the configuration, returning an error if invalid.
    fn validate(&self) -> ResourceResult<()> {
        Ok(())
    }
}

/// Core resource trait (bb8-style).
///
/// Defines the full lifecycle: create, validate, recycle, cleanup.
/// Each resource type has an associated `Config` and `Instance`.
#[async_trait]
pub trait Resource: Send + Sync + 'static {
    /// The configuration type for this resource.
    type Config: ResourceConfig;

    /// The instance type produced by this resource.
    type Instance: Send + Sync + 'static;

    /// Unique string identifier for this resource type (e.g. "postgres", "redis").
    fn id(&self) -> &str;

    /// Create a new instance from config and context.
    async fn create(
        &self,
        config: &Self::Config,
        ctx: &ResourceContext,
    ) -> ResourceResult<Self::Instance>;

    /// Check whether an existing instance is still valid/healthy.
    async fn is_valid(&self, _instance: &Self::Instance) -> ResourceResult<bool> {
        Ok(true)
    }

    /// Recycle an instance before returning it to the pool.
    async fn recycle(&self, _instance: &mut Self::Instance) -> ResourceResult<()> {
        Ok(())
    }

    /// Clean up an instance when it is permanently removed.
    async fn cleanup(&self, instance: Self::Instance) -> ResourceResult<()> {
        drop(instance);
        Ok(())
    }

    /// List resource IDs that this resource depends on.
    fn dependencies(&self) -> Vec<&str> {
        Vec::new()
    }
}

/// RAII guard that wraps a resource instance.
///
/// When the guard is dropped, the on-drop callback is invoked (typically
/// returning the instance to the pool). Use `into_inner()` to take
/// ownership without triggering the callback.
pub struct ResourceGuard<T> {
    resource: Option<T>,
    on_drop: Option<Box<dyn FnOnce(T) + Send>>,
}

impl<T> ResourceGuard<T> {
    /// Create a new guard wrapping `resource` with a drop callback.
    pub fn new<F>(resource: T, on_drop: F) -> Self
    where
        F: FnOnce(T) + Send + 'static,
    {
        Self {
            resource: Some(resource),
            on_drop: Some(Box::new(on_drop)),
        }
    }

    /// Take the resource out of the guard, preventing the drop callback.
    #[must_use]
    pub fn into_inner(mut self) -> T {
        self.on_drop.take(); // prevent callback
        self.resource.take().expect("guard used after into_inner")
    }
}

impl<T> std::ops::Deref for ResourceGuard<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.resource.as_ref().expect("guard used after into_inner")
    }
}

impl<T> std::ops::DerefMut for ResourceGuard<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.resource.as_mut().expect("guard used after into_inner")
    }
}

impl<T> Drop for ResourceGuard<T> {
    fn drop(&mut self) {
        if let (Some(resource), Some(on_drop)) = (self.resource.take(), self.on_drop.take()) {
            on_drop(resource);
        }
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for ResourceGuard<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceGuard")
            .field("resource", &self.resource)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn guard_deref() {
        let guard = ResourceGuard::new(42u32, |_| {});
        assert_eq!(*guard, 42);
    }

    #[test]
    fn guard_drop_fires_callback() {
        let called = Arc::new(AtomicBool::new(false));
        let called_c = called.clone();
        let guard = ResourceGuard::new("hello", move |_| {
            called_c.store(true, Ordering::SeqCst);
        });
        assert!(!called.load(Ordering::SeqCst));
        drop(guard);
        assert!(called.load(Ordering::SeqCst));
    }

    #[test]
    fn guard_into_inner_prevents_callback() {
        let called = Arc::new(AtomicBool::new(false));
        let called_c = called.clone();
        let guard = ResourceGuard::new(99u32, move |_| {
            called_c.store(true, Ordering::SeqCst);
        });
        let val = guard.into_inner();
        assert_eq!(val, 99);
        assert!(!called.load(Ordering::SeqCst));
    }

    #[test]
    fn guard_deref_mut() {
        let mut guard = ResourceGuard::new(String::from("hello"), |_| {});
        guard.push_str(" world");
        assert_eq!(*guard, "hello world");
    }
}
