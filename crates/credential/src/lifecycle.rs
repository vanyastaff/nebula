//! Credential lifecycle as DATA (ADR-0088 D2).
//!
//! Replaces the five capability sub-traits
//! (`Interactive`/`Refreshable`/`Revocable`/`Testable`/`Dynamic`) with a single
//! [`CredentialPolicy`] value that a credential's state declares. The
//! credential/secret field (AWS, Vault, GCP, Azure, k8s, SPIFFE) is unanimous:
//! capabilities are **data, not trait bounds** — a Vault dynamic secret is
//! leased *and* refreshable *and* revocable *and* expiring at once, so
//! orthogonal capability traits are the wrong model.
//!
//! The engine reads the policy a protocol computes from state and drives the
//! matching path; one OAuth2 protocol returns [`RefreshStrategy::RefreshToken`]
//! when a refresh token is present and [`RefreshStrategy::ReAcquire`] when it is
//! not — no separate trait, no compile-time capability lie.
//!
//! These are the foundational policy types. The `Protocol` trait that produces a
//! [`CredentialPolicy`] from state, and the migration off the sub-traits, land in
//! later steps of the ADR-0088 sequence; nothing consumes these yet.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The ~10 structural categories of credential, classified by **lifecycle
/// shape** rather than by wire scheme (there are ~35 wire schemes but only a
/// handful of distinct lifecycle shapes — ADR-0088 D1).
///
/// A single credential may legitimately inhabit more than one category (e.g. a
/// GCP service-account key can self-sign a bearer JWT *or* be exchanged), so a
/// protocol picks the category that drives its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CredentialCategory {
    /// Static secret valid until revoked — API key, HTTP Basic, PAT.
    StaticSecret,
    /// Request signed with a secret that never leaves the signer — AWS SigV4, HMAC.
    SignedRequest,
    /// Bearer token carrying its own expiry, no refresh pair — short-lived JWT.
    BearerWithExp,
    /// OAuth2 authorization-code / refresh-grant shape — an access token,
    /// usually paired with a refresh token.
    ///
    /// This is the structural **kind**, fixed for the credential type. A given
    /// instance may lack a refresh token (the provider issued none): it stays
    /// `RefreshPair`, but its [`RefreshStrategy`] is computed from live state and
    /// degrades to [`RefreshStrategy::ReAcquire`]. Category = the type; strategy =
    /// the state-derived behaviour.
    RefreshPair,
    /// Input credential exchanged for a scoped, shorter-lived output —
    /// AWS STS AssumeRole, GCP workload-identity-federation, RFC 8693 token exchange.
    FederatedExchange,
    /// Requires a human redirect / consent to (re)acquire — OAuth2 auth-code,
    /// device-code flow.
    InteractiveRedirect,
    /// X.509 or SSH key material — mTLS client cert, SSH key.
    KeyPair,
    /// Short-lived leased secret that must be renewed or it expires — Vault
    /// dynamic secret, Kubernetes projected service-account token.
    Leased,
    /// Server-side session — SAML / OIDC session, session cookie.
    Session,
    /// Connection string / DSN — database, message queue.
    ConnectionString,
}

/// How a credential's material is renewed as it nears or reaches expiry.
///
/// Data, not a trait. The engine reads this from the credential's policy and
/// drives the matching path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub enum RefreshStrategy {
    /// Valid until explicitly revoked; never auto-renewed (API key, PAT).
    #[default]
    Static,
    /// Renew without user interaction via the protocol's `refresh` — OAuth2
    /// refresh-token grant, Vault lease renew.
    RefreshToken,
    /// An external lease the engine's lease scheduler renews at a fraction of
    /// its TTL — Vault dynamic secret, Kubernetes projected token.
    Lease,
    /// Full re-acquisition round-trip; no incremental refresh — AWS STS
    /// AssumeRole, SAML/OIDC re-auth, OAuth2 without a refresh token.
    ReAcquire,
}

