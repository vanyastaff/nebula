//! Rotation dispatcher infrastructure for credential refresh / revoke.
//!
//! See ADR-0036 §Decision and Tech Spec §3.2-§3.5.
//!
//! `ResourceDispatcher` is an object-safe trampoline trait that stores
//! type-erased per-resource dispatch logic in `Manager::credential_resources`.
//! `TypedDispatcher<R>` is the generic implementor that downcasts
//! `&(dyn Any)` schemes to `<R::Credential as Credential>::Scheme` and
//! invokes the resource's `on_credential_refresh` / `on_credential_revoke`.

use std::{
    any::{Any, TypeId},
    future::Future,
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use nebula_core::ResourceKey;
use nebula_credential::{Credential, CredentialId};

use crate::{resource::Resource, runtime::managed::ManagedResource};

/// Object-safe trampoline for type-erased rotation dispatch.
///
/// Stored as `Arc<dyn ResourceDispatcher>` in `Manager::credential_resources`.
/// Implementations (currently only `TypedDispatcher<R>`) downcast schemes
/// to the resource's expected `<R::Credential as Credential>::Scheme` and
/// forward to `Resource::on_credential_refresh` / `on_credential_revoke`.
#[expect(
    dead_code,
    reason = "wired by Manager::credential_resources retype in Task 3"
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

    /// Dispatch refresh: downcast `scheme` to expected type, forward to
    /// `Resource::on_credential_refresh`. Returns boxed future to keep the
    /// trait object-safe (RPITIT not allowed on dyn-safe traits).
    ///
    /// **Task 4 wires the body** — currently a `todo!()` placeholder. The
    /// open question is how to bridge `&(dyn Any)` to the typed
    /// `SchemeGuard<'a, _>` parameter that `Resource::on_credential_refresh`
    /// requires.
    fn dispatch_refresh<'a>(
        &'a self,
        scheme: &'a (dyn Any + Send + Sync),
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
    #[expect(dead_code, reason = "called by register_inner in Task 3")]
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
        scheme: &'a (dyn Any + Send + Sync),
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::Error>> + Send + 'a>> {
        Box::pin(async move {
            // Downcast guard for diagnostics — the actual refresh dispatch
            // requires bridging &Scheme to SchemeGuard<'a, R::Credential>.
            // Task 4 of the П2 plan resolves this design question.
            let _scheme: &<R::Credential as Credential>::Scheme = scheme
                .downcast_ref::<<R::Credential as Credential>::Scheme>()
                .ok_or_else(crate::Error::scheme_type_mismatch::<R>)?;

            // TASK 4: wire SchemeGuard reconstruction from &Scheme + invoke
            // self.managed.resource.on_credential_refresh(guard, ctx)
            // Resolution path: add SchemeGuard::from_borrow helper in
            // nebula-credential (option α from plan), or pivot to
            // closure-based dispatch (option β).
            todo!("Task 4: wire SchemeGuard bridging — see plan critical note")
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
