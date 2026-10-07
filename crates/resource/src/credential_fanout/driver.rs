//! Reconciles credential bindings through the engine-owned resource manager.
//!
//! Durable credential projection is authoritative. Bounded background scans
//! reconcile published bindings at jittered intervals of at most 30 seconds.
//! Optional credential and lease events accelerate reconciliation; their loss
//! or closure cannot stop resolver-backed scans. Signal-only technical hosts
//! may use the hook path without a resolver.
//!
//! The host retains and supervises the driver. Cancellation drops all tracked
//! child futures before its stop callback permits a replacement driver.
//!
//! # What it does, per event
//!
//! - [`CredentialEvent::Refreshed`] — the credential-runtime facade has
//!   already CAS-persisted the fresh material into the store before emitting
//!   this. That is exactly the "engine has stored the fresh material" point,
//!   so a resolver-enabled driver reconciles the stored material epoch for
//!   published rotation bindings before dispatching a hook. A slot omitted
//!   from the reverse index remains opted out even when it carries projection
//!   metadata. Without a resolver the driver uses the
//!   [`ResourceFanoutIndex::dispatch_refresh`] hook-only path.
//! - [`CredentialEvent::Revoked`] and [`LeaseEvent::LeaseRevoked`] — the
//!   credential / dynamic-secret lease was revoked. Either triggers
//!   [`ResourceFanoutIndex::dispatch_revoke`]. The fan-out itself is already
//!   two-phase + cancellation-safe internally: synchronous
//!   `taint_slot_for` before the awaited tail, then queue-owned hook
//!   settlement after admission. This driver does **not**
//!   re-implement that — it only invokes `dispatch_revoke`.
//!   Queue-rejected admissions are retried by an independent periodic task,
//!   so their drain and hook-observation budgets cannot block material scans.
//!
//! A `LeaseRevoked` whose `credential_id` is `None` (an orphan lease
//! tracked without a nebula credential record) cannot address a
//! reverse-index row, so it is a no-op fan-out (logged at `debug`).
//!
//! # Observability
//!
//! Each dispatch returns a [`RotationOutcome`] aggregate. It is **never
//! silently dropped**: a credential-data-free `tracing` event records
//! `credential_id` / counts, and a non-zero `failed` / `timed_out`
//! escalates to `warn!`. This is a metrics/observability signal **only** —
//! it is *not* an audit write and is *not* re-emitted on an eventbus. The
//! fan-out internals already guarantee no credential/secret material reaches
//! any span; this driver adds only key-free counts.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nebula_core::CredentialKey;
use nebula_credential::{
    CredentialEvent, CredentialId, CredentialSlotResolver, LeaseEvent, TenantScope,
};
use nebula_eventbus::EventBus;

use crate::Manager;
use crate::credential_fanout::index::{
    AuthoritativeReconciliationLease, ResourceFanoutIndex, RotationOutcome,
};
use crate::credential_fanout::orchestrator::ScanHint;

/// Time window in which a second revoke for the same `CredentialId` is
/// treated as a duplicate of the first and skipped.
///
/// **Why this exists.** A single logical credential revoke surfaces on
/// *two* independent buses. `CredentialService::revoke` first calls
/// `LeaseLifecycle::revoke_for_credential`, which makes the lease
/// scheduler emit one [`LeaseEvent::LeaseRevoked`] **per released lease**,
/// and *then* the facade emits one [`CredentialEvent::Revoked`]. The
/// driver subscribes both, so without dedupe a single revoke would invoke
/// [`ResourceFanoutIndex::dispatch_revoke`] (and therefore every bound
/// resource's `on_credential_revoke` hook) two-or-more times for one
/// logical event: non-idempotent hooks double-fire and the
/// [`RotationOutcome`] metrics are inflated.
///
/// The window is a few seconds — comfortably longer than the gap between
/// the lease-scheduler `LeaseRevoked` emission(s) and the facade
/// `CredentialEvent::Revoked` for the *same* revoke (both happen inline
/// inside one `CredentialService::revoke` call), yet short enough that a
/// genuinely *new* revoke of the same credential after re-registration is
/// not suppressed. Taint is idempotent and a re-revoke after the window
/// is harmless, so erring slightly long is safe; erring short would
/// re-introduce the double-fire. Refresh does not use this time window:
/// resolver-enabled drivers gate refresh hooks by the durable material epoch.
const REVOKE_DEDUPE_WINDOW: Duration = Duration::from_secs(5);

/// Bounded last-seen set of recently-dispatched credential revokes, used
/// to collapse the lease-bus + credential-bus double-emission of one
/// logical revoke (see [`REVOKE_DEDUPE_WINDOW`]).
///
/// A small FIFO of `(CredentialId, dispatched_at)`: every revoke prunes
/// entries older than the window, then either observes its `cid` already
/// present (a duplicate — skipped) or records it and proceeds. Capacity
/// is bounded so a long-lived driver under heavy revoke churn cannot grow
/// it without limit; the oldest entry is evicted past the cap (it is
/// necessarily the least likely to still be inside the window).
///
/// # Upstream invariants this safety relies on
///
/// A **time-windowed** dedupe (rather than an exact once-per-logical-revoke
/// tracker) is only safe because the credential-runtime facade guarantees a
/// given [`CredentialId`] can never legitimately produce a *second*,
/// independent revoke signal after its first:
///
/// - **Facade tombstone CAS.** `CredentialService::revoke` transitions the
///   stored credential to a tombstoned state via a single compare-and-swap;
///   the transition happens at most once per credential, so the two signals
///   this dedupe collapses (`LeaseRevoked` × N + `CredentialEvent::Revoked`)
///   are always the double-emission of *one* logical CAS, never two distinct
///   revokes racing each other.
/// - **Re-revoke ⇒ `NotFound`.** A second `revoke` call against an
///   already-tombstoned credential fails closed (the facade reports the
///   credential as gone, not "revoked again") — it never re-emits
///   `CredentialEvent::Revoked` for the same id, so this dedupe can never
///   observe a genuine *new* revoke signal for a `CredentialId` it already
///   admitted.
/// - **Rebind-to-revoked fails.** A tombstoned `CredentialId` cannot be
///   reactivated and re-bound to accept a fresh revoke later — the id is
///   retired for good. So even a revoke arriving *after*
///   [`REVOKE_DEDUPE_WINDOW`] has elapsed for that id is either a very late
///   double-emission (harmless — taint is idempotent) or simply cannot
///   happen, never a legitimate second revoke this window would wrongly
///   suppress.
///
/// Together these mean the dedupe key space (`CredentialId`) is
/// write-once from this driver's point of view: it never needs to
/// distinguish "duplicate of an in-window revoke" from "a real second
/// revoke of the same id," because the latter is structurally impossible
/// upstream.
#[derive(Debug)]
struct RevokeDedupe {
    seen: VecDeque<(CredentialId, Instant)>,
}

