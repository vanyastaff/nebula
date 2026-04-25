//! `Manager` shape — minimum surface to demonstrate the dispatcher works.
//!
//! Validates two structural decisions that matter for Phase 6 Tech Spec:
//!
//! 1. **Reverse-index write path lands atomically** (resolves Phase 1 finding 🔴-1 from the live
//!    `manager.rs:262, 370` `todo!()` panic). `register::<R>` populates `credential_resources`
//!    whenever `R::Credential` is something other than [`crate::NoCredential`].
//!    `on_credential_refreshed` and `on_credential_revoked` consume the populated index — never
//!    panic on lookup.
//!
//! 2. **Parallel dispatch with per-resource isolation** (Strategy §4.3). Each per-resource future
//!    runs in its own `tokio::time::timeout` bubble. One slow / panicking / Err-returning resource
//!    does NOT block siblings. The whole batch joins via [`futures::future::join_all`].
//!
//! Production manager will be far richer (it's the 2101-line file under
//! `crates/resource/src/manager.rs`). The spike trims to exactly the
//! surface that proves §3.6 composes — registration, the reverse index,
//! the dispatcher.
//!
//! Type-erasure trick: each registered resource is wrapped in a
//! [`ResourceDispatcher`] trampoline that knows how to downcast a
//! `&dyn Any` scheme to the concrete `<R::Credential as Credential>::Scheme`.
//! The dispatcher is typed-erased at the registry boundary so the
//! manager can hold a heterogeneous `Vec<Box<dyn ResourceDispatcher>>`
//! per credential.

use std::{
    any::{Any, TypeId},
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use futures::future::join_all;
use nebula_credential::{Credential, CredentialId};
use tokio::sync::RwLock;

use crate::{no_credential::NoCredential, resource::Resource};

// ── Public dispatch outcomes ──────────────────────────────────────────

/// Per-resource dispatch result emitted by [`Manager::on_credential_refreshed`]
/// / [`Manager::on_credential_revoked`].
///
/// Production will broadcast these as `ResourceEvent::CredentialRefreshed`
/// / `ResourceEvent::CredentialRevoked` per Strategy §4.9. Spike just
/// returns them directly so tests can assert per-resource isolation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// Hook completed within budget.
    Ok,
    /// Hook returned `Err(...)`. String form keeps the spike's output
    /// type erased (real impl emits typed `Self::Error` per resource).
    Failed(String),
    /// Hook exceeded its per-resource timeout. Sibling dispatches
    /// continue regardless — this is the isolation invariant under test.
    TimedOut,
}

/// Aggregate outcome of a single rotation event.
#[derive(Debug, Clone, Default)]
pub struct RotationOutcome {
    /// One [`DispatchOutcome`] per resource registered against the
    /// rotated credential. Order matches `register` insertion order.
    pub per_resource: Vec<DispatchOutcome>,
}

impl RotationOutcome {
    /// True iff every resource ack'd `Ok`.
    pub fn all_ok(&self) -> bool {
        self.per_resource
            .iter()
            .all(|o| matches!(o, DispatchOutcome::Ok))
    }
}

// ── Dispatcher trampoline ─────────────────────────────────────────────

/// Type-erased dispatcher for a single registered resource.
///
/// The trampoline closures inside know the concrete `R` and can downcast
/// the type-erased scheme back to `<R::Credential as Credential>::Scheme`.
///
/// This is the spike's stand-in for production's reverse-index entry —
/// it's enough to prove the dispatch loop works without committing to a
/// concrete production layout.
pub trait ResourceDispatcher: Send + Sync + 'static {
    /// Returns the `TypeId` of the expected scheme. Used at dispatch time
    /// to fail-fast if the engine ever passes a scheme of the wrong type
    /// — that's a programmer error worth surfacing loudly.
    fn scheme_type_id(&self) -> TypeId;

    /// Calls `R::on_credential_refresh(scheme)` after downcasting.
    ///
    /// Returns a `BoxFuture` because we're behind `dyn Trait`. The
    /// production trait would use a generic associated future or pin the
    /// trait in another way; spike keeps it simple. RPITIT in trait
    /// objects is not yet stable on 1.95.
    ///
    /// `&(dyn Any + Send + Sync)` rather than `&dyn Any` because we
    /// hand the reference to a `Send` async block — bare `&dyn Any` is
    /// not `Sync`, so the resulting future would not be `Send` and
    /// could not be `join_all`'d on a multi-threaded runtime.
    fn dispatch_refresh<'a>(
        &'a self,
        scheme: &'a (dyn Any + Send + Sync),
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

    /// Calls `R::on_credential_revoke(credential_id)`.
    fn dispatch_revoke<'a>(
        &'a self,
        credential_id: &'a CredentialId,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;
}

