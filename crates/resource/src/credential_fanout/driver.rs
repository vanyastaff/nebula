//! Production wiring that drives the [`ResourceFanoutIndex`] from the
//! credential-rotation / lease-revoke event streams.
//!
//! # Why this exists
//!
//! [`ResourceFanoutIndex`] is the reverse index + per-slot fan-out port.
//! Until this module, every `bind` / `dispatch_refresh` / `dispatch_revoke`
//! caller was a `#[cfg(test)]` test — the index was implemented but unwired.
//! This module closes that: it is the single production consumer that turns a
//! completed credential refresh / revoke into the typed
//! `nebula_resource::Manager` slot ports for every resolved resource row
//! that bound the rotated credential.
//!
//! # Layering (no `nebula-resource → nebula-engine` edge)
//!
//! The rotation/revoke *signals* originate in the credential-runtime
//! composition root, which owns the resolver, the `RefreshCoordinator`, and
//! the lease lifecycle. That crate must **not** depend on `nebula-resource`.
//! The signals reach this driver as plain [`nebula_eventbus`] events
//! ([`CredentialEvent`] on `EventBus<CredentialEvent>`,
//! [`LeaseEvent`] on `EventBus<LeaseEvent>`) — the cross-crate-signal-via-
//! eventbus rule, **not** a direct sibling import.
//!
//! Only the engine simultaneously holds `Arc<ResourceFanoutIndex>`
//! and the `Arc<nebula_resource::Manager>` the engine already owns, and
//! only the engine legitimately depends on `nebula-resource` downward.
//! So the fan-out driver is wired by the engine: it subscribes the two
//! credential buses and drives the typed `Manager` slot ports.
//!
//! # What it does, per event
//!
//! - [`CredentialEvent::Refreshed`] — the credential-runtime facade has
//!   already CAS-persisted the fresh material into the store before emitting
//!   this. That is exactly the "engine has stored the fresh material" point,
//!   so a resolver-enabled driver reconciles the stored material epoch for
//!   published rotation bindings before dispatching a hook. A slot omitted
//!   from the reverse index remains opted out even when it carries projection
//!   metadata. Without a resolver the driver uses the legacy
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

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nebula_core::CredentialKey;
use nebula_credential::{
    CredentialEvent, CredentialId, CredentialSlotResolver, LeaseEvent, TenantScope,
};
use nebula_eventbus::EventBus;

use crate::Manager;
use crate::credential_fanout::index::{ResourceFanoutIndex, RotationOutcome};

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

/// Handle for the background fan-out driver task.
///
/// Holding the handle keeps the task alive; dropping it (or calling
/// [`abort`](Self::abort)) cancels it, so the driver never outlives the
/// engine that started it. The task itself loops forever — this handle
/// is the only path to shutdown.
pub struct ResourceFanoutDriver {
    handle: tokio::task::JoinHandle<()>,
    reconciliation_lease: Arc<
        std::sync::Mutex<Option<crate::credential_fanout::index::AuthoritativeReconciliationLease>>,
    >,
    lifecycle: Arc<DriverLifecycle>,
}

struct DriverLifecycle {
    stopped: std::sync::atomic::AtomicBool,
    on_stopped: Arc<dyn Fn() + Send + Sync>,
}

impl DriverLifecycle {
    fn stop(&self) {
        if !self.stopped.swap(true, std::sync::atomic::Ordering::AcqRel) {
            (self.on_stopped)();
        }
    }
}

impl std::fmt::Debug for ResourceFanoutDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceFanoutDriver")
            .field("is_finished", &self.handle.is_finished())
            .finish()
    }
}

impl ResourceFanoutDriver {
    /// Spawn the driver: subscribe `credential_bus` (+ optionally
    /// `lease_bus`) and drive the typed `Manager` slot ports through
    /// `index` for every resolved resource row that bound a rotated /
    /// revoked credential.
    ///
    /// `index` and `manager` are the engine-held `Arc`s
    /// (`WorkflowEngine` owns both); `credential_bus` / `lease_bus`
    /// are the buses the credential-runtime composition root publishes
    /// on. `lease_bus` is optional because a deployment without
    /// dynamic-secret leases (no `LeasedProvider`) has no lease bus —
    /// credential-level `CredentialEvent::Revoked` still drives revoke
    /// fan-out in that case.
    pub fn spawn(
        index: Arc<ResourceFanoutIndex>,
        manager: Arc<Manager>,
        credential_bus: Arc<EventBus<CredentialEvent>>,
        lease_bus: Option<Arc<EventBus<LeaseEvent>>>,
    ) -> Self {
        Self::spawn_with_resolver(index, manager, None, credential_bus, lease_bus)
    }

