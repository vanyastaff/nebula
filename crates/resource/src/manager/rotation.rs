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
            //
            // Classification (CodeRabbit 🟠 #3): the previous unconditional
            // `Error::permanent` mis-classified transient acquire failures
            // (network blip, vault unavailable, lock contention) as terminal
            // and biased the rotation-attempts metric toward `failed`. We
            // now consult `CredentialError::is_retryable()` (via
            // `nebula_error::Classify`) to map the failure to
            // [`Error::transient`] when retryable and [`Error::permanent`]
            // otherwise. Authentication failures should fail in credential
            // resolution before reaching the factory closure; if they DO
            // reach here, the classifier reports them as non-retryable so
            // the `permanent` arm still applies.
            let guard = factory.acquire().await.map_err(|e| {
                use nebula_error::Classify as _;
                let key = R::key();
                if e.is_retryable() {
                    crate::Error::transient(format!("{key}: SchemeFactory::acquire: {e}"))
                } else {
                    crate::Error::permanent(format!("{key}: SchemeFactory::acquire: {e}"))
                }
            })?;

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

// ============================================================================
// Dispatch loops — fan-out helpers for `Manager::on_credential_refreshed`
// and `Manager::on_credential_revoked`.
// ============================================================================

use std::time::Instant;

use futures::future::join_all;
use tracing::Instrument as _;

use crate::{
    error::{RefreshOutcome, RevokeOutcome},
    events::ResourceEvent,
    manager::Manager,
};

