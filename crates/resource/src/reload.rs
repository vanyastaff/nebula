//! Reload outcome types for config hot-reload and credential rotation.

/// Result of per-topology reload dispatch.
///
/// Used by [`Manager`](crate::manager::Manager) for both config changes
/// and credential rotation. Each variant maps to one of the four possible
/// outcomes when a reload is dispatched to a topology runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadOutcome {
    /// Applied immediately. Next acquire gets fresh config/credential.
    SwappedImmediately,
    /// Old runtime draining via `Arc` refcount (Service topology).
    PendingDrain {
        /// Generation counter of the runtime being drained.
        old_generation: u64,
    },
    /// Engine-side daemon (per ADR-0037, lives in `nebula_engine::daemon`)
    /// cancelled and restarting after a reload.
    Restarting,
    /// Fingerprint identical — no change needed.
    NoChange,
}
