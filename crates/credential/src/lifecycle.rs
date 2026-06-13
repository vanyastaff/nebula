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

/// `RefreshStrategy` relocated to `nebula-core::auth` (a pure data enum, so it
/// can back [`crate::SchemeFamily::refresh_classes`] without an inverted
/// dependency). Re-exported here so existing `nebula_credential::RefreshStrategy`
/// paths keep resolving.
pub use nebula_core::auth::{RefreshStrategy, RefreshStrategyKind, SchemeId};

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
    /// Hard renewal horizon: past this instant even a renewable lease must
    /// re-acquire, not renew (Kerberos TGT `renew_until`, a rotating
    /// refresh-token's absolute expiry). `None` = no horizon (renew indefinitely
    /// while `renewable`). [`CredentialPolicy::decide_refresh`] returns
    /// [`Decision::Reacquire`] once `now >= renew_until`.
    pub renew_until: Option<DateTime<Utc>>,
}

/// The single routing decision the resolver acts on for a credential at a
/// point in time — computed by [`CredentialPolicy::decide_refresh`].
///
/// **Total, pure, and time-free.** It carries no deadline: a deadline field
/// would have the identical type for a real value and `MAX`, so it could not
/// make the never-revalidated class unrepresentable.
/// Liveness is decided here, once, against an injected clock — the credential
/// author never returns a "valid forever" verdict.
///
/// Owner ruling (2026-06-12): **there is no exempt static category.** A
/// credential with no honest freshness signal (a plain API key — no expiry, no
/// refresh) still has a framework-imposed mandatory re-validation floor, so past
/// the floor it returns [`Decision::Revalidate`] rather than [`Decision::Usable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Decision {
    /// Material is fresh and within its re-validation floor — serve as-is.
    Usable,
    /// Renew without user interaction — a `RefreshToken` grant or a renewable
    /// `Lease`.
    Refresh,
    /// Re-acquire from scratch — `ReAcquire`, an expired non-renewable lease, or
    /// an interactive redirect; there is no incremental renew path.
    Reacquire,
    /// Re-validate a static credential past its mandatory floor. There is no
    /// refresh/re-acquire material — only a liveness probe (`Testable`). Owner
    /// ruling: no static credential is exempt from periodic re-validation.
    Revalidate,
    /// Terminally unusable — a revoked tombstone or a provider `invalid_grant`.
    /// Never served. Set by the caller from the durable revoke/reauth signal,
    /// not by [`CredentialPolicy::decide_refresh`] (which cannot see it).
    Dead,
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
        // Matched by reference: `RefreshStrategy` is no longer `Copy` (its
        // `ReAcquire` arm carries a `SchemeId`).
        match &self.refresh {
            RefreshStrategy::RefreshToken => true,
            RefreshStrategy::Lease => match &self.lease {
                Some(lease) => lease.renewable,
                None => false,
            },
            // A pure local key→token mint needs no human and no network — the
            // engine can always do it.
            RefreshStrategy::ReMintLocal => true,
            // Re-acquire is not a renewal (interactivity is handled at the
            // re-acquire seam, not here); `Watched` is rotated by an external
            // source the engine never drives.
            RefreshStrategy::Static
            | RefreshStrategy::ReAcquire { .. }
            | RefreshStrategy::Watched => false,
            // `RefreshStrategy` is `#[non_exhaustive]` (it lives in `nebula-core`).
            // A strategy this version does not recognise is treated as NOT
            // auto-renewable — fail safe (never claim a credential renews itself
            // when the engine cannot prove it). A new renewable strategy must add
            // an explicit arm above, not rely on this default.
            _ => false,
        }
    }

    /// The single, total, **pure** routing decision for this credential at
    /// `now` — what the resolver must do before serving the material.
    ///
    /// Inputs only: no clock read and no `rand`. `now` is injected; jitter is
    /// applied **once at the scheduler seam**, never here, so the function stays
    /// pure and cacheable (DESIGN §24 scale invariant). `early_refresh` is the
    /// proactive buffer before inline expiry; `floor` is the framework-imposed
    /// mandatory re-validation interval that applies even to credentials with no
    /// honest freshness signal (owner ruling 2026-06-12 — there is no exempt
    /// "valid forever" static category).
    ///
    /// [`Decision::Dead`] is never returned here: a revoked tombstone / terminal
    /// `invalid_grant` is a durable signal the caller holds, not something this
    /// pure function can observe.
    ///
    /// Complexity: O(1) (a fixed set of comparisons).
    #[must_use]
    pub fn decide_refresh(
        &self,
        last_validated: DateTime<Utc>,
        now: DateTime<Utc>,
        early_refresh: Duration,
        floor: Duration,
    ) -> Decision {
        let auto_renewable = self.is_auto_renewable();

        // 0. Externally-rotated material (`Watched`): the engine re-reads on
        //    change and never initiates renewal, so the resolver serves what it
        //    holds rather than scheduling a refresh.
        if matches!(self.refresh, RefreshStrategy::Watched) {
            return Decision::Usable;
        }

        // 1. Inline expiry: past it, or inside the proactive early-refresh window.
        if let Some(exp) = self.expires_at {
            if exp <= now {
                return if auto_renewable {
                    Decision::Refresh
                } else {
                    Decision::Reacquire
                };
            }
            let early = chrono::Duration::from_std(early_refresh).unwrap_or_else(|_| {
                // early_refresh is a small config buffer; an out-of-range value
                // means "no proactive window", not a panic.
                chrono::Duration::zero()
            });
            if exp - now <= early && auto_renewable {
                return Decision::Refresh;
            }
            // Within the window but nothing to renew (static/re-acquire): let it
            // ride until expiry rather than churn a re-acquire early.
            if exp - now <= early {
                return Decision::Usable;
            }
        }

        // 2. Server-tracked lease with no inline expiry: a renewable lease is
        //    renewed by the lease scheduler; a one-shot lease — or one past its
        //    hard renewal horizon (`renew_until`) — must re-acquire instead.
        if self.expires_at.is_none()
            && let Some(lease) = &self.lease
        {
            let past_horizon = lease.renew_until.is_some_and(|horizon| now >= horizon);
            return if auto_renewable && !past_horizon {
                Decision::Refresh
            } else {
                Decision::Reacquire
            };
        }

        // 3. Mandatory re-validation floor — applies to every credential,
        //    including a static secret whose state carries no expiry/lease
        //    (owner ruling: no "valid forever").
        let floor = chrono::Duration::from_std(floor).unwrap_or_else(|_| chrono::Duration::zero());
        if floor > chrono::Duration::zero() && now - last_validated >= floor {
            return if auto_renewable {
                Decision::Refresh
            } else {
                Decision::Revalidate
            };
        }

        Decision::Usable
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
        assert_eq!(p.refresh, RefreshStrategy::Static);
        assert_eq!(p.revoke, RevokeStrategy::None);
    }

    #[test]
    fn refresh_pair_is_auto_renewable_and_expiring() {
        let now = Utc::now();
        let p = CredentialPolicy {
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
            expires_at: None,
            lease: Some(LeaseRef {
                lease_id: "vault/lease/abc".to_owned(),
                lease_duration: Duration::from_hours(1),
                renewable: true,
                renew_until: None,
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
            expires_at: None,
            lease: Some(LeaseRef {
                lease_id: "vault/lease/one-shot".to_owned(),
                lease_duration: Duration::from_hours(1),
                renewable: false,
                renew_until: None,
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
            expires_at: None,
            lease: None,
            refresh: RefreshStrategy::ReAcquire {
                from: None,
                interactive: false,
            },
            revoke: RevokeStrategy::IssueTimePolicy,
        };
        assert!(!p.is_auto_renewable());
    }

    #[test]
    fn strategies_round_trip_json() {
        for r in [
            RefreshStrategy::Static,
            RefreshStrategy::RefreshToken,
            RefreshStrategy::Lease,
            RefreshStrategy::ReAcquire {
                from: Some(SchemeId::new("oauth2")),
                interactive: true,
            },
            RefreshStrategy::ReMintLocal,
            RefreshStrategy::Watched,
        ] {
            let back: RefreshStrategy =
                serde_json::from_str(&serde_json::to_string(&r).expect("ser")).expect("de");
            assert_eq!(r, back);
        }
    }

    const HOUR: Duration = Duration::from_hours(1);
    const FIVE_MIN: Duration = Duration::from_mins(5);

    #[test]
    fn static_within_floor_is_usable() {
        let now = Utc::now();
        let p = CredentialPolicy::static_secret();
        // Validated 30 min ago, floor 1h → still fresh.
        let last = now - chrono::Duration::minutes(30);
        assert_eq!(
            p.decide_refresh(last, now, FIVE_MIN, HOUR),
            Decision::Usable
        );
    }

    #[test]
    fn static_past_floor_revalidates_not_usable() {
        // Owner ruling 2026-06-12: no "valid forever". A static API key past its
        // mandatory floor must be re-validated, never silently served forever.
        let now = Utc::now();
        let p = CredentialPolicy::static_secret();
        let last = now - chrono::Duration::hours(2); // floor 1h elapsed
        assert_eq!(
            p.decide_refresh(last, now, FIVE_MIN, HOUR),
            Decision::Revalidate
        );
    }

    #[test]
    fn refresh_pair_past_expiry_refreshes() {
        let now = Utc::now();
        let p = CredentialPolicy {
            expires_at: Some(now - chrono::Duration::minutes(1)),
            lease: None,
            refresh: RefreshStrategy::RefreshToken,
            revoke: RevokeStrategy::HandleBased,
        };
        assert_eq!(
            p.decide_refresh(now, now, FIVE_MIN, HOUR),
            Decision::Refresh
        );
    }

    #[test]
    fn refresh_pair_in_early_window_refreshes() {
        let now = Utc::now();
        let p = CredentialPolicy {
            expires_at: Some(now + chrono::Duration::minutes(2)), // inside 5-min buffer
            lease: None,
            refresh: RefreshStrategy::RefreshToken,
            revoke: RevokeStrategy::HandleBased,
        };
        assert_eq!(
            p.decide_refresh(now, now, FIVE_MIN, HOUR),
            Decision::Refresh
        );
    }

    #[test]
    fn leased_renewable_no_inline_expiry_refreshes() {
        // A leased secret (expires_at: None) must NOT be
        // treated as fresh forever — it enters the renew path.
        let now = Utc::now();
        let p = CredentialPolicy {
            expires_at: None,
            lease: Some(LeaseRef {
                lease_id: "vault/lease/abc".to_owned(),
                lease_duration: HOUR,
                renewable: true,
                renew_until: None,
            }),
            refresh: RefreshStrategy::Lease,
            revoke: RevokeStrategy::HandleBased,
        };
        assert_eq!(
            p.decide_refresh(now, now, FIVE_MIN, HOUR),
            Decision::Refresh
        );
    }

    #[test]
    fn leased_one_shot_reacquires() {
        let now = Utc::now();
        let p = CredentialPolicy {
            expires_at: None,
            lease: Some(LeaseRef {
                lease_id: "vault/lease/one-shot".to_owned(),
                lease_duration: HOUR,
                renewable: false,
                renew_until: None,
            }),
            refresh: RefreshStrategy::Lease,
            revoke: RevokeStrategy::HandleBased,
        };
        assert_eq!(
            p.decide_refresh(now, now, FIVE_MIN, HOUR),
            Decision::Reacquire
        );
    }

    #[test]
    fn reacquire_past_expiry_reacquires() {
        let now = Utc::now();
        let p = CredentialPolicy {
            expires_at: Some(now - chrono::Duration::seconds(1)),
            lease: None,
            refresh: RefreshStrategy::ReAcquire {
                from: None,
                interactive: false,
            },
            revoke: RevokeStrategy::IssueTimePolicy,
        };
        assert_eq!(
            p.decide_refresh(now, now, FIVE_MIN, HOUR),
            Decision::Reacquire
        );
    }
}
