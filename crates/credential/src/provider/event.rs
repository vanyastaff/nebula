//! Lease lifecycle events for cross-crate signaling.
//!
//! Emitted via `EventBus<LeaseEvent>` by the engine's lease lifecycle
//! subsystem (`nebula_engine::credential::lease`). Subscribers (audit
//! sinks, metric scrapers, ops dashboards) consume these to track the
//! health of dynamic-secret grants without reaching into provider
//! internals.
//!
//! Like [`CredentialEvent`](crate::CredentialEvent), payloads carry
//! identifiers only — **never the secret value**.
//!
//! # Why a separate event type
//!
//! Lease events carry richer payload than the simple
//! `{credential_id}` shape of [`CredentialEvent`], and a significant
//! subset of leases are unattributed to a nebula `CredentialId` (e.g.
//! ad-hoc provider use through composition root). Publishing on
//! `EventBus<LeaseEvent>` rather than overloading `CredentialEvent`
//! keeps subscriber filtering crisp and avoids forcing every variant to
//! accept an `Option<CredentialId>`.

use std::borrow::Cow;
use std::time::Duration;

use crate::CredentialId;

/// Reason a lease left the lifecycle registry.
///
/// Carried by [`LeaseEvent::LeaseExpired`] to distinguish failure modes
/// for observability — a Vault `NotFound` (lease already gone upstream)
/// is operationally different from a renewal-budget exhaustion.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LeaseExpiryReason {
    /// Lease was renewed `max_retries` times without success against an
    /// `Unavailable` / `Backend` backend; the lifecycle dropped it.
    RenewalFailed,
    /// Provider responded with `NotFound` / `AccessDenied` — the upstream
    /// lease no longer exists, retries are pointless.
    NotFoundUpstream,
    /// The lifecycle was shut down via its `CancellationToken` while the
    /// lease was still active.
    Shutdown,
}

/// Lease lifecycle event.
///
/// Emitted after every state transition the lifecycle observes (renew
/// success, renew failure, revoke success, revoke failure, expiry).
/// `#[non_exhaustive]` so additive variants do not break subscribers.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum LeaseEvent {
    /// Lease was renewed against its issuing provider; the registry
    /// reset its next-renew schedule to the new TTL.
    LeaseRenewed {
        /// Optional credential id this lease is attributed to. `None`
        /// for orphan leases tracked without a nebula credential record.
        credential_id: Option<CredentialId>,
        /// Provider-specific lease identifier.
        lease_id: String,
        /// Issuing provider's `provider_name()`.
        provider: Cow<'static, str>,
        /// New TTL communicated by the provider on renew.
        new_ttl: Duration,
    },

    /// Lease was explicitly revoked through `LeasedProvider::revoke` and
    /// removed from the registry.
    LeaseRevoked {
        /// Optional credential id this lease was attributed to.
        credential_id: Option<CredentialId>,
        /// Provider-specific lease identifier.
        lease_id: String,
        /// Issuing provider's `provider_name()`.
        provider: Cow<'static, str>,
    },

    /// A renewal attempt failed. The lifecycle will retry per its backoff
    /// schedule; emit one of these per failed attempt for observability.
    LeaseRenewalFailed {
        /// Optional credential id this lease is attributed to.
        credential_id: Option<CredentialId>,
        /// Provider-specific lease identifier.
        lease_id: String,
        /// Issuing provider's `provider_name()`.
        provider: Cow<'static, str>,
        /// Short reason string from `ProviderError::Display`.
        reason: String,
    },

    /// A revoke attempt failed. Rotation pipelines DO NOT block on this
    /// — local state has already committed. Treat as audit signal.
    LeaseRevocationFailed {
        /// Optional credential id this lease was attributed to.
        credential_id: Option<CredentialId>,
        /// Provider-specific lease identifier.
        lease_id: String,
        /// Issuing provider's `provider_name()`.
        provider: Cow<'static, str>,
        /// Short reason string from `ProviderError::Display`.
        reason: String,
    },

    /// Lease left the registry without successful revoke — either
    /// renewal budget exhausted, upstream removed it, or lifecycle
    /// shutdown.
    LeaseExpired {
        /// Optional credential id this lease was attributed to.
        credential_id: Option<CredentialId>,
        /// Provider-specific lease identifier.
        lease_id: String,
        /// Issuing provider's `provider_name()`.
        provider: Cow<'static, str>,
        /// Why the lease left the registry.
        reason: LeaseExpiryReason,
    },
}

impl LeaseEvent {
    /// Provider-specific lease identifier for any variant.
    #[must_use]
    pub fn lease_id(&self) -> &str {
        match self {
            Self::LeaseRenewed { lease_id, .. }
            | Self::LeaseRevoked { lease_id, .. }
            | Self::LeaseRenewalFailed { lease_id, .. }
            | Self::LeaseRevocationFailed { lease_id, .. }
            | Self::LeaseExpired { lease_id, .. } => lease_id,
        }
    }

    /// Issuing provider name for any variant.
    #[must_use]
    pub fn provider(&self) -> &str {
        match self {
            Self::LeaseRenewed { provider, .. }
            | Self::LeaseRevoked { provider, .. }
            | Self::LeaseRenewalFailed { provider, .. }
            | Self::LeaseRevocationFailed { provider, .. }
            | Self::LeaseExpired { provider, .. } => provider.as_ref(),
        }
    }

    /// Attributed credential id for any variant, if known.
    #[must_use]
    pub fn credential_id(&self) -> Option<CredentialId> {
        match self {
            Self::LeaseRenewed { credential_id, .. }
            | Self::LeaseRevoked { credential_id, .. }
            | Self::LeaseRenewalFailed { credential_id, .. }
            | Self::LeaseRevocationFailed { credential_id, .. }
            | Self::LeaseExpired { credential_id, .. } => *credential_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accessors_return_payload() {
        let id = CredentialId::new();
        let ev = LeaseEvent::LeaseRenewed {
            credential_id: Some(id),
            lease_id: "lease-abc".to_owned(),
            provider: Cow::Borrowed("vault"),
            new_ttl: Duration::from_hours(1),
        };
        assert_eq!(ev.lease_id(), "lease-abc");
        assert_eq!(ev.provider(), "vault");
        assert_eq!(ev.credential_id(), Some(id));
    }

    #[test]
    fn orphan_lease_event_has_no_credential_id() {
        let ev = LeaseEvent::LeaseExpired {
            credential_id: None,
            lease_id: "orphan".to_owned(),
            provider: Cow::Borrowed("vault"),
            reason: LeaseExpiryReason::Shutdown,
        };
        assert_eq!(ev.credential_id(), None);
        assert_eq!(ev.lease_id(), "orphan");
    }

    #[test]
    fn expiry_reason_distinguishes_failure_modes() {
        // Three distinct variants — observability subscribers route on
        // these, so the API contract is that they remain distinguishable.
        let r1 = LeaseExpiryReason::RenewalFailed;
        let r2 = LeaseExpiryReason::NotFoundUpstream;
        let r3 = LeaseExpiryReason::Shutdown;
        assert_ne!(r1, r2);
        assert_ne!(r2, r3);
        assert_ne!(r1, r3);
    }
}
