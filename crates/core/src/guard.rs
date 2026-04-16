//! Guard traits for RAII resource/credential wrappers.

use std::time::Instant;

/// Base RAII guard trait. Implemented by CredentialGuard and ResourceGuard.
pub trait Guard: Send + Sync + 'static {
    /// Kind identifier ("credential" or "resource").
    fn guard_kind(&self) -> &'static str;
    /// When this guard was acquired.
    fn acquired_at(&self) -> Instant;
    /// How long this guard has been held.
    fn age(&self) -> std::time::Duration {
        self.acquired_at().elapsed()
    }
}

/// Typed guard with access to inner value.
pub trait TypedGuard: Guard {
    /// The inner value type.
    type Inner: ?Sized;
    /// Access the inner value.
    fn as_inner(&self) -> &Self::Inner;
}
