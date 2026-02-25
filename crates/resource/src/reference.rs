//! Resource references and provider traits.
//!
//! Provides type-safe references to resources and a common provider interface
//! that decouples resource acquisition from the concrete Manager implementation.

use std::any::TypeId;
use std::fmt;
use std::future::Future;

use crate::{Context, Guard, Resource, Result};

/// Type-safe reference to a resource.
///
/// Wraps a `TypeId` to identify a resource type. Used to request resources
/// from providers with compile-time and runtime type safety.
///
/// # Example
/// ```rust
/// use nebula_resource::ResourceRef;
/// use std::any::TypeId;
///
/// struct PostgresConnection;
///
/// let db_ref = ResourceRef::of::<PostgresConnection>();
/// assert_eq!(db_ref.type_id(), TypeId::of::<PostgresConnection>());
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResourceRef(TypeId);

impl ResourceRef {
    /// Create a resource reference from a type.
    ///
    /// # Example
    /// ```rust
    /// use nebula_resource::ResourceRef;
    ///
    /// struct DbConnection;
    /// let db_ref = ResourceRef::of::<DbConnection>();
    /// ```
    pub const fn of<T: 'static>() -> Self {
        Self(TypeId::of::<T>())
    }

    /// Get the underlying type ID.
    pub const fn type_id(self) -> TypeId {
        self.0
    }
}

impl fmt::Debug for ResourceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ResourceRef").field(&self.0).finish()
    }
}

impl fmt::Display for ResourceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ResourceRef({:?})", self.0)
    }
}

impl<T: 'static> From<std::marker::PhantomData<T>> for ResourceRef {
    fn from(_: std::marker::PhantomData<T>) -> Self {
        Self::of::<T>()
    }
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
///         let type_id = TypeId::of::<R>();
///         self.manager.acquire_by_type(type_id, ctx).await
///     }
///
///     // Dynamic acquisition by string ID
///     async fn acquire(&self, id: &str, ctx: &Context) -> Result<Box<dyn std::any::Any + Send>> {
///         self.manager.acquire(id, ctx).await
///     }
/// }
/// ```
pub trait ResourceProvider: Send + Sync {
    /// Acquire a typed resource instance by Resource type.
    ///
    /// Type-safe method that uses the Resource trait to identify and acquire
    /// the resource. The returned instance type matches `R::Instance`.
    ///
    /// # Example
    /// ```rust,ignore
    /// // Define a resource
    /// struct PostgresResource;
    /// impl Resource for PostgresResource {
    ///     type Config = PostgresConfig;
    ///     type Instance = PgConnection;
    ///     fn id() -> &'static str { "postgres" }
    /// }
    ///
    /// // Acquire it
    /// let conn = provider.resource::<PostgresResource>(&ctx).await?;
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Unavailable`] if the resource doesn't exist or acquisition fails.
    fn resource<R: Resource>(
        &self,
        ctx: &Context,
    ) -> impl Future<Output = Result<Guard<R::Instance>>> + Send;

    /// Acquire a resource instance by string ID (type-erased).
    ///
    /// Returns a type-erased resource that must be downcast to the expected type.
    /// Use this when the resource type is not known at compile time.
    ///
    /// # Example
    /// ```rust,ignore
    /// let any = provider.acquire("postgres", &ctx).await?;
    /// let guard = any.downcast::<Guard<PgConnection>>().unwrap();
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Unavailable`] if the resource doesn't exist or acquisition fails.
    fn acquire(
        &self,
        id: &str,
        ctx: &Context,
    ) -> impl Future<Output = Result<Box<dyn std::any::Any + Send>>> + Send;

    /// Check if a resource exists by type.
    ///
    /// # Default implementation
    ///
    /// The default performs a full `resource::<R>()` acquire, which takes
    /// a connection from the pool and immediately drops it. Implementations
    /// should override this with a lightweight check (e.g.
    /// [`Manager::is_registered`](crate::Manager::is_registered)) to avoid
    /// side effects.
    fn has_resource<R: Resource>(&self, ctx: &Context) -> impl Future<Output = bool> + Send {
        async move { self.resource::<R>(ctx).await.is_ok() }
    }

    /// Check if a resource exists by ID.
    ///
    /// # Default implementation
    ///
    /// The default performs a full `acquire()`, which takes a connection
    /// from the pool and immediately drops it. Implementations should
    /// override this with a lightweight check (e.g.
    /// [`Manager::is_registered`](crate::Manager::is_registered)) to avoid
    /// side effects.
    fn has(&self, id: &str, ctx: &Context) -> impl Future<Output = bool> + Send {
        async move { self.acquire(id, ctx).await.is_ok() }
    }

    /// Lightweight existence check by ID — no pool acquire.
    ///
    /// Returns `true` if the resource is registered, regardless of
    /// health or quarantine status. Providers that wrap a
    /// [`Manager`](crate::Manager) should delegate to
    /// [`Manager::is_registered`](crate::Manager::is_registered).
    ///
    /// The default falls back to [`has`](Self::has), which does a full
    /// acquire. Override for a side-effect-free check.
    fn exists(&self, id: &str, ctx: &Context) -> impl Future<Output = bool> + Send {
        async move { self.has(id, ctx).await }
    }

    /// Lightweight typed existence check — no pool acquire.
    ///
    /// The default falls back to [`has_resource`](Self::has_resource).
    /// Override for a side-effect-free check.
    fn exists_resource<R: Resource>(&self, ctx: &Context) -> impl Future<Output = bool> + Send {
        async move { self.has_resource::<R>(ctx).await }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DbConnection;
    struct CacheConnection;

    #[test]
    fn test_resource_ref_creation() {
        let db_ref = ResourceRef::of::<DbConnection>();
        assert_eq!(db_ref.type_id(), TypeId::of::<DbConnection>());
    }

    #[test]
    fn test_resource_ref_equality() {
        let ref1 = ResourceRef::of::<DbConnection>();
        let ref2 = ResourceRef::of::<DbConnection>();
        let ref3 = ResourceRef::of::<CacheConnection>();

        // Same type
        assert_eq!(ref1, ref2);
        // Different type
        assert_ne!(ref1, ref3);
    }

    #[test]
    fn test_resource_ref_type_safety() {
        let db_ref = ResourceRef::of::<DbConnection>();
        let cache_ref = ResourceRef::of::<CacheConnection>();

        // Different TypeIds for different types
        assert_ne!(db_ref.type_id(), cache_ref.type_id());
    }

    #[test]
    fn test_resource_ref_const() {
        // Can be used in const contexts
        const DB_REF: ResourceRef = ResourceRef::of::<DbConnection>();
        assert_eq!(DB_REF.type_id(), TypeId::of::<DbConnection>());
    }

    #[test]
    fn test_resource_ref_clone_copy() {
        let ref1 = ResourceRef::of::<DbConnection>();
        let ref2 = ref1; // Copy
        let ref3 = ref1; // Also Copy

        assert_eq!(ref1, ref2);
        assert_eq!(ref1, ref3);
    }
}
