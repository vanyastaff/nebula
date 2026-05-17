//! Production wiring that drives the [`ResourceFanoutIndex`] from the
//! credential-rotation / lease-revoke event streams.
//!
//! # Why this exists
//!
//! [`ResourceFanoutIndex`] is the engine-owned reverse index + per-slot
//! fan-out port (ADR-0067 §D1). Until this module, every
//! `bind` / `dispatch_refresh` / `dispatch_revoke` caller was a
//! `#[cfg(test)]` test — the index was implemented but unwired (ADR-0067
//! §Deferred "Rotation fan-out is implemented but unwired"). This module
//! closes that: it is the single production consumer that turns a
//! completed credential refresh / revoke into the typed
//! `nebula_resource::Manager` slot ports for every resolved resource row
//! that bound the rotated credential.
//!
//! # Layering (no `nebula-resource → nebula-engine` edge)
//!
//! The rotation/revoke *signals* originate in the credential-runtime
//! composition root (`nebula-credential-runtime`, ADR-0066), which owns
//! the resolver, the [`RefreshCoordinator`](super::super::RefreshCoordinator),
//! and the lease lifecycle. That crate must **not** depend on
//! `nebula-resource` (deny.toml `[[bans]]` `nebula-resource` wrapper
//! allowlist; ADR-0067 §D1). The signals reach this driver as plain
//! [`nebula-eventbus`](nebula_eventbus) events
//! ([`CredentialEvent`] on `EventBus<CredentialEvent>`,
//! [`LeaseEvent`] on `EventBus<LeaseEvent>`) — the AGENTS.md
//! cross-crate-signal-via-eventbus rule, **not** a direct sibling import.
//!
//! Only `nebula-engine` simultaneously holds `Arc<ResourceFanoutIndex>`
//! and the `Arc<nebula_resource::Manager>` the engine already owns, and
//! only `nebula-engine` legitimately depends on `nebula-resource`
//! downward. So the fan-out driver is engine-owned: it subscribes the
//! two credential buses and drives the typed `Manager` slot ports.
//!
//! # What it does, per event
//!
//! - [`CredentialEvent::Refreshed`] — the credential-runtime facade has
//!   already CAS-persisted the fresh material into the store
//!   (`CredentialService::refresh`) before emitting this. That is exactly
//!   the ADR-0067 §D1 "engine has stored the fresh material" point, so
//!   the driver calls
//!   [`ResourceFanoutIndex::dispatch_refresh`].
//! - [`CredentialEvent::Revoked`] and
//!   [`LeaseEvent::LeaseRevoked`] — the credential / dynamic-secret lease
//!   was revoked (`CredentialService::revoke` →
//!   `LeaseLifecycle::revoke_for_credential` → the lease scheduler emits
//!   `LeaseRevoked`; the facade additionally emits
//!   `CredentialEvent::Revoked`). Either triggers
//!   [`ResourceFanoutIndex::dispatch_revoke`]. The fan-out itself is
//!   already two-phase + cancellation-safe internally (ADR-0067
//!   §Deferred / #681): synchronous `taint_slot_for` outside the
//!   per-resource timeout, then the timeout-wrapped
//!   `drain_and_revoke` tail. This driver does **not** re-implement
//!   that — it only invokes `dispatch_revoke`.
//!
//! A `LeaseRevoked` whose `credential_id` is `None` (an orphan lease
//! tracked without a nebula credential record — `LeaseEvent` doc) cannot
//! address a reverse-index row, so it is a no-op fan-out (logged at
//! `debug`, not an error).
//!
//! # Observability (ADR-0028 §4 — eventbus is not audit)
//!
//! Each dispatch returns a [`RotationOutcome`] aggregate. It is **never
//! silently dropped**: a credential-data-free `tracing` event records
//! `credential_id` / counts, and a non-zero `failed` / `timed_out`
//! escalates to `warn!`. Per ADR-0028 §4 this aggregate is a
//! metrics/observability signal **only** — it is *not* an audit write
//! and is *not* re-emitted on an eventbus (that dashboard emission stays
//! a deferred Non-goal of this unit; ADR-0067 §Deferred
//! "`RotationOutcome` → eventbus emission"). The fan-out internals
//! already guarantee no credential/secret material reaches any span
//! (ADR-0030 §4); this driver adds only key-free counts.

use std::sync::Arc;
use std::time::Duration;

use nebula_credential::{CredentialEvent, CredentialId, LeaseEvent};
use nebula_eventbus::EventBus;
use nebula_resource::Manager;

use super::resource_fanout::{ResourceFanoutIndex, RotationOutcome};

/// Per-resource fan-out timeout budget applied to every resolved
/// resource row by [`ResourceFanoutIndex::dispatch_refresh`] /
/// [`dispatch_revoke`](ResourceFanoutIndex::dispatch_revoke).
///
/// There is no dedicated rotation-timeout knob on any engine config
/// today, so this driver pins the same 30s budget every other
/// engine-side credential I/O bound uses
/// (`executor::CREDENTIAL_TIMEOUT`,
/// `LeaseLifecycleConfig::provider_call_timeout` default,
/// `token_http::OAUTH_TOKEN_HTTP_TIMEOUT`) rather than inventing a
/// magic literal. It is a **per-resource** budget (one slow resource
/// cannot cascade-fail siblings — ADR-0036 invariant, enforced inside
/// the fan-out), not a global one.
const PER_RESOURCE_ROTATION_TIMEOUT: Duration = Duration::from_secs(30);

