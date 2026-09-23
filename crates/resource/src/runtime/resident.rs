//! Resident topology with one retained master and shared owning lease entries.
//!
//! Only the framework destroys instances. Retiring a master transfers its Arc
//! to framework cleanup; the final lifecycle owner extracts the instance.

use std::{marker::PhantomData, sync::Arc};

use tokio::sync::Mutex;

use crate::topology::store::StoreView;
use crate::{
    context::ResourceContext,
    error::Error,
    resource::Provider,
    topology::{CreatedEntry, Ticket, Topology, Unavailable, resident::config::Config},
    topology_tag::TopologyTag,
};

/// One retained master shared by leases without requiring instance cloning.
///
/// Creation, rotation, and terminal close serialize on one state lock. This
/// excludes transient Arc owners outside the lock from final-owner extraction.
pub struct Resident<R: Provider> {
    state: Mutex<ResidentState>,
    config: Config,
    _resource: PhantomData<fn() -> R>,
}

struct ResidentState {
    master: Option<crate::RetainedId>,
    built_epoch: u64,
    built_fingerprint: u64,
    closed: bool,
}

impl<R: Provider> std::fmt::Debug for Resident<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.try_lock().ok();
        f.debug_struct("Resident")
            .field("config", &self.config)
            .field(
                "is_initialized",
                &state.as_ref().map(|state| state.master.is_some()),
            )
            .field("closed", &state.as_ref().map(|state| state.closed))
            .finish_non_exhaustive()
    }
}

impl<R: Provider> Resident<R> {
    /// Creates a resident topology without initializing its master.
    pub fn new(config: Config) -> Self {
        Self {
            state: Mutex::new(ResidentState {
                master: None,
                built_epoch: 0,
                built_fingerprint: 0,
                closed: false,
            }),
            config,
            _resource: PhantomData,
        }
    }

    /// Returns the operational configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Reports whether a master is currently retained.
    pub async fn is_initialized(&self) -> bool {
        self.state.lock().await.master.is_some()
    }

    #[cfg(test)]
    pub(crate) async fn built_epoch_for_test(&self) -> u64 {
        self.state.lock().await.built_epoch
    }

