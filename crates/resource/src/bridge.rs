//! Bridge between the resource manager and the action execution layer.
//!
//! Provides [`AuthProvider`] for credential resolution and
//! [`ManagedResourceAccessor`] which implements the action crate's
//! `ResourceAccessor` trait, connecting the resource manager to action
//! execution contexts.

use std::any::Any;
use std::future::Future;
use std::pin::Pin;

use nebula_core::ResourceKey;

use crate::ctx::BasicCtx;
use crate::error::Error;
use crate::metrics::ResourceOpsMetrics;
use crate::options::AcquireOptions;

/// Type-erased auth material returned by [`AuthProvider::resolve`].
pub type AuthMaterial = Option<Box<dyn Any + Send + Sync>>;

/// Future type returned by [`AuthProvider::resolve`].
pub type ResolveFuture<'a> = Pin<Box<dyn Future<Output = Result<AuthMaterial, Error>> + Send + 'a>>;

/// Resolves authentication material for resource creation.
///
/// Implemented by the engine layer to bridge the credential store with
/// resource management. The returned value is type-erased and will be
/// downcast to `R::Auth` by the resource manager at acquire time.
///
/// # Implementors
///
/// A typical implementation wraps a `CredentialStore` and uses
/// `CredentialSnapshot::into_project::<S>()` to extract the auth scheme,
/// then boxes it:
///
/// ```rust,ignore
/// #[async_trait]
/// impl AuthProvider for EngineAuthProvider {
///     async fn resolve(
///         &self,
///         resource_key: &ResourceKey,
///     ) -> Result<Box<dyn Any + Send + Sync>, Error> {
///         let cred_key = self.bindings.get(resource_key)?;
///         let snapshot = self.store.get(&cred_key).await?;
///         Ok(snapshot.into_any_scheme())
///     }
/// }
/// ```
pub trait AuthProvider: Send + Sync {
    /// Resolve auth material for the given resource key.
    ///
    /// Returns type-erased auth (`Box<dyn Any>`) that will be downcast
    /// to the resource's `Auth` associated type. Returns `Ok(None)` if
    /// the resource requires no authentication (i.e., `Auth = ()`).
    fn resolve(&self, resource_key: &ResourceKey) -> ResolveFuture<'_>;
}

/// Type-erased acquire function stored per registered resource.
///
/// Created at registration time when concrete type information is
/// available, and called at acquire time through the type-erased path.
pub(crate) type ErasedAcquireFn = Box<
    dyn Fn(
        Option<Box<dyn Any + Send + Sync>>, // auth (None for Auth = ())
        BasicCtx,                            // execution context
        AcquireOptions,                      // acquire options
        Option<ResourceOpsMetrics>,          // optional metrics
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn Any + Send>, Error>> + Send>>
        + Send
        + Sync,
>;
