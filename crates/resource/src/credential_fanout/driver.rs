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
//!   so the driver calls [`ResourceFanoutIndex::dispatch_refresh`].
//! - [`CredentialEvent::Revoked`] and [`LeaseEvent::LeaseRevoked`] — the
//!   credential / dynamic-secret lease was revoked. Either triggers
//!   [`ResourceFanoutIndex::dispatch_revoke`]. The fan-out itself is already
//!   two-phase + cancellation-safe internally: synchronous
//!   `taint_slot_for` outside the per-resource timeout, then the
//!   timeout-wrapped `drain_and_revoke` tail. This driver does **not**
//!   re-implement that — it only invokes `dispatch_revoke`.
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

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nebula_credential::{CredentialEvent, CredentialId, LeaseEvent};
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
/// re-introduce the double-fire. Refresh is **not** deduped — a refresh
/// arrives on one bus only (`CredentialEvent::Refreshed`) and a real
/// re-refresh must always fan out.
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

/// Handle for the background fan-out driver task.
///
/// Holding the handle keeps the task alive; dropping it (or calling
/// [`abort`](Self::abort)) cancels it, so the driver never outlives the
/// engine that started it. The task itself loops forever — this handle
/// is the only path to shutdown.
pub struct ResourceFanoutDriver {
    handle: tokio::task::JoinHandle<()>,
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
        let mut credential_sub = credential_bus.subscribe();
        let mut lease_sub = lease_bus.map(|bus| bus.subscribe());
        let handle = tokio::spawn(async move {
            // Per-driver revoke dedupe: one logical credential revoke
            // double-emits (lease bus `LeaseRevoked`(s) + facade
            // `CredentialEvent::Revoked`); this collapses them within
            // `REVOKE_DEDUPE_WINDOW`. Owned by the loop task so it needs
            // no lock.
            let mut revoke_dedupe = RevokeDedupe::new();
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
                match &mut lease_sub {
                    Some(lsub) => {
                        tokio::select! {
                            ev = credential_sub.recv() => match ev {
                                Some(ev) => {
                                    Self::on_credential_event(
                                        &index, &manager, &mut revoke_dedupe, ev,
                                    ).await;
                                },
                                None => break,
                            },
                            ev = lsub.recv() => if let Some(ev) = ev {
                                Self::on_lease_event(
                                    &index, &manager, &mut revoke_dedupe, ev,
                                ).await;
                            } else {
                                // Lease bus closed but the credential bus
                                // is a separate live `Arc` — degrade to
                                // credential-only instead of retiring (a
                                // credential-level `CredentialEvent::Revoked`
                                // still drives revoke fan-out, exactly the
                                // no-lease-bus deployment path).
                                lease_sub = None;
                                tracing::debug!(
                                    target: "nebula_resource::credential_fanout",
                                    "resource rotation fan-out driver: lease signal \
                                     bus closed; degrading to credential-only \
                                     (CredentialEvent::Revoked still fans out revoke)"
                                );
                                continue;
                            },
                        }
                    },
                    None => match credential_sub.recv().await {
                        Some(ev) => {
                            Self::on_credential_event(&index, &manager, &mut revoke_dedupe, ev)
                                .await;
                        },
                        None => break,
                    },
                }
            }
            tracing::debug!(
                target: "nebula_resource::credential_fanout",
                "resource rotation fan-out driver stopped: credential signal bus closed"
            );
        });
        Self { handle }
    }

    /// Route a `CredentialEvent`: `Refreshed` → refresh fan-out,
    /// `Revoked` → (deduped) revoke fan-out. `ReauthRequired` (and any
    /// future additive variant) is not a rotation/revoke of stored
    /// material, so it is intentionally not fanned out.
    async fn on_credential_event(
        index: &ResourceFanoutIndex,
        manager: &Manager,
        revoke_dedupe: &mut RevokeDedupe,
        ev: CredentialEvent,
    ) {
        match ev {
            CredentialEvent::Refreshed { credential_id } => {
                // Refresh is intentionally NOT deduped: it arrives on one
                // bus only and a real re-refresh must always fan out.
                let outcome = index
                    .dispatch_refresh(credential_id, manager, PER_RESOURCE_ROTATION_TIMEOUT)
                    .await;
                Self::record(credential_id, "refresh", outcome);
            },
            CredentialEvent::Revoked { credential_id } => {
                Self::dispatch_revoke_deduped(
                    index,
                    manager,
                    revoke_dedupe,
                    credential_id,
                    "credential bus",
                )
                .await;
            },
            // Not a rotation of stored material — the sentinel/reauth
            // surface is consumed elsewhere; nothing to fan out.
            // `CredentialEvent` is `#[non_exhaustive]`; any future
            // additive variant defaults to "not a rotation/revoke of
            // resolved material" until a unit deliberately wires it.
            CredentialEvent::ReauthRequired { .. } => {},
            _ => {},
        }
    }

    /// Route a `LeaseEvent`: only `LeaseRevoked` with an attributed
    /// `credential_id` drives a (deduped) revoke fan-out. Renew/expiry/
    /// failure variants do not revoke stored credential material, and an
    /// orphan lease (`credential_id == None`) cannot address a
    /// reverse-index row.
    async fn on_lease_event(
        index: &ResourceFanoutIndex,
        manager: &Manager,
        revoke_dedupe: &mut RevokeDedupe,
        ev: LeaseEvent,
    ) {
        if let LeaseEvent::LeaseRevoked { credential_id, .. } = ev {
            match credential_id {
                Some(cid) => {
                    Self::dispatch_revoke_deduped(index, manager, revoke_dedupe, cid, "lease bus")
                        .await;
                },
                None => {
                    // Orphan lease with no nebula credential record — no
                    // reverse-index row can be keyed by it. A no-op
                    // fan-out, not an error.
                    tracing::debug!(
                        target: "nebula_resource::credential_fanout",
                        "lease revoked for an orphan lease (no credential id); \
                         resource rotation fan-out skipped"
                    );
                },
            }
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
    async fn dispatch_revoke_deduped(
        index: &ResourceFanoutIndex,
        manager: &Manager,
        revoke_dedupe: &mut RevokeDedupe,
        credential_id: CredentialId,
        source: &'static str,
    ) {
        if !revoke_dedupe.admit(credential_id, Instant::now()) {
            tracing::debug!(
                target: "nebula_resource::credential_fanout",
                %credential_id,
                source,
                "resource rotation fan-out: duplicate revoke within dedupe window \
                 (lease-bus + credential-bus double-emission of one logical revoke); \
                 skipped — first dispatch already tainted/drained the bound rows"
            );
            return;
        }
        let outcome = index
            .dispatch_revoke(credential_id, manager, PER_RESOURCE_ROTATION_TIMEOUT)
            .await;
        Self::record(credential_id, "revoke", outcome);
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
        if outcome.failed > 0 || outcome.timed_out > 0 {
            tracing::warn!(
                target: "nebula_resource::credential_fanout",
                %credential_id,
                op,
                success = outcome.success,
                failed = outcome.failed,
                timed_out = outcome.timed_out,
                dispatched = outcome.dispatched(),
                "resource rotation fan-out completed with non-success rows; \
                 siblings unaffected (per-resource isolation)"
            );
        } else {
            tracing::info!(
                target: "nebula_resource::credential_fanout",
                %credential_id,
                op,
                success = outcome.success,
                dispatched = outcome.dispatched(),
                "resource rotation fan-out completed"
            );
        }
    }

    /// Abort the running driver task. Safe to call multiple times.
    pub fn abort(&self) {
        self.handle.abort();
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
        self.handle.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lease-bus + credential-bus double-emission of one logical
    /// revoke (`LeaseRevoked` then `CredentialEvent::Revoked` for the same
    /// `CredentialId`, back-to-back) must collapse to a single dispatch
    /// inside the window. This is the pure-logic core of the
    /// `dispatch_revoke_deduped` guard the driver applies.
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