impl RevokeDedupe {
    /// Hard cap on retained entries. The window is seconds-long and the
    /// duplicate pair arrives back-to-back, so a handful of slots covers
    /// the realistic concurrent-revoke fan-in; the cap only bounds a
    /// pathological burst.
    const MAX_ENTRIES: usize = 256;

    fn new() -> Self {
        Self {
            seen: VecDeque::new(),
        }
    }

    /// Records a revoke dispatch for `cid` at `now` and returns whether
    /// it should be **dispatched** (`true`) or **skipped as a duplicate**
    /// (`false`).
    ///
    /// Prunes entries older than [`REVOKE_DEDUPE_WINDOW`] first so the
    /// check is strictly time-bounded, then: if `cid` is still present it
    /// is a duplicate of an in-window dispatch (skip, do not refresh the
    /// timestamp — the window is anchored at the *first* dispatch); else
    /// it is recorded and dispatched.
    fn admit(&mut self, cid: CredentialId, now: Instant) -> bool {
        while let Some(&(_, ts)) = self.seen.front() {
            if now.duration_since(ts) >= REVOKE_DEDUPE_WINDOW {
                self.seen.pop_front();
            } else {
                break;
            }
        }
        if self.seen.iter().any(|&(seen_cid, _)| seen_cid == cid) {
            return false;
        }
        self.seen.push_back((cid, now));
        if self.seen.len() > Self::MAX_ENTRIES {
            self.seen.pop_front();
        }
        true
    }
}

/// Per-resource fan-out timeout budget applied to every resolved resource
/// row by [`ResourceFanoutIndex::dispatch_refresh`] /
/// [`ResourceFanoutIndex::dispatch_revoke`].
///
/// There is no dedicated rotation-timeout knob on any engine config
/// today, so this driver pins the same 30s budget every other
/// engine-side credential I/O bound uses
/// (`executor::CREDENTIAL_TIMEOUT`,
/// `LeaseLifecycleConfig::provider_call_timeout` default,
/// `token_http::OAUTH_TOKEN_HTTP_TIMEOUT`) rather than inventing a
/// magic literal. It is a **per-resource** budget (one slow resource
/// cannot cascade-fail siblings — invariant, enforced inside
/// the fan-out), not a global one.
const PER_RESOURCE_ROTATION_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum distinct credential replacements retained while one material
/// projection fan-out is running. Repeated observations for one credential
/// replace its pending hint, and overflow falls back to durable reconciliation.
const MAX_PENDING_MATERIAL_REPLACEMENTS: usize = 256;
const MAX_PENDING_REFRESH_SCANS: usize = 256;

#[derive(Debug, Clone, Copy)]
enum RevokeSource {
    Credential,
    Lease,
}

/// Handle for the background fan-out driver task.
///
/// Holding the handle keeps the task alive; dropping it (or calling
/// [`abort`](Self::abort)) cancels it, so the driver never outlives the
/// engine that started it. The task itself loops forever — this handle
/// is the only path to shutdown.
pub struct ResourceFanoutDriver {
    handle: Option<tokio::task::JoinHandle<()>>,
    completed_join: Option<Result<(), tokio::task::JoinError>>,
    lifecycle: Arc<DriverLifecycle>,
}

struct DriverLifecycle {
    parent_stopped: std::sync::atomic::AtomicBool,
    active_children: std::sync::atomic::AtomicUsize,
    callback_fired: std::sync::atomic::AtomicBool,
    finished: std::sync::atomic::AtomicBool,
    completion: tokio::sync::Notify,
    on_stopped: Arc<dyn Fn() + Send + Sync>,
}

impl DriverLifecycle {
    fn reconciliation_child(
        self: &Arc<Self>,
        lease: AuthoritativeReconciliationLease,
    ) -> ReconciliationActivity {
        ReconciliationActivity {
            _lease: lease,
            _activity: self.child(),
        }
    }

    fn child(self: &Arc<Self>) -> DriverChildActivity {
        self.active_children
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        DriverChildActivity {
            lifecycle: Arc::clone(self),
        }
    }

    fn parent_stopped(&self) {
        self.parent_stopped
            .store(true, std::sync::atomic::Ordering::Release);
        self.try_finish();
    }

    fn child_stopped(&self) {
        let previous = self
            .active_children
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
        debug_assert!(previous > 0, "driver child activity count underflow");
        self.try_finish();
    }

    fn try_finish(&self) {
        if self
            .parent_stopped
            .load(std::sync::atomic::Ordering::Acquire)
            && self
                .active_children
                .load(std::sync::atomic::Ordering::Acquire)
                == 0
            && self
                .callback_fired
                .compare_exchange(
                    false,
                    true,
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Acquire,
                )
                .is_ok()
        {
            let _completion = scopeguard::guard((), |()| {
                self.finished
                    .store(true, std::sync::atomic::Ordering::Release);
                self.completion.notify_one();
            });
            (self.on_stopped)();
        }
    }
}

struct DriverChildActivity {
    lifecycle: Arc<DriverLifecycle>,
}

// Struct fields drop in declaration order, even before a captured future is
// first polled. Release authority before the last activity can finish lifecycle.
struct ReconciliationActivity {
    _lease: AuthoritativeReconciliationLease,
    _activity: DriverChildActivity,
}

impl Drop for DriverChildActivity {
    fn drop(&mut self) {
        self.lifecycle.child_stopped();
    }
}