/// How a credential can be revoked. The field has **no uniform revoke
/// endpoint**: Vault revokes by lease handle (RFC 7009 for OAuth2), whereas AWS
/// STS revokes by an issue-time-keyed deny policy — the bytes stay syntactically
/// valid until expiry but access is denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub enum RevokeStrategy {
    /// No provider-side revocation; "revoke" means delete the local record only.
    #[default]
    None,
    /// Revoke via a provider-side handle or endpoint — Vault lease revoke,
    /// OAuth2 RFC 7009 token revocation.
    HandleBased,
    /// Revoke via an issue-time-keyed deny policy; individual material is not
    /// invalidated — AWS STS revoke-older-sessions.
    IssueTimePolicy,
}

/// An external lease reference, Vault-style: the server tracks expiry and
/// renewal and the client holds only the identifier (the lease is the unit of
/// expiry, not the secret value).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseRef {
    /// Opaque, server-assigned lease identifier.
    pub lease_id: String,
    /// Lease duration granted at the last issue or renewal.
    pub lease_duration: Duration,
    /// Whether the lease can be renewed (some leases are one-shot).
    pub renewable: bool,
}

/// The lifecycle policy a credential declares — **capabilities as data**.
///
/// Expiry is three orthogonal cases (ADR-0088 D3): an inline [`Self::expires_at`]
/// (AWS STS / SPIFFE), an external renewable [`Self::lease`] (Vault), and
/// controller-managed declarative TTL (Kubernetes — surfaced as `expires_at`
/// from the projected token). Renewal is a per-credential capability, never
/// universal.
///
/// A `CredentialPolicy` is **computed** from state by a protocol; it is not
/// itself persisted (hence no `Serialize`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialPolicy {
    /// The structural category this credential belongs to.
    pub category: CredentialCategory,
    /// Inline absolute expiry, if the material carries one.
    pub expires_at: Option<DateTime<Utc>>,
    /// External renewable lease, if the material is leased.
    pub lease: Option<LeaseRef>,
    /// How the material is refreshed.
    pub refresh: RefreshStrategy,
    /// How the material is revoked.
    pub revoke: RevokeStrategy,
}

impl CredentialPolicy {
    /// A static, non-expiring, provider-irrevocable policy — the API-key / PAT
    /// shape.
    #[must_use]
    pub const fn static_secret() -> Self {
        Self {
            category: CredentialCategory::StaticSecret,
            expires_at: None,
            lease: None,
            refresh: RefreshStrategy::Static,
            revoke: RevokeStrategy::None,
        }
    }

    /// Does this credential expire — either an inline expiry or a lease?
    #[must_use]
    pub const fn is_expiring(&self) -> bool {
        self.expires_at.is_some() || self.lease.is_some()
    }

    /// Has the inline expiry passed at `now`?
    ///
    /// Lease expiry is tracked server-side and is deliberately *not* decided
    /// here — a leased credential with no inline `expires_at` returns `false`.
    #[must_use]
    pub fn is_expired_at(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|exp| exp <= now)
    }

    /// Can the engine renew this without human interaction?
    ///
    /// A `Lease` strategy is auto-renewable only when its [`LeaseRef`] is
    /// actually renewable — a one-shot lease (`renewable: false`) must re-acquire,
    /// not renew.
    #[must_use]
    pub const fn is_auto_renewable(&self) -> bool {
        match self.refresh {
            RefreshStrategy::RefreshToken => true,
            RefreshStrategy::Lease => match &self.lease {
                Some(lease) => lease.renewable,
                None => false,
            },
            RefreshStrategy::Static | RefreshStrategy::ReAcquire => false,
        }
    }
}