/// Handle for the background fan-out driver task.
///
/// Mirrors
/// [`ReclaimSweepHandle`](super::super::refresh::ReclaimSweepHandle):
/// holding the handle keeps the task alive; dropping it (or calling
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
    /// (`WorkflowEngine` owns both); `credential_bus` /  `lease_bus`
    /// are the buses the credential-runtime composition root publishes
    /// on (`EventMetricObserver::{event_bus, lease_bus}`). `lease_bus`
    /// is optional because a deployment without dynamic-secret leases
    /// (no `LeasedProvider`) has no lease bus — credential-level
    /// `CredentialEvent::Revoked` still drives revoke fan-out in that
    /// case.
    pub fn spawn(
        index: Arc<ResourceFanoutIndex>,
        manager: Arc<Manager>,
        credential_bus: Arc<EventBus<CredentialEvent>>,
        lease_bus: Option<Arc<EventBus<LeaseEvent>>>,
    ) -> Self {
        let mut credential_sub = credential_bus.subscribe();
        let mut lease_sub = lease_bus.map(|bus| bus.subscribe());
        let handle = tokio::spawn(async move {
            loop {
                // A `None` from either subscriber means the bus was
                // dropped (the credential-runtime composition root went
                // away) — no further rotation signals can arrive, so
                // the driver retires. `tokio::select!` over both so a
                // refresh and a lease-revoke are both observed promptly.
                match &mut lease_sub {
                    Some(lsub) => {
                        tokio::select! {
                            ev = credential_sub.recv() => match ev {
                                Some(ev) => {
                                    Self::on_credential_event(&index, &manager, ev).await;
                                },
                                None => break,
                            },
                            ev = lsub.recv() => match ev {
                                Some(ev) => {
                                    Self::on_lease_event(&index, &manager, ev).await;
                                },
                                None => break,
                            },
                        }
                    },
                    None => match credential_sub.recv().await {
                        Some(ev) => {
                            Self::on_credential_event(&index, &manager, ev).await;
                        },
                        None => break,
                    },
                }
            }
            tracing::debug!(
                target: "nebula_engine::credential::rotation",
                "resource rotation fan-out driver stopped: credential signal bus closed"
            );
        });
        Self { handle }
    }

    /// Route a `CredentialEvent`: `Refreshed` → refresh fan-out,
    /// `Revoked` → revoke fan-out. `ReauthRequired` (and any future
    /// additive variant) is not a rotation/revoke of stored material, so
    /// it is intentionally not fanned out.
    async fn on_credential_event(
        index: &ResourceFanoutIndex,
        manager: &Manager,
        ev: CredentialEvent,
    ) {
        match ev {
            CredentialEvent::Refreshed { credential_id } => {
                let outcome = index
                    .dispatch_refresh(credential_id, manager, PER_RESOURCE_ROTATION_TIMEOUT)
                    .await;
                Self::record(credential_id, "refresh", outcome);
            },
            CredentialEvent::Revoked { credential_id } => {
                let outcome = index
                    .dispatch_revoke(credential_id, manager, PER_RESOURCE_ROTATION_TIMEOUT)
                    .await;
                Self::record(credential_id, "revoke", outcome);
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
    /// `credential_id` drives a revoke fan-out. Renew/expiry/failure
    /// variants do not revoke stored credential material, and an orphan
    /// lease (`credential_id == None`) cannot address a reverse-index
    /// row.
    async fn on_lease_event(index: &ResourceFanoutIndex, manager: &Manager, ev: LeaseEvent) {
        if let LeaseEvent::LeaseRevoked { credential_id, .. } = ev {
            match credential_id {
                Some(cid) => {
                    let outcome = index
                        .dispatch_revoke(cid, manager, PER_RESOURCE_ROTATION_TIMEOUT)
                        .await;
                    Self::record(cid, "revoke", outcome);
                },
                None => {
                    // Orphan lease with no nebula credential record — no
                    // reverse-index row can be keyed by it (LeaseEvent
                    // doc). A no-op fan-out, not an error.
                    tracing::debug!(
                        target: "nebula_engine::credential::rotation",
                        "lease revoked for an orphan lease (no credential id); \
                         resource rotation fan-out skipped"
                    );
                },
            }
        }
    }

    /// Consume the [`RotationOutcome`] — **never silently dropped**
    /// (ADR-0028 §4: an observability signal, not an audit write and not
    /// an eventbus re-emission). Only `credential_id` + counts reach the
    /// span; the fan-out internals already guarantee no credential /
    /// secret material on any observability surface (ADR-0030 §4). A
    /// non-zero `failed` / `timed_out` escalates to `warn!` so a partial
    /// fan-out is operator-visible.
    fn record(credential_id: CredentialId, op: &'static str, outcome: RotationOutcome) {
        if outcome.dispatched() == 0 {
            // No resource row bound this credential — an expected no-op
            // for a credential no resource resolved.
            tracing::debug!(
                target: "nebula_engine::credential::rotation",
                %credential_id,
                op,
                "resource rotation fan-out: no bound resource rows (no-op)"
            );
            return;
        }
        if outcome.failed > 0 || outcome.timed_out > 0 {
            tracing::warn!(
                target: "nebula_engine::credential::rotation",
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
                target: "nebula_engine::credential::rotation",
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
        // engine that started it (mirrors `ReclaimSweepHandle`).
        self.handle.abort();
    }
}