impl Manager {
    /// Handle a credential refresh event by fanning out
    /// `Resource::on_credential_refresh` hooks to every resource bound to
    /// `credential_id`.
    ///
    /// Each per-resource future runs concurrently via `join_all` and has its
    /// own timeout budget — one slow or failing resource never poisons
    /// siblings (security amendment B-1 isolation). The per-resource budget
    /// is sourced from `RegisterOptions::credential_rotation_timeout` if set
    /// at register time, otherwise the manager's
    /// [`ManagerConfig::credential_rotation_timeout`](crate::manager::ManagerConfig::credential_rotation_timeout)
    /// default.
    ///
    /// Returns a list of `(ResourceKey, RefreshOutcome)` pairs — one per
    /// affected resource. If no resources are bound to the credential the
    /// result is an empty `Vec`.
    ///
    /// # Caller contract
    ///
    /// The caller (typically the engine refresh coordinator) constructs a
    /// [`SchemeFactory<C>`](nebula_credential::SchemeFactory) over the
    /// freshly-projected post-refresh state. Per fan-out branch the manager
    /// clones the factory, type-erases it through a `Box<dyn Any>`, and
    /// passes it to the resource's typed dispatcher. The dispatcher
    /// downcasts back to `SchemeFactory<R::Credential>` and calls
    /// `factory.acquire().await` to mint a fresh
    /// [`SchemeGuard<'_, R::Credential>`](nebula_credential::SchemeGuard)
    /// inside its own typed scope before invoking the resource hook.
    ///
    /// **Why dispatcher-side acquire (not manager-side):** `Box<dyn Any>`
    /// is `'static`-bound (because `Any: 'static`), so a non-`'static`
    /// `SchemeGuard<'_, C>` cannot be type-erased through it. `SchemeFactory<C>`
    /// IS `'static`, so it can. The acquire call is the same in either
    /// design (one per dispatcher); only the call boundary moves.
    ///
    /// # Errors
    ///
    /// This call itself never returns `Err`; per-resource failures are
    /// reported in the returned `Vec` via [`RefreshOutcome::Failed`] and
    /// [`RefreshOutcome::TimedOut`]. The `Result` shape is preserved for
    /// forward-compat with future caller-level guards (e.g. shutdown-in-
    /// progress short-circuit).
    pub async fn on_credential_refreshed<C: Credential>(
        &self,
        credential_id: &CredentialId,
        factory: SchemeFactory<C>,
        ctx: &CredentialContext,
    ) -> Result<Vec<(ResourceKey, RefreshOutcome)>, crate::Error> {
        let dispatchers = self
            .credential_resources
            .get(credential_id)
            .map(|entry| entry.value().clone())
            .unwrap_or_default();

        if dispatchers.is_empty() {
            return Ok(Vec::new());
        }

        let scheme_type_id = TypeId::of::<<C as Credential>::Scheme>();
        let default_timeout = self.credential_rotation_timeout;
        let metrics = self.metrics.clone();

        let span = tracing::info_span!(
            "resource.credential_refresh",
            credential_id = %credential_id,
            resources_affected = dispatchers.len(),
        );

        // Per-resource futures with isolation. Each future has its own
        // timeout budget; one slow or failing resource never poisons
        // siblings (security amendment B-1).
        let futures = dispatchers.into_iter().filter_map(|d| {
            // Defensive scheme-type check. The dispatcher's
            // `scheme_type_id()` should match the call's `C` if
            // `register_inner` was correct; if not, log and skip rather
            // than panic.
            if d.scheme_type_id() != scheme_type_id {
                tracing::error!(
                    resource = %d.resource_key(),
                    expected_scheme_type = ?scheme_type_id,
                    got_scheme_type = ?d.scheme_type_id(),
                    "dispatcher scheme_type_id mismatch — skipping (register_inner bug?)",
                );
                return None;
            }

            let timeout = d.timeout_override().unwrap_or(default_timeout);
            let key = d.resource_key();
            // Cheap Arc bump per dispatcher; the boxed factory becomes the
            // dispatcher's owned `SchemeFactory<R::Credential>` after
            // downcast.
            let factory_box: Box<dyn Any + Send + Sync> = Box::new(factory.clone());
            let metrics = metrics.clone();

            Some(async move {
                let dispatch_started = Instant::now();
                let dispatch = d.dispatch_refresh(factory_box, ctx);
                let outcome = match tokio::time::timeout(timeout, dispatch).await {
                    Ok(Ok(())) => RefreshOutcome::Ok,
                    Ok(Err(e)) => RefreshOutcome::Failed(e),
                    Err(_) => RefreshOutcome::TimedOut { budget: timeout },
                };
                let elapsed = dispatch_started.elapsed().as_secs_f64();

                if let Some(m) = metrics.as_ref() {
                    m.record_rotation_dispatch(&outcome, elapsed);
                }

                (key, outcome)
            })
        });

        let results: Vec<(ResourceKey, RefreshOutcome)> = join_all(futures).instrument(span).await;

        // Aggregate per-resource outcomes into a `RotationOutcome` for the
        // `CredentialRefreshed` event payload (Tech Spec §6.2). Per-resource
        // dispatch metrics are emitted inline above; this is the cycle-level
        // aggregate.
        let mut outcome = crate::error::RotationOutcome::default();
        for (_, o) in &results {
            match o {
                RefreshOutcome::Ok => outcome.ok += 1,
                RefreshOutcome::Failed(_) => outcome.failed += 1,
                RefreshOutcome::TimedOut { .. } => outcome.timed_out += 1,
            }
        }
        // Broadcast send errors (no live subscribers) are intentionally ignored.
        let _ = self.event_tx.send(ResourceEvent::CredentialRefreshed {
            credential_id: *credential_id,
            resources_affected: results.len(),
            outcome,
        });

        Ok(results)
    }

