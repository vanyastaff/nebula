//! Configuration types for the [`Manager`](super::Manager).
//!
//! - [`ManagerConfig`] — manager construction parameters
//! - [`RegisterOptions`] — extended options for `register_*_with` shorthands
//! - [`ShutdownConfig`] — graceful shutdown tuning
//! - [`DrainTimeoutPolicy`] — what to do on drain timeout (#302)

use std::{sync::Arc, time::Duration};

use nebula_core::ScopeLevel;

use crate::{integration::AcquireResilience, recovery::gate::RecoveryGate};

/// Policy that controls what `graceful_shutdown` does when the
/// drain phase expires with handles still outstanding (#302).
///
/// Before this split, `graceful_shutdown` always proceeded to
/// `registry.clear()` even on timeout, dropping live `ManagedResource`s
/// while handles remained outstanding. That turned a cooperative shutdown
/// into a use-after-logical-drop. The policy makes the choice explicit.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DrainTimeoutPolicy {
    /// On drain timeout, return
    /// [`ShutdownError::DrainTimeout`](super::ShutdownError::DrainTimeout)
    /// **without** clearing the registry.
    /// Live handles remain valid and the caller decides what to do next.
    /// This is the default — it preserves the "graceful" guarantee.
    #[default]
    Abort,
    /// On drain timeout, log, clear the registry anyway, and report the
    /// outstanding-handle count in [`ShutdownReport`](super::ShutdownReport).
    /// Opt-in escape hatch for supervisors that must exit on a deadline
    /// regardless of cost.
    Force,
}

/// Configuration for graceful shutdown.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ShutdownConfig {
    /// How long to wait for in-flight handles to be released.
    pub drain_timeout: Duration,
    /// What to do on drain timeout. Default: [`DrainTimeoutPolicy::Abort`].
    pub on_drain_timeout: DrainTimeoutPolicy,
    /// Upper bound on how long Phase 4 will wait for release-queue
    /// workers to finish processing outstanding tasks.
    pub release_queue_timeout: Duration,
}

impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            drain_timeout: Duration::from_secs(30),
            on_drain_timeout: DrainTimeoutPolicy::Abort,
            release_queue_timeout: Duration::from_secs(10),
        }
    }
}

impl ShutdownConfig {
    /// Override the drain timeout, returning `self` for chaining.
    ///
    /// `#[non_exhaustive]` prevents external crates from using struct
    /// literal construction; this (and the sibling setters) is the
    /// forward-compatible entry point for per-field customization.
    #[must_use]
    pub fn with_drain_timeout(mut self, timeout: Duration) -> Self {
        self.drain_timeout = timeout;
        self
    }

    /// Override the drain-timeout policy.
    #[must_use]
    pub fn with_drain_timeout_policy(mut self, policy: DrainTimeoutPolicy) -> Self {
        self.on_drain_timeout = policy;
        self
    }

    /// Override the release-queue timeout budget for Phase 4.
    #[must_use]
    pub fn with_release_queue_timeout(mut self, timeout: Duration) -> Self {
        self.release_queue_timeout = timeout;
        self
    }
}

/// Configuration for the [`Manager`](super::Manager).
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    /// Number of background workers for the release queue.
    ///
    /// Defaults to 2.
    pub release_queue_workers: usize,
    /// Optional shared metrics registry for telemetry counters.
    ///
    /// When `Some`, the manager records resource operation counters
    /// (`acquire_total`, `release_total`, etc.) into the registry.
    /// When `None`, metrics are silently skipped (zero overhead).
    pub metrics_registry: Option<Arc<nebula_metrics::MetricsRegistry>>,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            release_queue_workers: 2,
            metrics_registry: None,
        }
    }
}

/// Extended options for resource registration.
///
/// Used with the `register_*_with` convenience methods to configure
/// resilience and recovery beyond the simple `register_*` defaults.
///
/// Per ADR-0044, credential bindings are no longer threaded through
/// registration: the `resource: R` value handed to `Manager::register*`
/// already carries resolved credentials in its slot fields. The
/// [`slot_identity`](Self::slot_identity) here is a *stable hash over those
/// resolved bindings* (per ADR-0036 / ADR-0044) — it does **not** carry
/// secrets; it only lets the registry keep two resolved-credential
/// registrations on **separate rows** so they cannot share one runtime
/// (cross-tenant bleed). Compute it with
/// [`slot_identity`](crate::dedup::slot_identity).
///
/// `#[non_exhaustive]`: like the sibling [`ShutdownConfig`] /
/// [`DrainTimeoutPolicy`], new tuning fields must be additive without a
/// breaking struct-literal change. Construct via
/// [`RegisterOptions::default`] then the `with_*` setters.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RegisterOptions {
    /// Scope level for the resource (default: `Global`).
    pub scope: ScopeLevel,
    /// Optional acquire resilience (timeout + retry + circuit breaker).
    pub resilience: Option<AcquireResilience>,
    /// Optional recovery gate for thundering-herd prevention.
    pub recovery_gate: Option<Arc<RecoveryGate>>,
    /// Stable hash over this registration's resolved per-slot credential
    /// bindings. Defaults to
    /// [`SLOT_IDENTITY_UNBOUND`](crate::dedup::SLOT_IDENTITY_UNBOUND) — a
    /// registration with no resolved slots keeps the historical
    /// single-row-per-`(key, scope)` dedup behaviour. Set this (via
    /// [`with_slot_identity`](Self::with_slot_identity)) so two different
    /// resolved credentials at the same key+scope get distinct runtimes.
    pub slot_identity: u64,
}

impl Default for RegisterOptions {
    fn default() -> Self {
        Self {
            scope: ScopeLevel::Global,
            resilience: None,
            recovery_gate: None,
            slot_identity: crate::dedup::SLOT_IDENTITY_UNBOUND,
        }
    }
}

impl RegisterOptions {
    /// Override the scope level for this registration.
    #[must_use]
    pub fn with_scope(mut self, scope: ScopeLevel) -> Self {
        self.scope = scope;
        self
    }

    /// Attach an acquire-resilience policy for this registration.
    #[must_use]
    pub fn with_resilience(mut self, resilience: AcquireResilience) -> Self {
        self.resilience = Some(resilience);
        self
    }

    /// Attach a recovery gate for this registration.
    #[must_use]
    pub fn with_recovery_gate(mut self, gate: Arc<RecoveryGate>) -> Self {
        self.recovery_gate = Some(gate);
        self
    }

    /// Pin this registration to a resolved per-slot credential identity.
    ///
    /// Two registrations of the same resource type at the same scope with
    /// **different** `slot_identity` occupy distinct registry rows with
    /// distinct runtimes — the structural barrier against cross-tenant
    /// runtime bleed. Compute the value from the resolved slot bindings via
    /// [`slot_identity`](crate::dedup::slot_identity).
    #[must_use]
    pub fn with_slot_identity(mut self, slot_identity: u64) -> Self {
        self.slot_identity = slot_identity;
        self
    }
}
