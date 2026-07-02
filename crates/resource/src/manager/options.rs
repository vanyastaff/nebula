//! Configuration types for the [`Manager`](super::Manager).
//!
//! - [`ManagerConfig`] — manager construction parameters
//! - [`RegistrationSpec`] — the single parameter aggregate consumed by
//!   [`Manager::register`](super::Manager::register)
//! - [`RegisterOptions`] — scope / recovery / slot-identity knobs
//! - [`ShutdownConfig`] — graceful shutdown tuning
//! - [`DrainTimeoutPolicy`] — what to do on drain timeout (#302)

use std::{sync::Arc, time::Duration};

use nebula_core::ScopeLevel;

use crate::{recovery::gate::RecoveryGate, resource::Provider};

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
    /// Upper bound on how long the release-queue drain phase will wait
    /// for release-queue workers to finish processing outstanding tasks.
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

    /// Override the release-queue timeout budget.
    #[must_use]
    pub fn with_release_queue_timeout(mut self, timeout: Duration) -> Self {
        self.release_queue_timeout = timeout;
        self
    }
}

/// Configuration for the [`Manager`](super::Manager).
///
/// `#[non_exhaustive]`: like the sibling [`ShutdownConfig`] /
/// [`RegisterOptions`], new tuning fields must be additive without a
/// breaking struct-literal change. Construct via [`ManagerConfig::default`]
/// then the `with_*` setters.
#[derive(Debug, Clone)]
#[non_exhaustive]
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
    /// Default acquire-slow-log threshold (sqlx `acquire_slow_threshold`
    /// precedent).
    ///
    /// `knob`: `acquire_slow_threshold` | `default`: `None` (no slow-acquire
    /// logging) | `rationale`: an acquire that takes unexpectedly long is a
    /// backend-health or pool-sizing signal an operator wants surfaced
    /// without wiring a histogram dashboard first. When `Some(threshold)`,
    /// an acquire whose total wall-clock time (lookup through guard mint or
    /// terminal failure) exceeds `threshold` emits one
    /// `tracing::warn!(target: "resource")` naming the resource key, scope,
    /// and elapsed time — **at most once per acquire** (the check runs once,
    /// at completion, not on a timer). This is the manager-wide default;
    /// [`AcquireOptions::with_acquire_slow_threshold`](crate::AcquireOptions::with_acquire_slow_threshold)
    /// overrides it per call. Distinct from
    /// [`AcquireOptions::deadline`](crate::AcquireOptions::deadline): the
    /// deadline is an enforced budget an acquire is cancelled/rejected for
    /// exceeding, while the slow threshold is purely observational — an
    /// acquire that succeeds slowly still succeeds, it is just logged. A
    /// threshold larger than the deadline never fires (the acquire fails
    /// from the deadline first); a threshold smaller than the deadline warns
    /// on a still-successful-but-sluggish acquire before the deadline would
    /// have failed it.
    pub acquire_slow_threshold: Option<Duration>,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            release_queue_workers: 2,
            metrics_registry: None,
            acquire_slow_threshold: None,
        }
    }
}

impl ManagerConfig {
    /// Override the number of background release-queue workers.
    #[must_use]
    pub fn with_release_queue_workers(mut self, workers: usize) -> Self {
        self.release_queue_workers = workers;
        self
    }

    /// Attach a shared metrics registry for telemetry counters.
    #[must_use]
    pub fn with_metrics_registry(mut self, registry: Arc<nebula_metrics::MetricsRegistry>) -> Self {
        self.metrics_registry = Some(registry);
        self
    }

    /// Sets the manager-wide default acquire-slow-log threshold. See
    /// [`Self::acquire_slow_threshold`] for the WARN contract and its
    /// interplay with [`AcquireOptions::deadline`](crate::AcquireOptions::deadline).
    #[must_use]
    pub fn with_acquire_slow_threshold(mut self, threshold: Duration) -> Self {
        self.acquire_slow_threshold = Some(threshold);
        self
    }
}

/// Extended options for resource registration.
///
/// Used with the `register_*_with` convenience methods to configure
/// recovery beyond the simple `register_*` defaults.
///
/// Per slot model, credential bindings are no longer threaded through
/// registration: the `resource: R` value handed to `Manager::register*`
/// already carries resolved credentials in its slot fields. The
/// [`slot_identity`](Self::slot_identity) here is the **collision-free
/// structural identity** over those resolved bindings (per credential
/// isolation / slot model) — it does **not** carry secrets; it only lets
/// the registry keep two resolved-credential registrations on **separate
/// rows** so they cannot share one runtime (cross-tenant bleed). Set it
/// from the resolved `(slot, credential)` pairs via
/// [`with_slot_bindings`](Self::with_slot_bindings).
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
    /// Optional recovery gate for thundering-herd prevention.
    pub recovery_gate: Option<Arc<RecoveryGate>>,
    /// Collision-free structural slot identity over this registration's
    /// resolved per-slot credential bindings. Defaults to
    /// [`SlotIdentity::Unbound`](crate::dedup::SlotIdentity) — a
    /// registration with no resolved slots keeps the
    /// single-row-per-`(key, scope)` dedup behaviour. Set it (via
    /// [`with_slot_bindings`](Self::with_slot_bindings)) so two different
    /// resolved credentials at the same key+scope get distinct runtimes.
    /// Equality is exact and structural (no digest), so two distinct
    /// resolved binding sets can never alias onto one row.
    pub slot_identity: crate::dedup::SlotIdentity,
}

