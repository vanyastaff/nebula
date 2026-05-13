//! In-memory registry of leases under lifecycle management.
//!
//! The scheduler task owns the only mutable view; the public
//! [`LeaseLifecycle`](super::LeaseLifecycle) handle communicates with
//! the worker through a command channel, so the registry does not need
//! its own lock. The token type is opaque to callers.

use std::sync::Arc;

use nebula_credential::{CredentialId, LeaseHandle, LeasedProvider};
use tokio::time::Instant;

/// Opaque registry key — handed back from
/// [`LeaseLifecycle::track`](super::LeaseLifecycle::track) and accepted
/// by [`LeaseLifecycle::revoke`](super::LeaseLifecycle::revoke).
///
/// Internally a monotonic `u64`; the wrapping type forbids external
/// fabrication so callers cannot pass a stale or guessed token in.
/// Ord/PartialOrd derived because the scheduler heap entry is keyed
/// `(Instant, LeaseToken)` and needs a total order for tie-breaking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LeaseToken(pub(super) u64);

/// One tracked lease in the registry.
///
/// Carries the lease metadata, the provider that can renew / revoke it,
/// optional attribution to a credential record, the next scheduled
/// renew instant, and the consecutive-failure counter for backoff.
#[derive(Clone)]
pub(super) struct LeaseEntry {
    /// Lease handle as received from the resolution envelope.
    pub(super) lease: LeaseHandle,
    /// Provider capable of renewing / revoking this lease.
    pub(super) provider: Arc<dyn LeasedProvider>,
    /// Optional credential record this lease was resolved for. Used by
    /// `revoke_for_credential` to scan-and-revoke on rotation.
    pub(super) credential_id: Option<CredentialId>,
    /// When the next renew attempt should fire.
    pub(super) next_renew_at: Instant,
    /// Consecutive renewal failure count — drives the policy's backoff
    /// schedule. Reset to 0 on a successful renew.
    pub(super) consecutive_failures: u32,
}