    /// Spawn the driver with owner-qualified material reprojection enabled.
    pub fn spawn_with_resolver(
        index: Arc<ResourceFanoutIndex>,
        manager: Arc<Manager>,
        resolver: Option<Arc<dyn CredentialSlotResolver>>,
        credential_bus: Arc<EventBus<CredentialEvent>>,
        lease_bus: Option<Arc<EventBus<LeaseEvent>>>,
    ) -> Self {
        Self::spawn_with_resolver_and_lifecycle(
            index,
            manager,
            resolver,
            credential_bus,
            lease_bus,
            Arc::new(|| {}),
        )
    }

    /// Spawns a resolver-backed driver and reports its terminal lifecycle once.
    ///
    /// This is an engine composition seam. `on_stopped` runs exactly once for
    /// natural task completion, task panic, [`abort`](Self::abort), or handle
    /// drop, after authoritative reconciliation availability is withdrawn.
    #[doc(hidden)]
    pub fn spawn_with_resolver_and_lifecycle(
        index: Arc<ResourceFanoutIndex>,
        manager: Arc<Manager>,
        resolver: Option<Arc<dyn CredentialSlotResolver>>,
        credential_bus: Arc<EventBus<CredentialEvent>>,
        lease_bus: Option<Arc<EventBus<LeaseEvent>>>,
        on_stopped: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        manager.attach_rotation_index(&index);
        let reconciliation_lease = resolver
            .as_ref()
            .map(|_| index.acquire_authoritative_reconciliation_for(&manager));
        let reconciliation_lease = Arc::new(std::sync::Mutex::new(reconciliation_lease));
        let task_reconciliation_lease = Arc::clone(&reconciliation_lease);
        let lifecycle = Arc::new(DriverLifecycle {
            stopped: std::sync::atomic::AtomicBool::new(false),
            on_stopped,
        });
        let task_lifecycle = Arc::clone(&lifecycle);
        let mut credential_sub = credential_bus.subscribe();
        let mut lease_sub = lease_bus.map(|bus| bus.subscribe());
        let handle = tokio::spawn(async move {
            let _driver_lifecycle_on_exit = scopeguard::guard(
                (task_reconciliation_lease, task_lifecycle),
                |(lease, lifecycle)| {
                    lease
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take();
                    lifecycle.stop();
                },
            );
            // Per-driver revoke dedupe: one logical credential revoke
            // double-emits (lease bus `LeaseRevoked`(s) + facade
            // `CredentialEvent::Revoked`); this collapses them within
            // `REVOKE_DEDUPE_WINDOW`. Owned by the loop task so it needs
            // no lock.
            let mut revoke_dedupe = RevokeDedupe::new();
            let mut reconciliation = tokio::time::interval(Duration::from_secs(30));
            reconciliation.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut scans = tokio::task::JoinSet::new();
            let mut material_dispatches = tokio::task::JoinSet::new();
            let mut revoke_retries = tokio::task::JoinSet::new();
            let mut revoke_dispatches = tokio::task::JoinSet::new();
            let mut pending_material =
                HashMap::<CredentialId, (TenantScope, CredentialKey, u64)>::new();
            let mut pending_refresh_scans = HashSet::<CredentialId>::new();
            let mut full_scan_requested = false;
            let mut revoke_retry_requested = false;
            loop {
                // `tokio::select!` over both subscribers so a refresh and
                // a lease-revoke are both observed promptly. The two
                // buses are independent `Arc`s: a closed *lease* bus does
                // not imply the *credential* bus is gone, so it degrades
                // to credential-only rather than retiring the whole driver
                // (mirrors the no-lease-bus deployment path below). Only
                // the credential bus closing (the composition root went
                // away — no further rotation signals possible) retires
                // the driver.
                tokio::select! {
                    ev = credential_sub.recv() => match ev {
                        Some(CredentialEvent::Refreshed { credential_id }) if resolver.is_some() => {
                            if pending_refresh_scans.contains(&credential_id)
                                || pending_refresh_scans.len() < MAX_PENDING_REFRESH_SCANS
                            {
                                pending_refresh_scans.insert(credential_id);
                            } else {
                                pending_refresh_scans.clear();
                                full_scan_requested = true;
                            }
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
                                &mut revoke_dedupe,
                                &mut revoke_dispatches,
                                credential_id,
                                true,
                                "credential bus",
                            );
                        },
                        Some(ev) => Self::on_credential_event(
                            &index, &manager, resolver.as_deref(), ev,
                        ).await,
                        None => break,
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
                                &mut revoke_dedupe,
                                &mut revoke_dispatches,
                                credential_id,
                                false,
                                "lease bus",
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
                    _ = reconciliation.tick() => {
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
                        revoke_retries.spawn(async move {
                            index.retry_pending_revoke_admissions(&manager).await
                        });
                    },
                    () = std::future::ready(()), if (full_scan_requested || !pending_refresh_scans.is_empty())
                        && scans.is_empty()
                        && material_dispatches.is_empty() => {
                        let credential_id = if full_scan_requested {
                            full_scan_requested = false;
                            pending_refresh_scans.clear();
                            None
                        } else {
                            let credential_id = pending_refresh_scans.iter().next().copied();
                            if let Some(credential_id) = credential_id {
                                pending_refresh_scans.remove(&credential_id);
                            }
                            credential_id
                        };
                        if let Some(resolver) = resolver.as_ref() {
                            let index = Arc::clone(&index);
                            let manager = Arc::clone(&manager);
                            let resolver = Arc::clone(resolver);
                            // JoinSet aborts the scan when the driver is dropped.
                            // Scans cannot delay reception of revoke observations.
                            scans.spawn(async move {
                                index
                                    .reconcile_material(&manager, resolver.as_ref(), credential_id)
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
                                // One background material fan-out at a time keeps the
                                // per-row projection limit global to this driver while
                                // the receive loop remains free to admit revokes.
                                material_dispatches.spawn(async move {
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
                "resource rotation fan-out driver stopped: credential signal bus closed"
            );
        });
        Self {
            handle,
            reconciliation_lease,
            lifecycle,
        }
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
                // scan by the receive loop. Only the legacy driver reaches here.
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
            // Not a rotation of stored material. This driver is only a
            // resource-material fan-out observer; it has no credential
            // aggregate write authority.
            // `CredentialEvent` is `#[non_exhaustive]`; any future
            // additive variant defaults to "not a rotation/revoke of
            // resolved material" until a unit deliberately wires it.
            CredentialEvent::ReauthRequired { .. } => {},
            _ => {},
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
        revoke_dedupe: &mut RevokeDedupe,
        revoke_dispatches: &mut tokio::task::JoinSet<()>,
        credential_id: CredentialId,
        retain_terminal_credential_revoke: bool,
        source: &'static str,
    ) {
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
                    source,
                    "credential-level revoke followed a deduped lease revoke; \
                     rescanning published rows with terminal authority"
                );
            } else {
                tracing::debug!(
                    target: "nebula_resource::credential_fanout",
                    %credential_id,
                    source,
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
        revoke_dispatches.spawn(async move {
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
        self.reconciliation_lease
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        self.handle.abort();
        self.lifecycle.stop();
    }

    /// Whether the underlying task has finished (e.g. via abort or a
    /// closed signal bus).
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
}

impl Drop for ResourceFanoutDriver {
    fn drop(&mut self) {
        // Cancel the spawned task so the driver never outlives the
        // engine that started it.
        self.reconciliation_lease
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        self.handle.abort();
        self.lifecycle.stop();
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
    async fn abort_releases_authoritative_reconciliation_availability() {
        let index = Arc::new(ResourceFanoutIndex::new());
        let driver = ResourceFanoutDriver::spawn_with_resolver(
            Arc::clone(&index),
            Arc::new(Manager::new()),
            Some(Arc::new(NeverResolver)),
            Arc::new(EventBus::new(8)),
            None,
        );
        assert!(index.authoritative_reconciliation_available());

        driver.abort();

        assert!(!index.authoritative_reconciliation_available());
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

        ResourceFanoutDriver::spawn_revoke_deduped(
            &index,
            &manager,
            &mut dedupe,
            &mut dispatches,
            credential_id,
            false,
            "lease bus",
        );
        ResourceFanoutDriver::spawn_revoke_deduped(
            &index,
            &manager,
            &mut dedupe,
            &mut dispatches,
            credential_id,
            true,
            "credential bus",
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
