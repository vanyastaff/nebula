//! Rotation dispatcher infrastructure for credential refresh / revoke.
//!
//! See ADR-0036 §Decision and Tech Spec §3.2-§3.5.
//!
//! `ResourceDispatcher` is an object-safe trampoline trait that stores
//! type-erased per-resource dispatch logic in `Manager::credential_resources`.
//! `TypedDispatcher<R>` is the generic implementor that downcasts an owned,
//! type-erased `Box<SchemeFactory<R::Credential>>`, calls `acquire()` to mint
//! a fresh `SchemeGuard<'_, R::Credential>`, and forwards it to the resource's
//! `on_credential_refresh` / `on_credential_revoke` hook.
//!
//! **Why factory not guard at the boundary:** `Box<dyn Any>` is `'static`-
//! bound (because `Any: 'static`), so a non-`'static` `SchemeGuard<'_, C>`
//! cannot be type-erased through it. `SchemeFactory<C>` IS `'static` (its
//! inner `Arc<dyn Fn>` is `'static`), so it crosses the boundary cleanly.
//! The `acquire()` call inside `dispatch_refresh` produces a per-call guard
//! whose `ZeroizeOnDrop` semantics fire deterministically when the dispatch
//! future completes — equivalent secret-hygiene to manager-side acquire.

use std::{
    any::{Any, TypeId},
    future::Future,
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use nebula_core::ResourceKey;
use nebula_credential::{Credential, CredentialContext, CredentialId, SchemeFactory};

use crate::{resource::Resource, runtime::managed::ManagedResource};

/// Object-safe trampoline for type-erased rotation dispatch.
///
/// Stored as `Arc<dyn ResourceDispatcher>` in `Manager::credential_resources`.
/// Implementations (currently only `TypedDispatcher<R>`) downcast schemes
/// to the resource's expected `<R::Credential as Credential>::Scheme` and
/// forward to `Resource::on_credential_refresh` / `on_credential_revoke`.
#[expect(
    dead_code,
    reason = "dispatch_revoke is wired by on_credential_revoked in Task 5"
)]
pub(crate) trait ResourceDispatcher: Send + Sync + 'static {
    /// Resource key for diagnostics + event emission.
    fn resource_key(&self) -> ResourceKey;

    /// `TypeId` of the resource's `<R::Credential as Credential>::Scheme`.
    /// Used by callers to verify the scheme they're about to pass matches.
    fn scheme_type_id(&self) -> TypeId;

    /// Per-resource timeout override set at register time, or `None` to use
    /// the Manager default.
    fn timeout_override(&self) -> Option<Duration>;

    /// Dispatch refresh: downcast `factory_box` (an owned, type-erased
    /// `SchemeFactory<R::Credential>`), call `acquire()` to mint a fresh
    /// `SchemeGuard<'_, R::Credential>`, and forward to
    /// `Resource::on_credential_refresh`. Returns a boxed future to keep
    /// the trait object-safe (RPITIT is not allowed on dyn-safe traits).
    ///
    /// `factory_box` is `Box<SchemeFactory<R::Credential>>` erased to
    /// `Box<dyn Any + Send + Sync>`. Because `SchemeFactory<C>: 'static`
    /// (its inner `Arc<dyn Fn>` is `'static`), the box's trait-object
    /// lifetime defaults to `'static` cleanly — no lifetime juggling
    /// required for the dispatch boundary, while the inner `acquire()`
    /// still mints per-call non-`'static` `SchemeGuard`s with the correct
    /// `ZeroizeOnDrop` semantics.
    ///
    /// Per Task 4 design (option γ-modified, dispatcher-side acquire):
    /// callers (the manager) clone the `SchemeFactory<C>` per-dispatcher,
    /// box it, and pass it down. The dispatcher's `scheme_type_id()`
    /// should already have been verified by the caller; a downcast failure
    /// here indicates a dispatch bug and is reported as
    /// `Error::scheme_type_mismatch`.
    fn dispatch_refresh<'a>(
        &'a self,
        factory_box: Box<dyn Any + Send + Sync>,
        ctx: &'a CredentialContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::Error>> + Send + 'a>>;

    /// Dispatch revoke: forward `credential_id` to
    /// `Resource::on_credential_revoke`.
    fn dispatch_revoke<'a>(
        &'a self,
        credential_id: &'a CredentialId,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::Error>> + Send + 'a>>;
}

/// Typed wrapper that adapts `Resource::on_credential_refresh` /
/// `on_credential_revoke` to the object-safe `ResourceDispatcher` interface.
pub(crate) struct TypedDispatcher<R: Resource> {
    pub(crate) managed: Arc<ManagedResource<R>>,
    pub(crate) timeout_override: Option<Duration>,
}

impl<R: Resource> TypedDispatcher<R> {
    pub(crate) fn new(
        managed: Arc<ManagedResource<R>>,
        timeout_override: Option<Duration>,
    ) -> Self {
        Self {
            managed,
            timeout_override,
        }
    }
}

impl<R: Resource> ResourceDispatcher for TypedDispatcher<R> {
    fn resource_key(&self) -> ResourceKey {
        R::key()
    }

    fn scheme_type_id(&self) -> TypeId {
        TypeId::of::<<R::Credential as Credential>::Scheme>()
    }

    fn timeout_override(&self) -> Option<Duration> {
        self.timeout_override
    }

    fn dispatch_refresh<'a>(
        &'a self,
        factory_box: Box<dyn Any + Send + Sync>,
        ctx: &'a CredentialContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::Error>> + Send + 'a>> {
        Box::pin(async move {
            // Downcast Box<dyn Any> → Box<SchemeFactory<R::Credential>> → owned factory.
            // The dispatcher's `scheme_type_id()` should have already been
            // verified against the caller's credential type; a downcast
            // failure here therefore indicates a Manager-side dispatch bug.
            let factory_box: Box<SchemeFactory<R::Credential>> = factory_box
                .downcast::<SchemeFactory<R::Credential>>()
                .map_err(|_| crate::Error::scheme_type_mismatch::<R>())?;

            let factory: SchemeFactory<R::Credential> = *factory_box;

            // Acquire a fresh per-dispatcher SchemeGuard. Lives only for the
            // remainder of this future; dropped (and `ZeroizeOnDrop`
            // plaintext zeroed) at the await boundary below — RAII handles
            // secret hygiene per §15.7.
            let guard = factory
                .acquire()
                .await
                .map_err(|e| crate::Error::permanent(format!("SchemeFactory::acquire: {e}")))?;

            self.managed
                .resource
                .on_credential_refresh(guard, ctx)
                .await
                .map_err(Into::into)
        })
    }

    fn dispatch_revoke<'a>(
        &'a self,
        credential_id: &'a CredentialId,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::Error>> + Send + 'a>> {
        Box::pin(async move {
            self.managed
                .resource
                .on_credential_revoke(credential_id)
                .await
                .map_err(Into::into)
        })
    }
}
