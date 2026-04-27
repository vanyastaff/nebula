//! Resource lifecycle events.
//!
//! [`ResourceEvent`] captures significant lifecycle transitions for
//! observability and diagnostics. Events are emitted by the [`Manager`]
//! during registration, acquisition, release, and health-check operations.
//!
//! [`Manager`]: crate::manager::Manager

use std::time::Duration;

use nebula_core::{CredentialId, ResourceKey};

use crate::error::RotationOutcome;

/// A lifecycle event emitted by the resource manager.
///
/// Events are lightweight and cheap to clone. They carry enough
/// information for logging, metrics, and audit trails without
/// holding references to live resources.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ResourceEvent {
    /// A resource was successfully registered.
    Registered {
        /// The key of the registered resource.
        key: ResourceKey,
    },
    /// A resource was removed from the registry.
    Removed {
        /// The key of the removed resource.
        key: ResourceKey,
    },
    /// A resource was successfully acquired.
    AcquireSuccess {
        /// The key of the acquired resource.
        key: ResourceKey,
        /// How long the acquire operation took.
        duration: Duration,
    },
    /// A resource acquisition failed.
    AcquireFailed {
        /// The key of the resource that failed to acquire.
        key: ResourceKey,
        /// Human-readable error description.
        error: String,
    },
    /// A resource handle was released (dropped).
    Released {
        /// The key of the released resource.
        key: ResourceKey,
        /// How long the handle was held.
        held: Duration,
        /// Whether the handle was tainted before release.
        tainted: bool,
    },
    /// A resource's health status changed.
    HealthChanged {
        /// The key of the affected resource.
        key: ResourceKey,
        /// Whether the resource is now healthy.
        healthy: bool,
    },
    /// A resource's configuration was hot-reloaded.
    ConfigReloaded {
        /// The key of the reloaded resource.
        key: ResourceKey,
    },
    /// A retry attempt is about to be made after a transient acquire failure.
    RetryAttempt {
        /// The key of the resource being retried.
        key: ResourceKey,
        /// 1-based attempt number (the initial attempt is not counted).
        attempt: u32,
        /// How long the retry will sleep before the next attempt.
        backoff: Duration,
        /// Human-readable description of the error that triggered the retry.
        error: String,
    },
    /// Pool backpressure was detected (semaphore full).
    BackpressureDetected {
        /// The key of the resource under pressure.
        key: ResourceKey,
    },
    /// A recovery gate changed state.
    RecoveryGateChanged {
        /// The key of the resource whose gate transitioned.
        key: ResourceKey,
        /// Human-readable description of the new gate state.
        state: String,
    },
    /// Aggregate outcome of one credential refresh cycle.
    ///
    /// Emitted by [`Manager::on_credential_refreshed`] after every
    /// per-resource dispatch future has completed (Tech Spec §6.2). The
    /// payload reports how many resources were affected and how their
    /// outcomes broke down across `ok` / `failed` / `timed_out`.
    ///
    /// Per-resource health-change signals on revocation failure are emitted
    /// inline via [`Self::HealthChanged`] (security amendment B-2). This
    /// aggregate captures only the cycle-level outcome distribution, so
    /// subscribers that miss it still see per-resource failure events.
    ///
    /// [`Manager::on_credential_refreshed`]: crate::manager::Manager::on_credential_refreshed
    CredentialRefreshed {
        /// The credential whose rotation triggered the cycle.
        credential_id: CredentialId,
        /// Total resources reached by the dispatch fan-out. Equal to
        /// `outcome.total()`.
        resources_affected: usize,
        /// Aggregate breakdown of per-resource outcomes.
        outcome: RotationOutcome,
    },
    /// Aggregate outcome of one credential revocation cycle.
    ///
    /// Emitted by [`Manager::on_credential_revoked`] after every
    /// per-resource dispatch future has completed (Tech Spec §6.2).
    /// Symmetric to [`Self::CredentialRefreshed`].
    ///
    /// [`Manager::on_credential_revoked`]: crate::manager::Manager::on_credential_revoked
    CredentialRevoked {
        /// The credential whose revocation triggered the cycle.
        credential_id: CredentialId,
        /// Total resources reached by the dispatch fan-out. Equal to
        /// `outcome.total()`.
        resources_affected: usize,
        /// Aggregate breakdown of per-resource outcomes.
        outcome: RotationOutcome,
    },
}

impl ResourceEvent {
    /// Returns the resource key associated with this event, if any.
    ///
    /// Aggregate events ([`Self::CredentialRefreshed`],
    /// [`Self::CredentialRevoked`]) span multiple resources and return
    /// `None`; use the `credential_id` field on those payloads for the
    /// rotation identifier.
    pub fn key(&self) -> Option<&ResourceKey> {
        match self {
            Self::Registered { key }
            | Self::Removed { key }
            | Self::AcquireSuccess { key, .. }
            | Self::AcquireFailed { key, .. }
            | Self::Released { key, .. }
            | Self::HealthChanged { key, .. }
            | Self::ConfigReloaded { key }
            | Self::RetryAttempt { key, .. }
            | Self::BackpressureDetected { key }
            | Self::RecoveryGateChanged { key, .. } => Some(key),
            Self::CredentialRefreshed { .. } | Self::CredentialRevoked { .. } => None,
        }
    }
}
