//! Resource lifecycle events.
//!
//! [`ResourceEvent`] captures significant lifecycle transitions for
//! observability and diagnostics. Events are emitted by the [`Manager`]
//! during registration, acquisition, release, and health-check operations.
//!
//! [`Manager`]: crate::manager::Manager

use std::time::Duration;

use nebula_core::ResourceKey;

/// A lifecycle event emitted by the resource manager.
///
/// Events are lightweight and cheap to clone. They carry enough
/// information for logging, metrics, and audit trails without
/// holding references to live resources.
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
}

impl ResourceEvent {
    /// Returns the resource key associated with this event.
    pub fn key(&self) -> &ResourceKey {
        match self {
            Self::Registered { key }
            | Self::Removed { key }
            | Self::AcquireSuccess { key, .. }
            | Self::AcquireFailed { key, .. }
            | Self::Released { key, .. }
            | Self::HealthChanged { key, .. }
            | Self::ConfigReloaded { key } => key,
        }
    }
}
