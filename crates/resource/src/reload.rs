//! Reload outcome types for config hot-reload and credential rotation.

/// Result of per-topology reload dispatch.
///
/// Used by [`Manager`](crate::manager::Manager) for both config changes
/// and credential rotation. Each variant maps to one of the possible
/// outcomes when a reload is dispatched to a topology runtime.
///
/// The historical `Restarting` variant was reachable only from the
/// pre-collapse `Service` topology (a former engine-side daemon row);
/// post topology collapse (3 runtimes) it is structurally unreachable
/// and was removed. The two surviving outcomes — `NoChange` (fingerprint
/// identical) and `SwappedImmediately` (config/credential swapped in
/// place) — cover every live registry row. See the
/// [`manager`](crate::manager) module docs for the relabel rationale.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadOutcome {
    /// Applied immediately. Next acquire gets fresh config/credential.
    SwappedImmediately,
    /// Fingerprint identical — no change needed.
    NoChange,
}
