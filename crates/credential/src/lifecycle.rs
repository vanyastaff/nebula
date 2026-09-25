//! Credential lifecycle: what governs, and what is cut (ADR-0088 D2).
//!
//! **What governs today** is the five capability sub-traits
//! (`Interactive`/`Refreshable`/`Revocable`/`Testable`/`Dynamic`) plus the durable
//! `reauth_required` bit. Production slot resolution reads the registered capability
//! set and rejects on `MissingCapabilities` or `ReauthRequired` before any material is
//! decrypted (`crate::runtime::projection::slot`).
//!
//! **What is cut** is [`CredentialPolicy`] as the routing model. The type ships, and the
//! `#[credential]` macro derives [`CredentialLifecycle`] for every annotated credential,
//! but the only code that consults a policy is `CredentialResolver::resolve_with_refresh`
//! (`crate::runtime::resolver`) and its low-level `CredentialResolver::scheme_factory`
//! adapter. These technical APIs remain available, but production slot resolution
//! does not call them. The `Protocol` trait that would
//! compute a policy from state, and the migration off the sub-traits, remain unwritten.
//! 1.0 ships the capability traits as the governing model, and this cut is deliberate
//! rather than pending.
//!
//! **What an author must still write.** The macro synthesizes
//! [`RefreshStrategy::RefreshToken`] for a credential with `fn refresh`, and
//! [`RefreshStrategy::Static`] otherwise, so hand-write `fn policy` in three cases: a
//! leased (`Dynamic`) credential, where the macro rejects the omission outright; a
//! credential whose refresh strategy depends on live state, where the synthesized policy
//! emits a constant strategy; and one whose `AuthScheme::Family` declares a refresh class
//! other than the synthesized one, where the F3 containment guard in `resolve_with_refresh`
//! rejects the wrong policy at runtime.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// `RefreshStrategy` relocated to `nebula-core::auth` (a pure data enum, so it
/// can back [`crate::SchemeFamily::refresh_classes`] without an inverted
/// dependency). Re-exported here so existing `nebula_credential::RefreshStrategy`
/// paths keep resolving.
pub use nebula_core::auth::{RefreshStrategy, RefreshStrategyKind, SchemeId};
pub use nebula_storage_port::CredentialIncidentRef;

/// Secret-free durable availability of a persisted credential.
///
/// Claims remain internal authority, but their durable operation gate is part
/// of public availability. Claim ids, generations, holders, and fences never
/// enter this projection. Tombstones remain a persistence invariant:
/// management reads treat them as absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CredentialLifecycleState {
    /// No durable gate prevents ordinary credential use or refresh.
    Ready,
    /// Automatic refresh is deferred until a backend-authored instant.
    RefreshDeferred {
        /// Earliest instant at which automatic refresh may be attempted again.
        retry_at: DateTime<Utc>,
    },
    /// Automatic refresh is durably blocked until an explicit
    /// authority-changing transition supplies new material.
    RefreshBlocked,
    /// The credential cannot be used until interactive authorization succeeds.
    ReauthRequired,
    /// One typed provider operation is currently in flight.
    OperationInFlight {
        /// Public operation category; no claim authority is exposed.
        operation: CredentialLifecycleOperation,
    },
    /// An expired provider operation requires explicit reconciliation.
    ReconciliationRequired {
        /// Typed operation when the incident was created by an operation-aware
        /// runtime. `None` denotes a legacy unclassified incident, which cannot
        /// be adjudicated and must be replaced explicitly.
        operation: Option<CredentialLifecycleOperation>,
        /// The incident a reconciliation must name. `None` only while a legacy
        /// unclassified operation is still in flight, which has no expired
        /// incident yet.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        incident: Option<CredentialIncidentRef>,
    },
}

/// Public, secret-free provider operation category used by lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialLifecycleOperation {
    /// Renewable material refresh.
    Refresh,
    /// Provider-side credential revocation.
    Revoke,
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
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct LeaseRef {
    /// Opaque, server-assigned lease identifier.
    pub lease_id: String,
    /// Lease duration granted at the last issue or renewal.
    #[zeroize(skip)]
    pub lease_duration: Duration,
    /// Whether the lease can be renewed (some leases are one-shot).
    #[zeroize(skip)]
    pub renewable: bool,
    /// Hard renewal horizon: past this instant even a renewable lease must
    /// re-acquire, not renew (Kerberos TGT `renew_until`, a rotating
    /// refresh-token's absolute expiry). `None` = no horizon (renew indefinitely
    /// while `renewable`). [`CredentialPolicy::decide_refresh`] returns
    /// [`Decision::Reacquire`] once `now >= renew_until`.
    #[zeroize(skip)]
    pub renew_until: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for LeaseRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LeaseRef")
            .field("lease_id", &"[REDACTED]")
            .field("lease_duration", &self.lease_duration)
            .field("renewable", &self.renewable)
            .field("renew_until", &self.renew_until)
            .finish()
    }
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

/// The lifecycle policy a credential declares — **the authoring surface, not the gate**.
///
/// Expiry is three orthogonal cases (ADR-0088 D3): an inline [`Self::expires_at`]
/// (AWS STS / SPIFFE), an external renewable [`Self::lease`] (Vault), and
/// controller-managed declarative TTL (Kubernetes — surfaced as `expires_at`
/// from the projected token). Renewal is a per-credential capability, never
/// universal.
///
/// A `CredentialPolicy` is **computed** from state by
/// [`CredentialLifecycle::policy`]; it is not itself persisted (hence no `Serialize`).
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

        // Hard renewal horizon (lease `renew_until`, e.g. Kerberos TGT / a
        // rotating refresh-token's absolute expiry): once crossed, the material
        // can no longer be *renewed* — only re-acquired. This is independent of
        // whether the inline `expires_at` or the lease drives the expiry signal,
        // so it is computed once and gates every `Refresh` decision below (a
        // would-be refresh past the horizon becomes `Reacquire`). It does not by
        // itself force re-acquisition of still-valid material.
        let past_horizon = self
            .lease
            .as_ref()
            .and_then(|lease| lease.renew_until)
            .is_some_and(|horizon| now >= horizon);

        // 0. Externally-rotated material (`Watched`): the engine re-reads on
        //    change and never initiates renewal, so the resolver serves what it
        //    holds rather than scheduling a refresh.
        if matches!(self.refresh, RefreshStrategy::Watched) {
            return Decision::Usable;
        }

        // 1. Inline expiry: past it, or inside the proactive early-refresh window.
        if let Some(exp) = self.expires_at {
            if exp <= now {
                return if auto_renewable && !past_horizon {
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
            if exp - now <= early && auto_renewable && !past_horizon {
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
        if self.expires_at.is_none() && self.lease.is_some() {
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
/// The `#[nebula::credential]` macro (ADR-0088 D1) derives this impl, taking the
/// [`RefreshStrategy`] and [`RevokeStrategy`] from the capability methods the
/// author wrote, so the policy data cannot disagree with the compile-gated
/// capability impls. Hand-write `fn policy` in three cases the synthesized value
/// cannot cover: a leased (`Dynamic`) credential, where the macro rejects the
/// omission outright; a credential whose refresh strategy depends on live state,
/// where the synthesized policy emits a constant strategy; and a credential whose
/// `AuthScheme::Family` declares a refresh class other than the synthesized
/// [`RefreshStrategy::RefreshToken`], where the F3 containment guard in
/// `resolve_with_refresh` rejects the wrong policy at runtime.
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
#[path = "lifecycle_tests.rs"]
mod tests;