/// A credential type's lifecycle policy, computed from its stored state.
///
/// The `#[nebula::credential]` macro (ADR-0088 D1) will derive this — the
/// [`RefreshStrategy`] / [`RevokeStrategy`] from which capability methods the
/// author wrote, so the policy data cannot disagree with the compile-gated
/// capability impls. Until the macro lands, credentials implement it by hand.
///
/// The policy is *computed*, never persisted, so it can reflect live state (e.g.
/// an OAuth2 credential reporting [`RefreshStrategy::RefreshToken`] only while it
/// actually holds a refresh token, and [`RefreshStrategy::ReAcquire`] otherwise).
pub trait CredentialLifecycle: crate::Credential {
    /// Compute the lifecycle policy for the given stored state.
    fn policy(state: &Self::State) -> CredentialPolicy
    where
        Self: Sized;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_secret_is_inert() {
        let p = CredentialPolicy::static_secret();
        assert!(!p.is_expiring());
        assert!(!p.is_auto_renewable());
        assert_eq!(p.category, CredentialCategory::StaticSecret);
        assert_eq!(p.refresh, RefreshStrategy::Static);
        assert_eq!(p.revoke, RevokeStrategy::None);
    }

    #[test]
    fn refresh_pair_is_auto_renewable_and_expiring() {
        let now = Utc::now();
        let p = CredentialPolicy {
            category: CredentialCategory::RefreshPair,
            expires_at: Some(now + chrono::Duration::minutes(60)),
            lease: None,
            refresh: RefreshStrategy::RefreshToken,
            revoke: RevokeStrategy::HandleBased,
        };
        assert!(p.is_expiring());
        assert!(p.is_auto_renewable());
        assert!(!p.is_expired_at(now));
        assert!(p.is_expired_at(now + chrono::Duration::minutes(61)));
    }

    #[test]
    fn lease_is_expiring_but_inline_expiry_decides_is_expired() {
        let now = Utc::now();
        let p = CredentialPolicy {
            category: CredentialCategory::Leased,
            expires_at: None,
            lease: Some(LeaseRef {
                lease_id: "vault/lease/abc".to_owned(),
                lease_duration: Duration::from_hours(1),
                renewable: true,
            }),
            refresh: RefreshStrategy::Lease,
            revoke: RevokeStrategy::HandleBased,
        };
        assert!(p.is_expiring());
        assert!(p.is_auto_renewable());
        // No inline expiry → never reported expired locally (server-tracked).
        assert!(!p.is_expired_at(now + chrono::Duration::days(365)));
    }

    #[test]
    fn one_shot_lease_is_not_auto_renewable() {
        let p = CredentialPolicy {
            category: CredentialCategory::Leased,
            expires_at: None,
            lease: Some(LeaseRef {
                lease_id: "vault/lease/one-shot".to_owned(),
                lease_duration: Duration::from_hours(1),
                renewable: false,
            }),
            refresh: RefreshStrategy::Lease,
            revoke: RevokeStrategy::HandleBased,
        };
        // Leased + non-renewable ⇒ must re-acquire, not auto-renew.
        assert!(!p.is_auto_renewable());
        assert!(p.is_expiring());
    }

    #[test]
    fn reacquire_is_not_auto_renewable() {
        let p = CredentialPolicy {
            category: CredentialCategory::FederatedExchange,
            expires_at: None,
            lease: None,
            refresh: RefreshStrategy::ReAcquire,
            revoke: RevokeStrategy::IssueTimePolicy,
        };
        assert!(!p.is_auto_renewable());
    }

    #[test]
    fn category_and_strategies_round_trip_json() {
        for cat in [
            CredentialCategory::StaticSecret,
            CredentialCategory::Leased,
            CredentialCategory::InteractiveRedirect,
        ] {
            let json = serde_json::to_string(&cat).expect("serialize");
            let back: CredentialCategory = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(cat, back);
        }
        let r = RefreshStrategy::RefreshToken;
        assert_eq!(
            r,
            serde_json::from_str(&serde_json::to_string(&r).expect("ser")).expect("de")
        );
    }
}