/// Invalid resource reconciliation startup inputs.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ResourceFanoutSpawnError {
    /// Neither durable projection nor hint input is available.
    #[error("resource reconciliation requires a resolver or a hint bus")]
    MissingInputs,
    /// The reverse index is attached to another manager.
    #[error("resource fan-out index belongs to another manager")]
    ManagerAffinity,
}

impl std::fmt::Debug for ResourceFanoutDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceFanoutDriver")
            .field("is_finished", &self.is_finished())
            .finish()
    }
}

impl ResourceFanoutDriver {
    /// Starts durable credential reconciliation with optional wake hints.
    ///
    /// A resolver-backed driver continues scanning after hints close. A
    /// signal-only driver exits when its last bus closes. `on_stopped` runs
    /// once after the parent and all tracked children have stopped, including
    /// cancellation and panic. Rejected startup does not invoke the callback.
    ///
    /// # Errors
    /// Returns a typed error for missing inputs or conflicting manager affinity.
    #[doc(hidden)]
    pub fn try_spawn(
        index: Arc<ResourceFanoutIndex>,
        manager: Arc<Manager>,
        resolver: Option<Arc<dyn CredentialSlotResolver>>,
        credential_bus: Option<Arc<EventBus<CredentialEvent>>>,
        lease_bus: Option<Arc<EventBus<LeaseEvent>>>,
        on_stopped: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, ResourceFanoutSpawnError> {
        if resolver.is_none() && credential_bus.is_none() && lease_bus.is_none() {
            return Err(ResourceFanoutSpawnError::MissingInputs);
        }
        index
            .claim_manager_affinity(&manager)
            .map_err(|_| ResourceFanoutSpawnError::ManagerAffinity)?;
        manager.attach_rotation_index(&index);
        let reconciliation_lease = resolver
            .as_ref()
            .map(|_| index.acquire_authoritative_reconciliation_for(&manager))
            .transpose()
            .map_err(|_| ResourceFanoutSpawnError::ManagerAffinity)?;
        let lifecycle = Arc::new(DriverLifecycle {
            parent_stopped: std::sync::atomic::AtomicBool::new(false),
            active_children: std::sync::atomic::AtomicUsize::new(0),
            callback_fired: std::sync::atomic::AtomicBool::new(false),
            finished: std::sync::atomic::AtomicBool::new(false),
            completion: tokio::sync::Notify::new(),
            on_stopped,
        });
        let task_lifecycle = Arc::clone(&lifecycle);
        let mut credential_sub = credential_bus.map(|bus| bus.subscribe());
        let mut lease_sub = lease_bus.map(|bus| bus.subscribe());
        let driver_lifecycle_on_exit = scopeguard::guard(
            (reconciliation_lease, task_lifecycle),
            |(lease, lifecycle)| {
                drop(lease);
                lifecycle.parent_stopped();
            },
        );
        let driver_lifecycle = Arc::clone(&lifecycle);
        let handle = tokio::spawn(async move {
            let _driver_lifecycle_on_exit = driver_lifecycle_on_exit;
            // Per-driver revoke dedupe: one logical credential revoke
            // double-emits (lease bus `LeaseRevoked`(s) + facade
            // `CredentialEvent::Revoked`); this collapses them within
            // `REVOKE_DEDUPE_WINDOW`. Owned by the loop task so it needs
            // no lock.
            let mut revoke_dedupe = RevokeDedupe::new();
            let reconciliation = tokio::time::sleep(Duration::ZERO);
            tokio::pin!(reconciliation);
            let mut scans = tokio::task::JoinSet::new();
            let mut material_dispatches = tokio::task::JoinSet::new();
            let mut revoke_retries = tokio::task::JoinSet::new();
            let mut revoke_dispatches = tokio::task::JoinSet::new();
            let mut pending_material =
                HashMap::<CredentialId, (TenantScope, CredentialKey, u64)>::new();
            let mut pending_refresh_scans = HashMap::<CredentialId, ScanHint>::new();
            let mut full_scan_requested = false;
            let mut revoke_retry_requested = false;
            loop {
                if resolver.is_none() && credential_sub.is_none() && lease_sub.is_none() {
                    break;
                }
                tokio::select! {
                    ev = async {
                        match credential_sub.as_mut() {
                            Some(sub) => sub.recv().await,
                            None => std::future::pending().await,
                        }
                    } => match ev {
                        Some(CredentialEvent::Refreshed { credential_id }) if resolver.is_some() => {
                            Self::request_targeted_scan(
                                &mut pending_refresh_scans,
                                &mut full_scan_requested,
                                credential_id,
                                ScanHint::Refreshed,
                            );
                        },
                        // A credential that now needs reauthentication denies
                        // use at its current material: re-observe its rows
                        // promptly so they suspend before the periodic scan.
                        Some(CredentialEvent::ReauthRequired { credential_id, .. })
                            if resolver.is_some() =>
                        {
                            Self::request_targeted_scan(
                                &mut pending_refresh_scans,
                                &mut full_scan_requested,
                                credential_id,
                                ScanHint::Availability,
                            );
                        },
                        Some(CredentialEvent::MaterialReplaced {
                            credential_id,
                            scope,
                            credential_key,
                        }) if resolver.is_some() => {
                            let context_sequence = manager.remember_material_replacement(
                                &index,
                                credential_id,
                                scope.clone(),
                                credential_key.clone(),
                            );
                            if pending_material.contains_key(&credential_id)
                                || pending_material.len() < MAX_PENDING_MATERIAL_REPLACEMENTS
                            {
                                pending_material.insert(
                                    credential_id,
                                    (scope, credential_key, context_sequence),
                                );
                            } else {
                                full_scan_requested = true;
                                tracing::warn!(
                                    credential_id = %credential_id,
                                    "material replacement queue full; durable reconciliation requested"
                                );
                            }
                        },
                        Some(CredentialEvent::Revoked { credential_id }) => {
                            Self::spawn_revoke_deduped(
                                &index,
                                &manager,
                                &lifecycle,
                                &mut revoke_dedupe,
                                &mut revoke_dispatches,
                                credential_id,
                                RevokeSource::Credential,
                            );
                        },
                        Some(ev) => Self::on_credential_event(
                            &index, &manager, resolver.as_deref(), ev,
                        ).await,
                        None => credential_sub = None,
                    },
                    ev = async {
                        match lease_sub.as_mut() {
                            Some(sub) => sub.recv().await,
                            None => std::future::pending().await,
                        }
                    } => match ev {
                        Some(LeaseEvent::LeaseRevoked { credential_id: Some(credential_id), .. }) => {
                            Self::spawn_revoke_deduped(
                                &index,
                                &manager,
                                &lifecycle,
                                &mut revoke_dedupe,
                                &mut revoke_dispatches,
                                credential_id,
                                RevokeSource::Lease,
                            );
                        },
                        Some(LeaseEvent::LeaseRevoked { credential_id: None, .. }) => {
                            tracing::debug!(
                                target: "nebula_resource::credential_fanout",
                                "lease revoked for an orphan lease (no credential id); resource rotation fan-out skipped"
                            );
                        },
                        Some(_) => {},
                        None => lease_sub = None,
                    },
                    () = index.revoke_retry_notified() => {
                        revoke_retry_requested = true;
                    },
                    () = index.material_retry_notified(), if resolver.is_some() => {
                        full_scan_requested = true;
                    },
                    () = &mut reconciliation => {
                        reconciliation.as_mut().reset(tokio::time::Instant::now()
                            + crate::jitter::apply_jitter(Duration::from_secs(30), 0.5));
                        revoke_retry_requested = true;
                        if resolver.is_some() {
                            pending_refresh_scans.clear();
                            full_scan_requested = true;
                        }
                    },
                    () = std::future::ready(()), if revoke_retry_requested
                        && revoke_retries.is_empty() => {
                        revoke_retry_requested = false;
                        let index = Arc::clone(&index);
                        let manager = Arc::clone(&manager);
                        let child_activity = lifecycle.child();
                        revoke_retries.spawn(async move {
                            let _child_activity = child_activity;
                            index.retry_pending_revoke_admissions(&manager).await
                        });
                    },
                    () = std::future::ready(()), if (full_scan_requested || !pending_refresh_scans.is_empty())
                        && scans.is_empty()
                        && material_dispatches.is_empty() => {
                        let (credential_id, hint) = if full_scan_requested {
                            full_scan_requested = false;
                            pending_refresh_scans.clear();
                            (None, ScanHint::Availability)
                        } else {
                            let next = pending_refresh_scans
                                .iter()
                                .next()
                                .map(|(credential_id, hint)| (*credential_id, *hint));
                            if let Some((credential_id, _)) = next {
                                pending_refresh_scans.remove(&credential_id);
                            }
                            (next.map(|(credential_id, _)| credential_id), next.map_or(ScanHint::Availability, |(_, hint)| hint))
                        };
                        if let Some(resolver) = resolver.as_ref() {
                            let index = Arc::clone(&index);
                            let manager = Arc::clone(&manager);
                            let resolver = Arc::clone(resolver);
                            let Ok(reconciliation_lease) =
                                index.acquire_authoritative_reconciliation_for(&manager)
                            else {
                                tracing::error!(
                                    target: "nebula_resource::credential_fanout",
                                    "resource fan-out scan lost its established manager affinity"
                                );
                                continue;
                            };
                            let child_scope = lifecycle.reconciliation_child(reconciliation_lease);
                            // JoinSet aborts the scan when the driver is dropped.
                            // Scans cannot delay reception of revoke observations.
                            scans.spawn(async move {
                                let _child_scope = child_scope;
                                index
                                    .reconcile_material(
                                        &manager,
                                        resolver.as_ref(),
                                        credential_id,
                                        hint,
                                    )
                                    .await
                            });
                        }
                    },
                    () = std::future::ready(()), if !pending_material.is_empty()
                        && material_dispatches.is_empty()
                        && scans.is_empty() => {
                        if let Some(credential_id) = pending_material.keys().next().copied() {
                            let Some((scope, credential_key, context_sequence)) = pending_material.remove(&credential_id) else {
                                continue;
                            };
                            if let Some(resolver) = resolver.as_ref() {
                                let index = Arc::clone(&index);
                                let manager = Arc::clone(&manager);
                                let resolver = Arc::clone(resolver);
                                let Ok(reconciliation_lease) =
                                    index.acquire_authoritative_reconciliation_for(&manager)
                                else {
                                    tracing::error!(
                                        target: "nebula_resource::credential_fanout",
                                        "material fan-out lost its established manager affinity"
                                    );
                                    continue;
                                };
                                let child_scope = lifecycle.reconciliation_child(reconciliation_lease);
                                // One background material fan-out at a time keeps the
                                // per-row projection limit global to this driver while
                                // the receive loop remains free to admit revokes.
                                material_dispatches.spawn(async move {
                                    let _child_scope = child_scope;
                                    let outcome = index
                                        .dispatch_material_replacement(
                                            credential_id,
                                            &scope,
                                            &credential_key,
                                            resolver.as_ref(),
                                            &manager,
                                        )
                                        .await;
                                    (credential_id, context_sequence, outcome)
                                });
                            }
                        }
                    },
                    result = scans.join_next(), if !scans.is_empty() => {
                        match result {
                            Some(Ok(outcome)) if outcome.failed + outcome.timed_out + outcome.abandoned == 0 => {
                                tracing::debug!(?outcome, "credential projection reconciliation complete");
                            },
                            Some(Ok(outcome)) => tracing::warn!(?outcome, "credential projection reconciliation incomplete"),
                            Some(Err(_)) => tracing::warn!("credential projection reconciliation task failed"),
                            None => {},
                        }
                    },
                    result = revoke_retries.join_next(), if !revoke_retries.is_empty() => {
                        match result {
                            Some(Ok(outcome)) if outcome.failed + outcome.timed_out + outcome.abandoned == 0 => {
                                tracing::debug!(?outcome, "credential revoke admission reconciliation complete");
                            },
                            Some(Ok(outcome)) => tracing::warn!(?outcome, "credential revoke admission reconciliation incomplete"),
                            Some(Err(_)) => tracing::warn!("credential revoke admission reconciliation task failed"),
                            None => {},
                        }
                    },
                    result = revoke_dispatches.join_next(), if !revoke_dispatches.is_empty() => {
                        if matches!(result, Some(Err(_))) {
                            tracing::warn!("credential revoke fan-out task failed");
                        }
                    },
                    result = material_dispatches.join_next(), if !material_dispatches.is_empty() => {
                        match result {
                            Some(Ok((credential_id, context_sequence, outcome))) => {
                                if outcome.all_hooks_settled_successfully() {
                                    index.complete_material_context(
                                        &credential_id,
                                        context_sequence,
                                        &manager,
                                    );
                                }
                                Self::record(credential_id, "material_replacement", outcome);
                            },
                            Some(Err(_)) => tracing::warn!("material replacement fan-out task failed"),
                            None => {},
                        }
                    },
                }
            }
            tracing::debug!(
                target: "nebula_resource::credential_fanout",
                "resource rotation fan-out driver stopped: signal inputs closed"
            );
        });
        Ok(Self {
            handle: Some(handle),
            completed_join: None,
            lifecycle: driver_lifecycle,
        })
    }

    /// Route a `CredentialEvent`: `Refreshed` → refresh fan-out,
    /// `Revoked` → (deduped) revoke fan-out. `ReauthRequired` (and any
    /// future additive variant) is not a rotation/revoke of stored
    /// material, so it is intentionally not fanned out.
    async fn on_credential_event(
        index: &ResourceFanoutIndex,
        manager: &Manager,
        resolver: Option<&dyn CredentialSlotResolver>,
        ev: CredentialEvent,
    ) {
        match ev {
            CredentialEvent::Refreshed { credential_id } => {
                // Resolver-backed refresh hints are coalesced into the background
                // scan by the receive loop. Only a signal-only driver reaches here.
                let outcome = index
                    .dispatch_refresh(credential_id, manager, PER_RESOURCE_ROTATION_TIMEOUT)
                    .await;
                Self::record(credential_id, "refresh", outcome);
            },
            CredentialEvent::MaterialReplaced {
                credential_id,
                scope,
                credential_key,
            } => {
                let outcome = if let Some(resolver) = resolver {
                    index
                        .dispatch_material_replacement(
                            credential_id,
                            &scope,
                            &credential_key,
                            resolver,
                            manager,
                        )
                        .await
                } else {
                    tracing::warn!(
                        credential_id = %credential_id,
                        "material replacement fan-out has no credential resolver"
                    );
                    RotationOutcome::default()
                };
                Self::record(credential_id, "material_replacement", outcome);
            },
            // The receive loop handles revokes before this fallback so it can
            // taint synchronously and move the potentially long drain/hook tail
            // into a tracked background task.
            CredentialEvent::Revoked { .. } => {},
            // A resolver-backed driver turns this into a targeted
            // availability scan in the receive loop; a signal-only driver has no
            // way to observe availability and leaves it to the host. This
            // driver has no credential aggregate write authority.
            // `CredentialEvent` is `#[non_exhaustive]`; any future
            // additive variant defaults to "not a rotation/revoke of
            // resolved material" until a unit deliberately wires it.
            CredentialEvent::ReauthRequired { .. } => {},
            _ => {},
        }
    }

    /// Queues a targeted scan of `credential_id`, keeping the strongest hint
    /// already queued for it; a full queue collapses into one full scan.
    fn request_targeted_scan(
        pending: &mut HashMap<CredentialId, ScanHint>,
        full_scan_requested: &mut bool,
        credential_id: CredentialId,
        hint: ScanHint,
    ) {
        if let Some(queued) = pending.get_mut(&credential_id) {
            *queued = queued.merge(hint);
        } else if pending.len() < MAX_PENDING_REFRESH_SCANS {
            pending.insert(credential_id, hint);
        } else {
            pending.clear();
            *full_scan_requested = true;
        }
    }

    /// Revoke fan-out with the per-credential dedupe window applied.
    ///
    /// A single logical credential revoke surfaces on both buses
    /// (`LeaseEvent::LeaseRevoked` × N released leases, then
    /// `CredentialEvent::Revoked`). The first arrival within
    /// [`REVOKE_DEDUPE_WINDOW`] dispatches; later arrivals for the same
    /// `CredentialId` inside the window are skipped (debug-logged, the
    /// taint they would re-apply is already applied and idempotent). This
    /// keeps non-idempotent `on_credential_revoke` hooks single-fire and
    /// the [`RotationOutcome`] metrics un-inflated per logical revoke.
    fn spawn_revoke_deduped(
        index: &Arc<ResourceFanoutIndex>,
        manager: &Arc<Manager>,
        lifecycle: &Arc<DriverLifecycle>,
        revoke_dedupe: &mut RevokeDedupe,
        revoke_dispatches: &mut tokio::task::JoinSet<()>,
        credential_id: CredentialId,
        source: RevokeSource,
    ) {
        let retain_terminal_credential_revoke = matches!(source, RevokeSource::Credential);
        if !revoke_dedupe.admit(credential_id, Instant::now()) {
            // Lease and credential buses may describe the same logical
            // teardown. The lease arrival may win dedupe, but the later
            // credential event still carries stronger terminal authority
            // needed to fence future registration publication and to catch
            // rows published after the lease event's snapshot.
            if retain_terminal_credential_revoke {
                tracing::debug!(
                    target: "nebula_resource::credential_fanout",
                    %credential_id,
                    ?source,
                    "credential-level revoke followed a deduped lease revoke; \
                     rescanning published rows with terminal authority"
                );
            } else {
                tracing::debug!(
                    target: "nebula_resource::credential_fanout",
                    %credential_id,
                    ?source,
                    "resource rotation fan-out: duplicate lease revoke within dedupe window; \
                     skipped — first dispatch already tainted the bound rows"
                );
                return;
            }
        }
        let mut outcome =
            index.prepare_revoke(credential_id, manager, retain_terminal_credential_revoke);
        let index = Arc::clone(index);
        let manager = Arc::clone(manager);
        let child_activity = lifecycle.child();
        revoke_dispatches.spawn(async move {
            let _child_activity = child_activity;
            outcome.add(index.finish_prepared_revoke(credential_id, &manager).await);
            Self::record(credential_id, "revoke", outcome);
        });
    }

    /// Consume the [`RotationOutcome`] — **never silently dropped**. Only
    /// `credential_id` + counts reach the span; the fan-out internals already
    /// guarantee no credential / secret material on any observability surface.
    /// A non-zero `failed` / `timed_out` escalates to `warn!` so a partial
    /// fan-out is operator-visible.
    fn record(credential_id: CredentialId, op: &'static str, outcome: RotationOutcome) {
        if outcome.dispatched() == 0 {
            // No resource row bound this credential — an expected no-op
            // for a credential no resource resolved.
            tracing::debug!(
                target: "nebula_resource::credential_fanout",
                %credential_id,
                op,
                "resource rotation fan-out: no bound resource rows (no-op)"
            );
            return;
        }
        if outcome.failed > 0 || outcome.timed_out > 0 || outcome.abandoned > 0 {
            tracing::warn!(
                target: "nebula_resource::credential_fanout",
                %credential_id,
                op,
                success = outcome.success,
                failed = outcome.failed,
                timed_out = outcome.timed_out,
                deferred = outcome.deferred,
                abandoned = outcome.abandoned,
                drain_timed_out = outcome.drain_timed_out,
                observation_timed_out = outcome.observation_timed_out,
                dispatched = outcome.dispatched(),
                "resource rotation fan-out completed with non-success rows; \
                 siblings unaffected (per-resource isolation)"
            );
        } else if outcome.deferred > 0 {
            tracing::info!(
                target: "nebula_resource::credential_fanout",
                %credential_id,
                op,
                success = outcome.success,
                deferred = outcome.deferred,
                drain_timed_out = outcome.drain_timed_out,
                observation_timed_out = outcome.observation_timed_out,
                dispatched = outcome.dispatched(),
                "resource rotation fan-out accepted queue-owned deferred rows"
            );
        } else {
            tracing::info!(
                target: "nebula_resource::credential_fanout",
                %credential_id,
                op,
                success = outcome.success,
                drain_timed_out = outcome.drain_timed_out,
                observation_timed_out = outcome.observation_timed_out,
                dispatched = outcome.dispatched(),
                "resource rotation fan-out completed"
            );
        }
    }

    /// Abort the running driver task. Safe to call multiple times.
    pub fn abort(&self) {
        if let Some(handle) = &self.handle {
            handle.abort();
        }
    }

    /// Joins the driver and waits for every tracked child and stop callback.
    ///
    /// Cancellation of this wait preserves the completed parent outcome for
    /// the next wait. A terminal join error is returned once; later waits
    /// acknowledge the already joined driver with `Ok(())`.
    ///
    /// # Errors
    /// Returns the parent task's panic or cancellation error after quiescence.
    pub async fn wait(&mut self) -> Result<(), tokio::task::JoinError> {
        if let Some(handle) = self.handle.as_mut() {
            self.completed_join = Some(handle.await);
            self.handle = None;
        }
        while !self
            .lifecycle
            .finished
            .load(std::sync::atomic::Ordering::Acquire)
        {
            self.lifecycle.completion.notified().await;
        }
        self.completed_join.take().unwrap_or(Ok(()))
    }

    /// Whether the underlying task has finished (e.g. via abort or a
    /// closed signal bus).
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.handle
            .as_ref()
            .is_none_or(tokio::task::JoinHandle::is_finished)
    }
}