/// Concrete dispatcher built from a typed `R: Resource`. Holds an
/// `Arc<R>` so the manager can keep multiple references without
/// requiring `R: Clone`.
struct TypedDispatcher<R: Resource> {
    resource: Arc<R>,
}

impl<R: Resource> ResourceDispatcher for TypedDispatcher<R> {
    fn scheme_type_id(&self) -> TypeId {
        TypeId::of::<<R::Credential as Credential>::Scheme>()
    }

    fn dispatch_refresh<'a>(
        &'a self,
        scheme: &'a (dyn Any + Send + Sync),
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            // Downcast the type-erased scheme. If this fails, it's a
            // programmer error — Manager constructed the dispatcher
            // map keyed on the wrong type. Spike turns the panic into
            // a string error so the test harness can observe it.
            let scheme: &<R::Credential as Credential>::Scheme =
                scheme.downcast_ref().ok_or_else(|| {
                    "scheme type mismatch — dispatcher wired against the wrong credential"
                        .to_owned()
                })?;
            self.resource
                .on_credential_refresh(scheme)
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn dispatch_revoke<'a>(
        &'a self,
        credential_id: &'a CredentialId,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            self.resource
                .on_credential_revoke(credential_id)
                .await
                .map_err(|e| e.to_string())
        })
    }
}

// ── Manager ───────────────────────────────────────────────────────────

/// Spike `Manager` — minimum surface for dispatcher validation.
///
/// Production `Manager` is the 2101-line file at `crates/resource/src/
/// manager.rs`. Spike keeps three things:
/// - `register::<R>` (populates the reverse index)
/// - `on_credential_refreshed(id, scheme)` (parallel dispatch)
/// - `on_credential_revoked(id)` (parallel dispatch)
///
/// The per-resource timeout is configurable so tests can pin a small
/// budget and demonstrate isolation under deliberate slowdowns.
pub struct Manager {
    /// Reverse index from credential → list of resources bound to it.
    /// Phase 1 finding 🔴-1: live code's `register` forgot to populate
    /// this and dispatch panicked with `todo!()`. Spike makes the write
    /// path explicit.
    by_credential: RwLock<HashMap<CredentialId, Vec<Arc<dyn ResourceDispatcher>>>>,
    /// Per-resource refresh budget. Defaults to 1s — small enough that
    /// tests can drive deliberate slowdowns without long sleeps; big
    /// enough that healthy hooks always finish.
    per_resource_timeout: Duration,
}

impl Manager {
    /// Construct a new manager with the default per-resource timeout (1s).
    pub fn new() -> Self {
        Self {
            by_credential: RwLock::new(HashMap::new()),
            per_resource_timeout: Duration::from_secs(1),
        }
    }

    /// Construct with an explicit per-resource timeout. Tests pin this
    /// to demonstrate that one slow resource does not extend wall-clock
    /// for siblings.
    pub fn with_timeout(per_resource_timeout: Duration) -> Self {
        Self {
            by_credential: RwLock::new(HashMap::new()),
            per_resource_timeout,
        }
    }

