//! Resource provider trait for decoupled resource acquisition.
//!
//! Provides a common provider interface that decouples resource acquisition
//! from the concrete Manager implementation.

use std::future::Future;

use crate::{Context, Guard, Resource, Result};
use nebula_core::ResourceKey;

/// Type-erased resource reference — used inside manager internals.
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
/// use nebula_resource::{ResourceProvider, Context, Result, Guard, Resource};
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