    /// Per-slot rotation hook dispatch for the Resident topology, with the
    /// create-vs-rotate reconcile (per-resource revoke deferral / #680).
    ///
    /// Takes `create_lock` so it is **mutually excluded** with the
    /// `create_entry` slow path: a rotation dispatch and a first-acquire create
    /// can never interleave, which is what makes the reconcile exactly-once.
    /// Under the lock:
    ///
    /// - **Instance present, built epoch ≥ slot epoch** — up to date: deliver
    ///   the hook normally.
    /// - **Instance present, built epoch < slot epoch** — the instance was
    ///   bound to a pre-rotation credential (the lost-update): still deliver
    ///   the hook (the resource's `&self` reaction rebinds against the now
    ///   current slot) and, on success, advance the recorded epoch.
    /// - **No instance** — nothing live to refresh. Genuinely a no-op
    ///   `Ok(())`: a never-created resident has no stale instance to leave
    ///   behind, and a create *racing* this dispatch is serialised by
    ///   `create_lock` — it runs strictly before or after.
    ///
    /// `refresh = true` selects `on_credential_refresh`, `false`
    /// `on_credential_revoke`. The revoke direction is symmetric: an instance
    /// built against an older epoch is still delivered the revoke hook (it
    /// must stop emitting on the now-revoked credential); a never-created
    /// resident has nothing emitting, so the no-op is correct there too.
    ///
    /// # Errors
    ///
    /// Propagates the resource's `on_credential_*` error; on a stale-reconcile
    /// failure the recorded epoch is deliberately not advanced so the next
    /// dispatch re-attempts. A hook that hangs or panics is bounded/isolated
    /// and surfaces as a typed [`Error`] the same way.
    ///
    /// # Two-tier hook ceiling
    ///
    /// The hook dispatch below is bounded by
    /// [`DEFAULT_AUTHOR_HOOK_CEILING`](crate::hook_guard::DEFAULT_AUTHOR_HOOK_CEILING)
    /// *after* `create_lock` is already held — the ceiling times only the
    /// author's hook body, never the lock-wait. `Manager::refresh_slot` /
    /// `drain_and_revoke` additionally wrap the *whole* dispatch (lock-wait
    /// included) in a looser framework-owned outer backstop. Deriving the
    /// ceiling from author-set `Config::create_timeout` was rejected
    /// (ADR-0093 Tier-1: the author must never extend the framework's own
    /// budget) — this fixed constant is independent of it.
    pub(crate) async fn dispatch_resident_hook(
        &self,
        resource: &R,
        retained: &crate::RetainedStore<Arc<R::Instance>>,
        slot: &str,
        refresh: bool,
    ) -> Result<(), crate::topology::HookFault> {
        // Serialise against the create slow path: the reconcile must not
        // interleave with an instance being built / its epoch being
        // published, so delivery is exactly-once.
        let mut state = self.state.lock().await;

        let Some(runtime) = state.master.and_then(|id| retained.lease(id)) else {
            // No live runtime. Not a stale-skip: nothing is bound to a
            // credential at all, and a concurrent first create is excluded
            // by `create_lock` (it runs strictly before/after this and
            // records its own `built_epoch`). A genuinely never-created,
            // never-bound resident is a legitimate no-op.
            tracing::debug!(
                resource = %R::key(),
                slot,
                refresh,
                "resident slot hook: no live runtime — legitimate no-op \
                 (never created; not a stale-skip)"
            );
            return Ok(());
        };

        let slot_epoch = resource.credential_slot_epoch();
        let built = state.built_epoch;
        let stale = built < slot_epoch;
        if stale {
            tracing::warn!(
                resource = %R::key(),
                slot,
                refresh,
                built_epoch = built,
                slot_epoch,
                "resident slot hook: live runtime is stale (built against an \
                 older credential epoch) — reconciling by delivering the hook"
            );
        }

        // Bound + isolate the author hook itself — after `create_lock` is
        // already held, so the ceiling times only the hook body (see the
        // two-tier hook ceiling doc section above).
        //
        // SAFETY (unwind): `_guard` (the `create_lock` `MutexGuard`) is a
        // local of this function, not captured by the wrapped future below,
        // so a caught panic never unwinds past this call — `catch_unwind`
        // stops at its own boundary and control returns here normally. The
        // guard therefore still releases via its ordinary `Drop` at this
        // function's return, never left held or torn.
        let hook_op = if refresh {
            "on_credential_refresh"
        } else {
            "on_credential_revoke"
        };
        let result = match crate::hook_guard::guard_author_hook(
            crate::hook_guard::DEFAULT_AUTHOR_HOOK_CEILING,
            async {
                if refresh {
                    resource.on_credential_refresh(slot, &runtime).await
                } else {
                    resource.on_credential_revoke(slot, &runtime).await
                }
            },
        )
        .await
        {
            Ok(result) => result.map_err(crate::topology::HookFault::Failed),
            Err(fault) => {
                fault.observe(&R::key(), "rotation");
                Err(match fault {
                    crate::hook_guard::HookFault::Panicked => {
                        crate::topology::HookFault::Failed(Error::permanent(format!(
                            "resident {hook_op} hook panicked — caught and isolated under \
                         panic=unwind (fan-out not crashed); inert under panic=abort"
                        )))
                    },
                    crate::hook_guard::HookFault::TimedOut => crate::topology::HookFault::TimedOut,
                })
            },
        };

        match result {
            Ok(()) => {
                if stale {
                    state.built_epoch = slot_epoch;
                }
                Ok(())
            },
            Err(e) => Err(e),
        }
    }
}