    /// Register a resource against an optional credential.
    ///
    /// # Reverse-index write path
    ///
    /// - If `R::Credential` resolves to [`NoCredential`], the resource is NOT inserted into the
    ///   reverse index. `credential_id` is ignored. Trying to register a `NoCredential` resource
    ///   against a real credential id is a configuration error, but spike tolerates it (production
    ///   should warn).
    /// - Otherwise, `credential_id` MUST be `Some(_)` — caller is binding this resource to a
    ///   specific stored credential. `register` errors if it isn't, surfacing the misuse early.
    ///
    /// This method is the explicit write path that Phase 1 found missing.
    /// Production will additionally do schema validation, lifecycle
    /// state setup, etc.
    pub async fn register<R: Resource>(
        &self,
        resource: Arc<R>,
        credential_id: Option<CredentialId>,
    ) -> Result<(), &'static str> {
        // Compile-time check whether R opted out of credentials.
        let opted_out = TypeId::of::<R::Credential>() == TypeId::of::<NoCredential>();

        match (opted_out, credential_id) {
            (true, _) => {
                // No reverse-index write — `NoCredential` resources
                // never receive rotation hooks.
                Ok(())
            },
            (false, None) => Err("credential-bearing Resource registered without a credential id"),
            (false, Some(id)) => {
                let dispatcher: Arc<dyn ResourceDispatcher> =
                    Arc::new(TypedDispatcher { resource });
                let mut guard = self.by_credential.write().await;
                guard.entry(id).or_default().push(dispatcher);
                Ok(())
            },
        }
    }

    /// Parallel rotation dispatch with per-resource timeout isolation.
    ///
    /// `scheme` is borrowed (not cloned) per Strategy §4.3 hot-path
    /// invariant: "each clone is another zeroize obligation".
    pub async fn on_credential_refreshed(
        &self,
        credential_id: &CredentialId,
        scheme: &(dyn Any + Send + Sync),
    ) -> RotationOutcome {
        // Take a cheap snapshot of the dispatchers under read lock,
        // drop the guard before any `.await`. Holding `RwLockReadGuard`
        // across `.await` is fine for tokio::sync::RwLock but releasing
        // early matches the production hot-path pattern (avoid blocking
        // `register` while a slow rotation is in flight).
        let dispatchers: Vec<Arc<dyn ResourceDispatcher>> = {
            let guard = self.by_credential.read().await;
            match guard.get(credential_id) {
                Some(list) => list.to_vec(),
                None => return RotationOutcome::default(),
            }
        };

        let timeout = self.per_resource_timeout;
        let futures = dispatchers.iter().map(|d| {
            let d = Arc::clone(d);
            async move {
                match tokio::time::timeout(timeout, d.dispatch_refresh(scheme)).await {
                    Ok(Ok(())) => DispatchOutcome::Ok,
                    Ok(Err(e)) => DispatchOutcome::Failed(e),
                    Err(_) => DispatchOutcome::TimedOut,
                }
            }
        });

        let per_resource = join_all(futures).await;
        RotationOutcome { per_resource }
    }

    /// Parallel revocation dispatch — symmetric to refresh.
    pub async fn on_credential_revoked(&self, credential_id: &CredentialId) -> RotationOutcome {
        let dispatchers: Vec<Arc<dyn ResourceDispatcher>> = {
            let guard = self.by_credential.read().await;
            match guard.get(credential_id) {
                Some(list) => list.to_vec(),
                None => return RotationOutcome::default(),
            }
        };

        let timeout = self.per_resource_timeout;
        let futures = dispatchers.iter().map(|d| {
            let d = Arc::clone(d);
            async move {
                match tokio::time::timeout(timeout, d.dispatch_revoke(credential_id)).await {
                    Ok(Ok(())) => DispatchOutcome::Ok,
                    Ok(Err(e)) => DispatchOutcome::Failed(e),
                    Err(_) => DispatchOutcome::TimedOut,
                }
            }
        });

        let per_resource = join_all(futures).await;
        RotationOutcome { per_resource }
    }

    /// Test-only inspection — how many dispatchers are wired against this id.
    /// Production will instead emit a metric.
    pub async fn dispatcher_count(&self, credential_id: &CredentialId) -> usize {
        self.by_credential
            .read()
            .await
            .get(credential_id)
            .map(Vec::len)
            .unwrap_or(0)
    }
}

impl Default for Manager {
    fn default() -> Self {
        Self::new()
    }
}
