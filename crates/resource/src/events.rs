//! Resource lifecycle events.
//!
//! [`ResourceEvent`] captures significant lifecycle transitions for
//! observability and diagnostics. Events are emitted by the [`Manager`]
//! during registration, acquisition, release, and health-check operations.
//!
//! [`Manager`]: crate::manager::Manager

use std::time::Duration;

use nebula_core::{ExecutionId, ResourceKey, WorkflowId, obs::SpanId};

use crate::error::ErrorKind;

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
        /// Typed classification of the failure, so subscribers can route on
        /// the kind (retryable / backpressure / revoked / …) without parsing
        /// the `error` string.
        kind: ErrorKind,
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
    /// A `#[credential]` slot was refreshed on this resource (engine fan-out).
    SlotRefreshed {
        /// The key of the resource whose slot was refreshed.
        key: ResourceKey,
        /// The slot name that was refreshed.
        slot: String,
    },
    /// A `#[credential]` slot's credential was revoked.
    SlotRevoked {
        /// The key of the resource whose slot was revoked.
        key: ResourceKey,
        /// The slot name that was revoked.
        slot: String,
    },
    /// The per-resource refresh hook failed or timed out. `error` is an
    /// already-redacted string (NEVER credential material).
    SlotRefreshFailed {
        /// The key of the resource whose slot refresh failed.
        key: ResourceKey,
        /// The slot name whose refresh failed.
        slot: String,
        /// Already-redacted error description.
        error: String,
    },
    /// The per-resource revoke hook failed. `error` is an already-redacted
    /// string (NEVER credential material).
    SlotRevokeFailed {
        /// The key of the resource whose slot revoke failed.
        key: ResourceKey,
        /// The slot name whose revoke failed.
        slot: String,
        /// Already-redacted error description.
        error: String,
    },
    /// A background pool-maintenance sweep evicted idle-timed-out,
    /// max-lifetime-exceeded, stale-fingerprint, or revoked idle instances.
    /// Emitted only when at least one instance was evicted.
    MaintenanceEvicted {
        /// The key of the pool whose idle instances were evicted.
        key: ResourceKey,
        /// Number of instances evicted in this maintenance cycle.
        evicted: usize,
    },
    /// A lease was still held past its [`Provider::max_hold_duration`]
    /// deadline — leak/hang detection (HikariCP `leakDetectionThreshold`
    /// equivalent). Emitted by the hold-deadline watchdog while the guard is
    /// still alive; the lease is NOT forcibly released (warn-only).
    ///
    /// [`Provider::max_hold_duration`]: crate::resource::Provider::max_hold_duration
    HoldDeadlineExceeded {
        /// The key of the resource whose lease overran its hold deadline.
        key: ResourceKey,
        /// How long the lease has been held when the watchdog fired.
        held: Duration,
        /// The configured hold deadline that was exceeded.
        deadline: Duration,
        /// The execution id of the [`ResourceContext`](crate::context::ResourceContext)
        /// that acquired this lease, if the scope carried one — names which
        /// execution to go blame for the leak.
        execution_id: Option<ExecutionId>,
        /// The workflow id of the acquiring context's scope, if present.
        workflow_id: Option<WorkflowId>,
        /// The tracing span id active at acquire time, if the acquiring
        /// context carried one — lets a trace backend jump straight from
        /// this event to the acquiring span.
        span_id: Option<SpanId>,
    },
}

impl ResourceEvent {
    /// Returns the resource key associated with this event.
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
            | Self::RecoveryGateChanged { key, .. }
            | Self::SlotRefreshed { key, .. }
            | Self::SlotRevoked { key, .. }
            | Self::SlotRefreshFailed { key, .. }
            | Self::SlotRevokeFailed { key, .. }
            | Self::MaintenanceEvicted { key, .. }
            | Self::HoldDeadlineExceeded { key, .. } => Some(key),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_events_carry_no_credential_data() {
        let k = ResourceKey::new("k").expect("valid key");

        let refreshed = ResourceEvent::SlotRefreshed {
            key: k.clone(),
            slot: "db".into(),
        };
        assert_eq!(refreshed.key().map(ResourceKey::as_str), Some("k"));

        let revoked = ResourceEvent::SlotRevoked {
            key: k.clone(),
            slot: "db".into(),
        };
        assert_eq!(revoked.key().map(ResourceKey::as_str), Some("k"));

        let failed = ResourceEvent::SlotRefreshFailed {
            key: k.clone(),
            slot: "db".into(),
            error: "transient: upstream 503".into(),
        };
        assert_eq!(failed.key().map(ResourceKey::as_str), Some("k"));
        let ResourceEvent::SlotRefreshFailed { error, .. } = &failed else {
            unreachable!()
        };
        assert!(!error.contains("secret"), "error must be redacted");

        let revoke_failed = ResourceEvent::SlotRevokeFailed {
            key: k,
            slot: "db".into(),
            error: "transient: upstream 503".into(),
        };
        assert_eq!(revoke_failed.key().map(ResourceKey::as_str), Some("k"));
        let ResourceEvent::SlotRevokeFailed { error, .. } = &revoke_failed else {
            unreachable!()
        };
        assert!(!error.contains("secret"), "error must be redacted");
    }
}