    /// Handle a credential revocation event by fanning out
    /// `Resource::on_credential_revoke` hooks to every resource bound to
    /// `credential_id`.
    ///
    /// Symmetric to [`on_credential_refreshed`](Self::on_credential_refreshed)
    /// minus the scheme-material plumbing — `on_credential_revoke` takes only
    /// `&CredentialId`, so no `SchemeFactory<C>` or generic `C: Credential`
    /// bound is required here.
    ///
    /// Each per-resource future runs concurrently via `join_all` and has its
    /// own timeout budget — one slow or failing resource never poisons
    /// siblings (security amendment B-1 isolation). The per-resource budget
    /// is sourced from `RegisterOptions::credential_rotation_timeout` if set
    /// at register time, otherwise the manager's
    /// [`ManagerConfig::credential_rotation_timeout`](crate::manager::ManagerConfig::credential_rotation_timeout)
    /// default.
    ///
    /// Per security amendment B-2, every non-`Ok` per-resource outcome
    /// (`Failed` or `TimedOut`) emits a
    /// [`ResourceEvent::HealthChanged`] event with `healthy: false` inline
    /// so the operator sees a per-resource failure signal even if the
    /// aggregate `CredentialRevoked` event (Task 7) is dropped by a
    /// saturated subscriber. Successful revocations emit only the aggregate
    /// event.
    ///
    /// Returns a list of `(ResourceKey, RevokeOutcome)` pairs — one per
    /// affected resource. If no resources are bound to the credential the
    /// result is an empty `Vec`.
    ///
    /// # Errors
    ///
    /// This call itself never returns `Err`; per-resource failures are
    /// reported in the returned `Vec` via [`RevokeOutcome::Failed`] and
    /// [`RevokeOutcome::TimedOut`]. The `Result` shape is preserved for
    /// forward-compat with future caller-level guards (e.g. shutdown-in-
    /// progress short-circuit).
    pub async fn on_credential_revoked(
        &self,
        credential_id: &CredentialId,
    ) -> Result<Vec<(ResourceKey, RevokeOutcome)>, crate::Error> {
        let dispatchers = self
            .credential_resources
            .get(credential_id)
            .map(|entry| entry.value().clone())
            .unwrap_or_default();

        if dispatchers.is_empty() {
            return Ok(Vec::new());
        }

        let default_timeout = self.credential_rotation_timeout;
        let event_tx = self.event_tx.clone();
        let metrics = self.metrics.clone();

        let span = tracing::warn_span!(
            "resource.credential_revoke",
            credential_id = %credential_id,
            resources_affected = dispatchers.len(),
        );

        // Per-resource futures with isolation. Each future has its own
        // timeout budget; one slow or failing resource never poisons
        // siblings (security amendment B-1). Per security amendment B-2:
        // emit HealthChanged{healthy:false} inline for any non-Ok outcome
        // so the operator sees a per-resource failure signal even if the
        // aggregate event is lost.
        let futures = dispatchers.into_iter().map(|d| {
            let timeout = d.timeout_override().unwrap_or(default_timeout);
            let key = d.resource_key();
            let credential_id = *credential_id;
            let event_tx = event_tx.clone();
            let metrics = metrics.clone();

            async move {
                let dispatch_started = Instant::now();
                let dispatch = d.dispatch_revoke(&credential_id);
                let outcome = match tokio::time::timeout(timeout, dispatch).await {
                    Ok(Ok(())) => RevokeOutcome::Ok,
                    Ok(Err(e)) => RevokeOutcome::Failed(e),
                    Err(_) => RevokeOutcome::TimedOut { budget: timeout },
                };
                let elapsed = dispatch_started.elapsed().as_secs_f64();

                if let Some(m) = metrics.as_ref() {
                    m.record_revoke_dispatch(&outcome, elapsed);
                }

                // Security amendment B-2: emit HealthChanged{healthy:false}
                // for non-Ok outcomes. Successful revocations emit only the
                // aggregate CredentialRevoked event below. Broadcast send
                // errors (no live subscribers) are intentionally ignored.
                if !matches!(outcome, RevokeOutcome::Ok) {
                    let _ = event_tx.send(ResourceEvent::HealthChanged {
                        key: key.clone(),
                        healthy: false,
                    });
                }

                (key, outcome)
            }
        });

        let results: Vec<(ResourceKey, RevokeOutcome)> = join_all(futures).instrument(span).await;

        // Aggregate per-resource outcomes into a `RotationOutcome` for the
        // `CredentialRevoked` event payload (Tech Spec §6.2). Per-resource
        // dispatch metrics are emitted inline above; this is the cycle-level
        // aggregate.
        let mut outcome = crate::error::RotationOutcome::default();
        for (_, o) in &results {
            match o {
                RevokeOutcome::Ok => outcome.ok += 1,
                RevokeOutcome::Failed(_) => outcome.failed += 1,
                RevokeOutcome::TimedOut { .. } => outcome.timed_out += 1,
            }
        }
        // Broadcast send errors (no live subscribers) are intentionally ignored.
        let _ = self.event_tx.send(ResourceEvent::CredentialRevoked {
            credential_id: *credential_id,
            resources_affected: results.len(),
            outcome,
        });

        Ok(results)
    }
}