impl<R> Resident<R>
where
    R: crate::topology::resident::ResidentProvider + Send + Sync + 'static,
{
    /// Builds before replacing, preserving the old master on failure.
    async fn clone_or_create(
        &self,
        resource: &R,
        resource_config: &R::Config,
        ctx: &ResourceContext,
        retained: &crate::RetainedStore<Arc<R::Instance>>,
    ) -> Result<CreatedEntry<Arc<R::Instance>>, Error> {
        use crate::resource::ResourceConfig as _;
        let config_fingerprint = resource_config.fingerprint();
        let mut state = self.state.lock().await;
        if state.closed {
            return Err(Error::cancelled().with_resource_key(R::key()));
        }
        if let Some(existing) = state.master.and_then(|id| retained.lease(id)) {
            let config_unchanged = state.built_fingerprint == config_fingerprint;
            if resource.is_alive_sync(&existing) && config_unchanged {
                return Ok(CreatedEntry::new(Arc::clone(&existing)));
            }
            if config_unchanged && !self.config.recreate_on_failure {
                return Err(Error::transient("resident runtime is not alive"));
            }
        }
        let instance = match tokio::time::timeout(
            self.config.create_timeout,
            resource.create(resource_config, ctx),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => return Err(Error::transient("resident: create timed out")),
        };
        // No await after creation: the store absorbs all retained ownership,
        // including publication rejected by a concurrent terminal fence.
        let entry = Arc::new(instance);
        if let Some(id) = state.master {
            if retained.replace(id, Arc::clone(&entry)) != crate::ReplaceStatus::Replaced {
                return Err(Error::cancelled().with_resource_key(R::key()));
            }
        } else {
            match retained.retain(Arc::clone(&entry)) {
                crate::RetainStatus::Published(id) => state.master = Some(id),
                crate::RetainStatus::Retired(_) => {
                    return Err(Error::cancelled().with_resource_key(R::key()));
                },
            }
        }
        state.built_epoch = resource.credential_slot_epoch();
        state.built_fingerprint = config_fingerprint;
        Ok(CreatedEntry::new(entry))
    }
}

// ─── Topology impl for Resident ───────────────────────────────────────────────
//
// `Resident<R>` shares its retained master with `Entry = Arc<R::Instance>`;
// `pools() == false`, so the framework store stays empty and every acquire is an
// idle-miss that calls `create_entry` (clone-or-create). The revoke fence cannot
// reach the master (it is not in the store), so revoke policy runs
// through `dispatch_credential_hook`.

impl<R> Topology<R> for Resident<R>
where
    R: Provider<Topology = Resident<R>>
        + crate::topology::resident::ResidentProvider
        + Send
        + Sync
        + 'static,
{
    type Entry = Arc<R::Instance>;

    /// Always succeeds — resident is unbounded (one shared instance).
    fn try_reserve(&self, _store: StoreView<'_, Self::Entry>) -> Result<Ticket, Unavailable> {
        Ok(Ticket::infallible())
    }

    async fn create_entry(
        &self,
        resource: &R,
        config: &R::Config,
        ctx: &ResourceContext,
        retained: &crate::RetainedStore<Self::Entry>,
    ) -> Result<CreatedEntry<Self::Entry>, Error> {
        self.clone_or_create(resource, config, ctx, retained).await
    }

    fn entry_instance<'s>(&self, entry: &'s Self::Entry) -> &'s R::Instance {
        entry
    }

    fn into_owned_instance(&self, entry: Self::Entry) -> Option<R::Instance> {
        Arc::into_inner(entry)
    }

    async fn quiesce(&self) -> Result<(), Error> {
        let mut state = self.state.lock().await;
        state.closed = true;
        state.master = None;
        Ok(())
    }

    /// Resident does not pool: a released clone is dropped, never recycled, so
    /// the framework idle store stays empty.
    fn pools(&self) -> bool {
        false
    }

    /// Resident tears down its credential-bound master handle on revoke via the
    /// cell reconcile in [`dispatch_credential_hook`](Self::dispatch_credential_hook)
    /// (the master handle is never in the framework store, so the store fence
    /// cannot reach it). Declaring this lets the registration footgun-guard
    /// distinguish a correctly-revoke-handling non-pooling topology from an
    /// under-built custom one.
    fn handles_own_revoke(&self) -> bool {
        true
    }

    async fn dispatch_credential_hook(
        &self,
        resource: &R,
        _store: StoreView<'_, Self::Entry>,
        retained: &crate::RetainedStore<Self::Entry>,
        slot: &str,
        refresh: bool,
    ) -> Result<(), crate::topology::HookFault> {
        // The resident's master handle is NOT in the framework store, so the
        // store-fence cannot reach it: revoke / refresh teardown runs the
        // create-vs-rotate reconcile against the master cell instead.
        self.dispatch_resident_hook(resource, retained, slot, refresh)
            .await
    }

    fn tag(&self) -> TopologyTag {
        TopologyTag::Resident
    }
}

#[cfg(test)]
#[path = "resident_tests.rs"]
mod tests;
