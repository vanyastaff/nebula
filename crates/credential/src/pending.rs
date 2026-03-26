//! Typed pending state for interactive credential flows (v2).
//!
//! [`PendingState`] represents ephemeral data held between the initial
//! `resolve()` call and the subsequent `continue_resolve()` callback.
//! The framework stores it encrypted with TTL and single-use semantics
//! via `PendingStateStore` -- credential authors never manage storage
//! directly.
//!
//! Non-interactive credentials use [`NoPendingState`] as a zero-cost
//! marker type.

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// Typed pending state for interactive credential flows.
///
/// Stored via `PendingStateStore` with encryption, TTL, single-use.
///
/// # Security properties
///
/// - **TTL enforced:** [`expires_in`](PendingState::expires_in)
///   determines max lifetime (typically 5-15 min).
/// - **Single-use:** consumed (deleted) on first read by
///   `continue_resolve()`.
/// - **Encrypted at rest** by the `PendingStateStore` implementation.
/// - **Zeroize on drop:** secrets zeroed when state is dropped.
/// - Serialization buffers wrapped in `Zeroizing<Vec<u8>>` by store
///   implementation.
pub trait PendingState: Serialize + DeserializeOwned + Send + Sync + Zeroize + 'static {
    /// Stable identifier for this pending-state type
    /// (e.g. `"oauth2_pending"`).
    const KIND: &'static str;

    /// How long this pending state should remain valid.
    fn expires_in(&self) -> Duration;
}

/// Marker type for non-interactive credentials.
///
/// Credential types that resolve in a single step (API key, basic auth,
/// database) use `NoPendingState` as their `Credential::Pending`
/// associated type. It serializes to `null` and expires immediately.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NoPendingState;

impl Zeroize for NoPendingState {
    fn zeroize(&mut self) {
        // No sensitive data to zeroize.
    }
}

impl PendingState for NoPendingState {
    const KIND: &'static str = "none";

    fn expires_in(&self) -> Duration {
        Duration::ZERO
    }
}
