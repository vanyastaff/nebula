//! Configuration types for the [`Manager`](super::Manager).
//!
//! - [`ManagerConfig`] — manager construction parameters
//! - [`RegisterOptions`] — extended options for `register_*_with` shorthands
//! - [`ShutdownConfig`] — graceful shutdown tuning
//! - [`DrainTimeoutPolicy`] — what to do on drain timeout (#302)

use std::{sync::Arc, time::Duration};

use nebula_core::{CredentialId, ScopeLevel};

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
    pub metrics_registry: Option<Arc<nebula_telemetry::metrics::MetricsRegistry>>,
    /// Default per-resource timeout budget for credential rotation hooks
    /// (`on_credential_refresh` / `on_credential_revoke`).
    ///
    /// Each registered resource may override this via
    /// `RegisterOptions::credential_rotation_timeout` (Task 6). When a
    /// rotation hook exceeds the per-resource budget the dispatcher reports
    /// `RefreshOutcome::TimedOut` / `RevokeOutcome::TimedOut` and the
    /// remaining sibling dispatches continue unaffected (security amendment
    /// B-1: per-resource isolation).
    ///
    /// Defaults to 30 seconds.
    pub credential_rotation_timeout: Duration,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            release_queue_workers: 2,
            metrics_registry: None,
            credential_rotation_timeout: Duration::from_secs(30),
        }
    }
}

/// Extended options for resource registration.
///
/// Used with the `register_*_with` convenience methods to configure
/// resilience and recovery beyond the simple `register_*` defaults.
#[derive(Debug, Clone)]
pub struct RegisterOptions {
    /// Scope level for the resource (default: `Global`).
    pub scope: ScopeLevel,
    /// Optional acquire resilience (timeout + retry + circuit breaker).
    pub resilience: Option<AcquireResilience>,
    /// Optional recovery gate for thundering-herd prevention.
    pub recovery_gate: Option<Arc<RecoveryGate>>,
    /// Credential ID this resource binds to.
    ///
    /// Required for resources where `R::Credential != NoCredential` —
    /// `Manager::register` returns
    /// [`Error::missing_credential_id`](crate::Error::missing_credential_id)
    /// if a credential-bearing resource is registered without an ID. Ignored
    /// for `NoCredential`-bound resources (the manager logs a warning if one
    /// is supplied alongside `Credential = NoCredential`).
    ///
    /// Set via [`RegisterOptions::with_credential_id`].
    pub credential_id: Option<CredentialId>,
    /// Per-resource override for the default credential rotation timeout.
    ///
    /// `None` falls back to [`ManagerConfig::credential_rotation_timeout`]
    /// (default `30s`). Only meaningful for credential-bearing resources;
    /// ignored for `NoCredential`-bound resources.
    ///
    /// Set via [`RegisterOptions::with_rotation_timeout`].
    pub credential_rotation_timeout: Option<Duration>,
}

impl Default for RegisterOptions {
    fn default() -> Self {
        Self {
            scope: ScopeLevel::Global,
            resilience: None,
            recovery_gate: None,
            credential_id: None,
            credential_rotation_timeout: None,
        }
    }
}

impl RegisterOptions {
    /// Sets the credential ID this resource binds to.
    ///
    /// Required for credential-bearing resources (`R::Credential != NoCredential`).
    /// `Manager::register` errors with
    /// [`Error::missing_credential_id`](crate::Error::missing_credential_id)
    /// if a credential-bearing resource is registered without an ID.
    #[must_use]
    pub fn with_credential_id(mut self, id: CredentialId) -> Self {
        self.credential_id = Some(id);
        self
    }

    /// Overrides the default credential rotation timeout for this resource.
    ///
    /// Falls back to [`ManagerConfig::credential_rotation_timeout`] (default
    /// `30s`) when not set. Only meaningful for credential-bearing resources.
    #[must_use]
    pub fn with_rotation_timeout(mut self, timeout: Duration) -> Self {
        self.credential_rotation_timeout = Some(timeout);
        self
    }
}
