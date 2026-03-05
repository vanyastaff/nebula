//! Resource references and provider traits.
//!
//! Provides type-safe references to resources and a common provider interface
//! that decouples resource acquisition from the concrete Manager implementation.

use std::future::Future;
use std::marker::PhantomData;

use crate::{Context, Guard, Resource, Result};
use nebula_core::ResourceKey;

/// Typed reference to a specific resource instance in the registry.
///
/// Carries the `ResourceKey` (string id) plus compile-time type information.
/// Use `erase()` to store in collections or when wiring dependency graphs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRef<R: Resource> {
    /// Stable, normalized key for this resource instance.
    pub key: ResourceKey,
    _phantom: PhantomData<fn() -> R>,
}

impl<R: Resource> ResourceRef<R> {
    /// Create a typed reference. Returns an error if `key` is not a valid `ResourceKey`.
    pub fn new(key: &str) -> Result<Self> {
        let key = ResourceKey::new(key).map_err(|e| crate::error::Error::Configuration {
            message: format!("invalid resource key: {e}"),
            source: None,
        })?;
        Ok(Self {
            key,
            _phantom: PhantomData,
        })
    }

    /// Type-erased reference used in manager internals and components.
    pub fn erase(self) -> ErasedResourceRef {
        ErasedResourceRef { key: self.key }
    }

    /// Create a type-only reference for dependency declaration (key from `Resource::declare_key`).
    ///
    /// Use in `ActionComponents` when declaring "I need a resource of type R".
    pub fn of() -> Self
    where
        R: Resource,
    {
        Self {
            key: R::declare_key(),
            _phantom: PhantomData,
        }
    }
}

impl<R: Resource> From<ResourceRef<R>> for ErasedResourceRef {
    fn from(r: ResourceRef<R>) -> Self {
        r.erase()
    }
}

/// Type-erased resource reference — used inside `ResourceComponents` and manager internals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErasedResourceRef {
    /// Resource key in the registry.
    pub key: ResourceKey,
}

/// Provider trait for acquiring resources.
///
/// Decouples resource acquisition from the concrete [`Manager`](crate::Manager)
/// implementation. Supports both type-based and string-based resource acquisition.
///
/// # Example Implementation
///
/// ```rust,ignore
/// use nebula_resource::{ResourceProvider, ResourceRef, Context, Result, Guard, Resource};
///
/// struct MyProvider {
///     manager: Arc<Manager>,
/// }
///
/// impl ResourceProvider for MyProvider {
///     // Type-safe acquisition by Resource type
///     async fn resource<R: Resource>(&self, ctx: &Context) -> Result<Guard<R::Instance>> {
///         self.manager.acquire::<R>(ctx).await
///     }
///
///     // Dynamic acquisition by string ID
///     async fn acquire(&self, id: &str, ctx: &Context) -> Result<Box<dyn std::any::Any + Send>> {
///         self.manager.acquire_any(id, ctx).await
///     }
/// }
/// ```
pub trait ResourceProvider: Send + Sync {
    /// Acquire a typed resource instance by Resource type.
    ///
    /// Type-safe method that uses the Resource trait to identify and acquire
    /// the resource. The returned instance type matches `R::Instance`.
    fn resource<R: Resource>(
        &self,
        ctx: &Context,
    ) -> impl Future<Output = Result<Guard<R::Instance>>> + Send;

    /// Acquire a resource instance by string ID (type-erased).
    ///
    /// Returns a type-erased resource that must be downcast to the expected type.
    /// Use this when the resource type is not known at compile time.
    fn acquire(
        &self,
        id: &str,
        ctx: &Context,
    ) -> impl Future<Output = Result<Box<dyn std::any::Any + Send>>> + Send;

    /// Check if a resource exists by type.
    ///
    /// Default implementation uses a full acquire; override for lightweight checks.
    fn has_resource<R: Resource>(&self, ctx: &Context) -> impl Future<Output = bool> + Send {
        async move { self.resource::<R>(ctx).await.is_ok() }
    }

    /// Check if a resource exists by ID.
    ///
    /// Default implementation uses a full acquire; override for lightweight checks.
    fn has(&self, id: &str, ctx: &Context) -> impl Future<Output = bool> + Send {
        async move { self.acquire(id, ctx).await.is_ok() }
    }

    /// Lightweight existence check by ID — no pool acquire.
    ///
    /// Default implementation falls back to [`has`](Self::has); override for
    /// side-effect-free checks.
    fn exists(&self, id: &str, ctx: &Context) -> impl Future<Output = bool> + Send {
        async move { self.has(id, ctx).await }
    }

    /// Lightweight typed existence check — no pool acquire.
    ///
    /// Default implementation falls back to [`has_resource`](Self::has_resource).
    fn exists_resource<R: Resource>(&self, ctx: &Context) -> impl Future<Output = bool> + Send {
        async move { self.has_resource::<R>(ctx).await }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
    use crate::error::Result;
    use crate::metadata::ResourceMetadata;
    use crate::resource::Config;

    #[derive(Clone)]
    struct MarkerConfig;

    impl Config for MarkerConfig {}

    struct MarkerResource;

    impl Resource for MarkerResource {
        type Config = MarkerConfig;
        type Instance = ();

        fn metadata(&self) -> ResourceMetadata {
            ResourceMetadata::from_key(nebula_core::ResourceKey::try_from("marker").expect("valid key"))
        }

        async fn create(&self, _config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
            Ok(())
        }
    }

    #[test]
    fn resource_ref_captures_key() {
        // ResourceRef<R> only uses Resource as a marker bound in tests;
        // we don't need a concrete Resource impl here, just validate key wiring.
        let r = ResourceRef::<MarkerResource>::new("http-global").unwrap();
        assert_eq!(r.key.as_str(), "http-global");
    }

    #[test]
    fn erase_preserves_key() {
        let r = ResourceRef::<MarkerResource>::new("http-global").unwrap();
        let erased = r.erase();
        assert_eq!(erased.key.as_str(), "http-global");
    }
}
