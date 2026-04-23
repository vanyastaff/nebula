//! RAII guard traits for typed resource/credential lifetime management.
//!
//! Domain crates (`nebula-credential`, `nebula-resource`) provide concrete
//! guard types; this module defines the shared contract.

use std::{fmt, time::Instant};

/// Base RAII guard trait. Implemented by `CredentialGuard` and `ResourceGuard`.
pub trait Guard: Send + Sync + 'static {
    /// Stable kind identifier for metrics labels, logs, debug format.
    fn guard_kind(&self) -> &'static str;

    /// When this guard was acquired — for lifetime tracking, metrics, expiry checks.
    fn acquired_at(&self) -> Instant;

    /// How long this guard has been held.
    fn age(&self) -> std::time::Duration {
        self.acquired_at().elapsed()
    }
}

/// Typed guard — exposes inner type for generic helpers.
pub trait TypedGuard: Guard {
    /// The inner value type.
    type Inner: ?Sized;
    /// Access the inner value.
    fn as_inner(&self) -> &Self::Inner;
}

/// Helper: fully redacted `Debug` format.
///
/// Output: `Guard<credential>[REDACTED]`
pub fn debug_redacted<G: Guard>(g: &G, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "Guard<{}>[REDACTED]", g.guard_kind())
}

/// Helper: `Debug` format with type info but no content.
///
/// Output: `Guard<resource, inner=PgPool, age=1.2s>`
pub fn debug_typed<G: TypedGuard>(g: &G, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(
        f,
        "Guard<{}, inner={}, age={:?}>",
        g.guard_kind(),
        std::any::type_name::<G::Inner>(),
        g.age()
    )
}