impl RegisterOptions {
    /// The resolved-credential structural identity for this registration,
    /// for callers that express it through the options struct rather than
    /// building a [`RegistrationSpec`] directly.
    #[must_use]
    pub fn effective_slot_identity(&self) -> crate::dedup::SlotIdentity {
        self.slot_identity.clone()
    }
}

impl Default for RegisterOptions {
    fn default() -> Self {
        Self {
            scope: ScopeLevel::Global,
            recovery_gate: None,
            slot_identity: crate::dedup::SlotIdentity::Unbound,
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

    /// Attach a recovery gate for this registration.
    #[must_use]
    pub fn with_recovery_gate(mut self, gate: Arc<RecoveryGate>) -> Self {
        self.recovery_gate = Some(gate);
        self
    }

    /// Pin this registration to the **collision-free structural identity**
    /// of its resolved `(slot, credential)` bindings.
    ///
    /// Two registrations of the same resource type at the same scope whose
    /// resolved bindings differ occupy distinct registry rows with distinct
    /// runtimes; identical bindings collapse to one shared row. Equality is
    /// *exact and structural* (no digest), so two distinct resolved binding
    /// sets can never alias onto one row — the cross-tenant-bleed failure
    /// mode a collidable digest exposes is eliminated by construction. An
    /// empty binding set keeps the single-row-per-`(key, scope)` behaviour.
    #[must_use]
    pub fn with_slot_bindings(mut self, bindings: &[(&str, &str)]) -> Self {
        self.slot_identity =
            crate::dedup::SlotIdentity::from_bindings(bindings.iter().map(|(s, c)| (*s, *c)));
        self
    }
}

/// The single parameter aggregate consumed by
/// [`Manager::register`](super::Manager::register).
///
/// Collapses what used to be a 3-deep `register` → `register_with_identity`
/// → `register_with_slot_identity` delegation chain plus ~17 per-topology
/// `register_<topo>[_with]` shorthands into one struct fed to one funnel.
/// It is a plain struct with **public fields and no builder**: every field
/// names a registry row exactly, the one genuinely-optional policy
/// (`recovery_gate`) is `Option<Arc<RecoveryGate>>`, and `slot_identity`
/// defaults via [`SlotIdentity::Unbound`](crate::dedup::SlotIdentity) at
/// the construction site (no `Default` impl is possible — `R` / `R::Config`
/// are generic and not `Default`).
///
/// Per slot model the `resource: R` value is expected to have **all
/// `#[credential]` slot fields already resolved and populated** before it
/// reaches here; `Manager::register` does not resolve credential bindings.
///
/// `slot_identity` is the structural anti-bleed seam: two registrations of
/// the same resource type at the same `scope` whose resolved
/// `(slot, credential)` bindings differ occupy **distinct** registry rows
/// with **distinct** topology runtimes.
/// [`SlotIdentity::Unbound`](crate::dedup::SlotIdentity) preserves the
/// historical single-row-per-`(key, scope)` dedup contract. It carries no
/// secret bytes — only a stable identity over the resolved binding *names*.
pub struct RegistrationSpec<R: Provider> {
    /// The fully-constructed resource value, all credential slots resolved.
    pub resource: R,
    /// The validated-on-`register` resource config.
    pub config: R::Config,
    /// Scope level the row is keyed under.
    pub scope: ScopeLevel,
    /// Collision-free structural resolved-credential identity. Use
    /// [`SlotIdentity::Unbound`](crate::dedup::SlotIdentity) for the
    /// historical single-row-per-`(key, scope)` behaviour.
    pub slot_identity: crate::dedup::SlotIdentity,
    /// The resource's lease topology, by value. The topology *kind* is static
    /// per `R` (a Postgres is always `Pooled`); only its config (cap, sizes) is
    /// runtime, so callers construct the concrete
    /// [`Provider::Topology`] — e.g. `Pooled::new(pool_config, fingerprint)`
    /// — and hand it here.
    pub topology: R::Topology,
    /// Optional recovery gate for thundering-herd prevention.
    pub recovery_gate: Option<Arc<RecoveryGate>>,
}

impl<R: Provider> std::fmt::Debug for RegistrationSpec<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `resource: R` and `topology: R::Topology` are unconstrained
        // generics with no `Debug` bound (and may hold a live credential
        // guard behind a slot cell) — identify the spec by its registry
        // coordinates instead of the opaque payload.
        f.debug_struct("RegistrationSpec")
            .field("key", &R::key())
            .field("scope", &self.scope)
            .field("slot_identity", &self.slot_identity)
            .field("recovery_gate", &self.recovery_gate.is_some())
            .finish_non_exhaustive()
    }
}