impl Drop for ResourceFanoutDriver {
    fn drop(&mut self) {
        // Cancel the spawned task so the driver never outlives the
        // engine that started it.
        self.abort();
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;

    use super::*;

    struct NeverResolver;

    impl CredentialSlotResolver for NeverResolver {
        fn resolve_slot<'a>(
            &'a self,
            _scope: &'a TenantScope,
            _credential_id: CredentialId,
            _expected_key: CredentialKey,
            _required_capabilities: nebula_credential::Capabilities,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> std::pin::Pin<
            Box<
                dyn Future<
                        Output = Result<
                            nebula_credential::ErasedCredentialGuard,
                            nebula_credential::CredentialSlotResolveError,
                        >,
                    > + Send
                    + 'a,
            >,
        > {
            Box::pin(std::future::pending())
        }
    }

    #[tokio::test]
    async fn aborting_an_unpolled_child_releases_authority_before_stop_callback() {
        let index = Arc::new(ResourceFanoutIndex::new());
        let manager = Arc::new(Manager::new());
        let saw_unavailable = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let callback_index = Arc::clone(&index);
        let callback_observation = Arc::clone(&saw_unavailable);
        let lifecycle = Arc::new(DriverLifecycle {
            parent_stopped: std::sync::atomic::AtomicBool::new(false),
            active_children: std::sync::atomic::AtomicUsize::new(0),
            callback_fired: std::sync::atomic::AtomicBool::new(false),
            finished: std::sync::atomic::AtomicBool::new(false),
            completion: tokio::sync::Notify::new(),
            on_stopped: Arc::new(move || {
                callback_observation.store(
                    !callback_index.authoritative_reconciliation_available(),
                    std::sync::atomic::Ordering::SeqCst,
                );
            }),
        });
        let lease = index
            .acquire_authoritative_reconciliation_for(&manager)
            .expect("affinity");
        let child_scope = lifecycle.reconciliation_child(lease);
        let mut children = tokio::task::JoinSet::new();
        children.spawn(async move {
            let _child_scope = child_scope;
            std::future::pending::<()>().await;
        });
        lifecycle.parent_stopped();
        assert!(
            !lifecycle
                .finished
                .load(std::sync::atomic::Ordering::Acquire)
        );
        children.abort_all();
        assert!(
            children
                .join_next()
                .await
                .expect("child")
                .expect_err("cancelled")
                .is_cancelled()
        );
        assert!(saw_unavailable.load(std::sync::atomic::Ordering::SeqCst));
        assert!(
            lifecycle
                .finished
                .load(std::sync::atomic::Ordering::Acquire)
        );
    }

    #[tokio::test]
    async fn missing_inputs_are_rejected_without_claiming_authority() {
        let index = Arc::new(ResourceFanoutIndex::new());
        let rejected = ResourceFanoutDriver::try_spawn(
            Arc::clone(&index),
            Arc::new(Manager::new()),
            None,
            None,
            None,
            Arc::new(|| {}),
        );
        assert!(matches!(
            rejected,
            Err(ResourceFanoutSpawnError::MissingInputs)
        ));
        assert!(!index.authoritative_reconciliation_available());
    }

    #[tokio::test]
    async fn cancelled_wait_preserves_parent_outcome_until_children_stop() {
        let mut driver = ResourceFanoutDriver::try_spawn(
            Arc::new(ResourceFanoutIndex::new()),
            Arc::new(Manager::new()),
            Some(Arc::new(NeverResolver)),
            None,
            None,
            Arc::new(|| {}),
        )
        .expect("resolver-only reconciliation");
        let child = driver.lifecycle.child();
        driver.abort();
        assert!(
            tokio::time::timeout(Duration::from_millis(25), driver.wait())
                .await
                .is_err()
        );
        assert!(
            driver.handle.is_none(),
            "parent has joined before waiting for children"
        );
        assert!(
            !driver
                .lifecycle
                .finished
                .load(std::sync::atomic::Ordering::Acquire)
        );
        drop(child);
        assert!(
            driver
                .wait()
                .await
                .expect_err("parent cancellation retained")
                .is_cancelled()
        );
        assert!(
            driver
                .lifecycle
                .finished
                .load(std::sync::atomic::Ordering::Acquire)
        );
        assert!(driver.wait().await.is_ok());
    }

    #[tokio::test]
    async fn resolver_reconciliation_survives_closed_credential_hints() {
        let index = Arc::new(ResourceFanoutIndex::new());
        let manager = Arc::new(Manager::new());
        let hints = Arc::new(EventBus::new(8));
        let driver = ResourceFanoutDriver::try_spawn(
            Arc::clone(&index),
            manager,
            Some(Arc::new(NeverResolver)),
            Some(Arc::clone(&hints)),
            None,
            Arc::new(|| {}),
        )
        .expect("valid resolver and manager");
        drop(hints);

        tokio::time::sleep(Duration::from_millis(25)).await;

        assert!(
            !driver.is_finished(),
            "closing an ephemeral hint bus must not stop durable reconciliation"
        );
        assert!(index.authoritative_reconciliation_available());
        driver.abort();
    }

    #[tokio::test]
    async fn abort_releases_authoritative_reconciliation_availability() {
        let index = Arc::new(ResourceFanoutIndex::new());
        let manager = Arc::new(Manager::new());
        let driver = ResourceFanoutDriver::try_spawn(
            Arc::clone(&index),
            Arc::clone(&manager),
            Some(Arc::new(NeverResolver)),
            Some(Arc::new(EventBus::new(8))),
            None,
            Arc::new(|| {}),
        )
        .expect("valid resolver and manager");
        assert!(index.authoritative_reconciliation_available());

        driver.abort();

        tokio::time::timeout(Duration::from_secs(1), async {
            while index.authoritative_reconciliation_available() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("aborted task destroys its authority lease");
    }

    #[tokio::test]
    async fn abort_keeps_authority_until_in_flight_reconciliation_is_cancelled() {
        let index = Arc::new(ResourceFanoutIndex::new());
        let manager = Arc::new(Manager::new());
        let driver = ResourceFanoutDriver::try_spawn(
            Arc::clone(&index),
            Arc::clone(&manager),
            Some(Arc::new(NeverResolver)),
            Some(Arc::new(EventBus::new(8))),
            None,
            Arc::new(|| {}),
        )
        .expect("valid resolver and manager");
        // Reconciliation tasks take their own scoped lease before entering
        // resolver I/O. Model a task paused at that boundary deterministically.
        let in_flight_reconciliation = index
            .acquire_authoritative_reconciliation_for(&manager)
            .expect("manager affinity");

        driver.abort();

        assert!(
            index.authoritative_reconciliation_available(),
            "driver cancellation must not demote rows while a scan can still publish completion"
        );
        drop(in_flight_reconciliation);
        tokio::time::timeout(Duration::from_secs(1), async {
            while index.authoritative_reconciliation_available() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the last cancelled scan releases authority and triggers fail-closed demotion");
    }

    #[tokio::test]
    async fn stopped_callback_waits_for_child_authority_to_be_destroyed() {
        let index = Arc::new(ResourceFanoutIndex::new());
        let manager = Arc::new(Manager::new());
        let callback_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let callback_saw_unavailable = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let callback_index = Arc::clone(&index);
        let callback_count_ref = Arc::clone(&callback_count);
        let callback_saw_unavailable_ref = Arc::clone(&callback_saw_unavailable);
        let lifecycle = Arc::new(DriverLifecycle {
            parent_stopped: std::sync::atomic::AtomicBool::new(false),
            active_children: std::sync::atomic::AtomicUsize::new(0),
            callback_fired: std::sync::atomic::AtomicBool::new(false),
            finished: std::sync::atomic::AtomicBool::new(false),
            completion: tokio::sync::Notify::new(),
            on_stopped: Arc::new(move || {
                callback_count_ref.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                callback_saw_unavailable_ref.store(
                    !callback_index.authoritative_reconciliation_available(),
                    std::sync::atomic::Ordering::SeqCst,
                );
            }),
        });
        let child_activity = lifecycle.child();
        let child_authority = index
            .acquire_authoritative_reconciliation_for(&manager)
            .expect("manager affinity");

        lifecycle.parent_stopped();
        assert_eq!(
            callback_count.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "parent shutdown cannot publish quiescence while a child future is alive"
        );

        drop(child_authority);
        drop(child_activity);
        assert_eq!(callback_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(callback_saw_unavailable.load(std::sync::atomic::Ordering::SeqCst));
        lifecycle.parent_stopped();
        assert_eq!(
            callback_count.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "terminal callback is exactly once"
        );
    }

    #[tokio::test]
    async fn manager_affinity_conflict_never_starts_a_live_driver() {
        let index = Arc::new(ResourceFanoutIndex::new());
        let first_manager = Arc::new(Manager::new());
        let other_manager = Arc::new(Manager::new());
        let credential_bus = Arc::new(EventBus::new(8));
        let first = ResourceFanoutDriver::try_spawn(
            Arc::clone(&index),
            first_manager,
            None,
            Some(Arc::clone(&credential_bus)),
            None,
            Arc::new(|| {}),
        )
        .expect("first manager claims index affinity");
        let conflict = ResourceFanoutDriver::try_spawn(
            Arc::clone(&index),
            Arc::clone(&other_manager),
            None,
            Some(Arc::clone(&credential_bus)),
            None,
            Arc::new(|| {}),
        );
        assert!(matches!(
            conflict,
            Err(ResourceFanoutSpawnError::ManagerAffinity)
        ));

        let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped_ref = Arc::clone(&stopped);
        let rejected = ResourceFanoutDriver::try_spawn(
            index,
            other_manager,
            None,
            Some(credential_bus),
            None,
            Arc::new(move || stopped_ref.store(true, std::sync::atomic::Ordering::SeqCst)),
        );
        assert!(matches!(
            rejected,
            Err(ResourceFanoutSpawnError::ManagerAffinity)
        ));
        assert!(!stopped.load(std::sync::atomic::Ordering::SeqCst));
        drop(first);
    }

    /// The lease-bus + credential-bus double-emission of one logical
    /// revoke (`LeaseRevoked` then `CredentialEvent::Revoked` for the same
    /// `CredentialId`, back-to-back) must collapse to a single dispatch
    /// inside the window. This is the pure-logic core of the
    /// `spawn_revoke_deduped` guard the driver applies.
    #[test]
    fn second_revoke_same_credential_within_window_is_skipped() {
        let mut d = RevokeDedupe::new();
        let cid = CredentialId::new();
        let t0 = Instant::now();

        // First (e.g. the lease-bus `LeaseRevoked`): dispatch.
        assert!(d.admit(cid, t0), "first revoke must dispatch");
        // The facade `CredentialEvent::Revoked` arrives back-to-back for
        // the SAME credential — a duplicate of one logical revoke: skip.
        assert!(
            !d.admit(cid, t0 + Duration::from_millis(1)),
            "the back-to-back second revoke for the same credential must be \
             skipped as a duplicate"
        );
        // A third in-window arrival (e.g. a second released lease's
        // `LeaseRevoked`) is also a duplicate.
        assert!(
            !d.admit(cid, t0 + Duration::from_millis(2)),
            "further in-window revokes for the same credential are duplicates"
        );
    }

    /// A different credential is never suppressed by another credential's
    /// in-window dispatch (the dedupe is strictly per-`CredentialId`).
    #[test]
    fn distinct_credentials_do_not_dedupe_each_other() {
        let mut d = RevokeDedupe::new();
        let a = CredentialId::new();
        let b = CredentialId::new();
        let t0 = Instant::now();
        assert!(d.admit(a, t0));
        assert!(
            d.admit(b, t0 + Duration::from_millis(1)),
            "a different credential's revoke must dispatch even within \
             another credential's window"
        );
    }

    #[tokio::test]
    async fn credential_event_retains_terminal_authority_after_lease_dedupe() {
        let index = Arc::new(ResourceFanoutIndex::new());
        let manager = Arc::new(Manager::new());
        let mut dedupe = RevokeDedupe::new();
        let mut dispatches = tokio::task::JoinSet::new();
        let credential_id = CredentialId::new();
        let lifecycle = Arc::new(DriverLifecycle {
            parent_stopped: std::sync::atomic::AtomicBool::new(false),
            active_children: std::sync::atomic::AtomicUsize::new(0),
            callback_fired: std::sync::atomic::AtomicBool::new(false),
            finished: std::sync::atomic::AtomicBool::new(false),
            completion: tokio::sync::Notify::new(),
            on_stopped: Arc::new(|| {}),
        });

        ResourceFanoutDriver::spawn_revoke_deduped(
            &index,
            &manager,
            &lifecycle,
            &mut dedupe,
            &mut dispatches,
            credential_id,
            RevokeSource::Lease,
        );
        ResourceFanoutDriver::spawn_revoke_deduped(
            &index,
            &manager,
            &lifecycle,
            &mut dedupe,
            &mut dispatches,
            credential_id,
            RevokeSource::Credential,
        );

        let bind = crate::Bind {
            resource_key: nebula_core::ResourceKey::new("future").expect("valid resource key"),
            scope: nebula_core::ScopeLevel::Global,
            slot_name: "auth".to_owned(),
            slot_identity: crate::SlotIdentity::from_bindings([("auth", "credential")]),
        };
        index.stage_bind(credential_id, bind.clone());
        assert!(index.publish_staged_entry(&credential_id, &bind));
        dispatches.abort_all();
    }

    /// A genuinely new revoke of the same credential *after* the window
    /// has elapsed (e.g. re-registered then revoked again) must dispatch —
    /// the dedupe collapses a double-emission, not a real later revoke.
    #[test]
    fn revoke_after_window_elapsed_dispatches_again() {
        let mut d = RevokeDedupe::new();
        let cid = CredentialId::new();
        let t0 = Instant::now();
        assert!(d.admit(cid, t0));
        assert!(
            !d.admit(cid, t0 + Duration::from_millis(10)),
            "still inside the window — duplicate"
        );
        assert!(
            d.admit(cid, t0 + REVOKE_DEDUPE_WINDOW + Duration::from_millis(1)),
            "a new revoke strictly after the dedupe window must dispatch"
        );
    }

    /// Pruning is time-bounded: stale entries are evicted on the next
    /// `admit`, so the set never retains entries older than the window
    /// and stays bounded under churn.
    #[test]
    fn stale_entries_are_pruned_on_admit() {
        let mut d = RevokeDedupe::new();
        let t0 = Instant::now();
        let first = CredentialId::new();
        assert!(d.admit(first, t0));
        // Many distinct revokes after the window — each prunes the now
        // stale older entries; the set cannot grow unbounded.
        for i in 1..32u64 {
            let cid = CredentialId::new();
            let t = t0 + REVOKE_DEDUPE_WINDOW + Duration::from_millis(i);
            assert!(d.admit(cid, t), "post-window distinct revoke dispatches");
        }
        assert!(
            d.seen.len() <= RevokeDedupe::MAX_ENTRIES,
            "the dedupe set must stay bounded"
        );
        assert!(
            d.seen.iter().all(|&(_, ts)| {
                let now = t0 + REVOKE_DEDUPE_WINDOW + Duration::from_millis(31);
                now.duration_since(ts) < REVOKE_DEDUPE_WINDOW
            }),
            "no retained entry may be older than the dedupe window"
        );
    }
}
