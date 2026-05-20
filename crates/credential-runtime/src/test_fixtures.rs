//! Test-only credential fixtures.
//!
//! The three first-party builtins (`bearer_token`, `shared_key`,
//! `signing_key`) are all static — none implements `Refreshable` /
//! `Testable` / `Revocable` / `Interactive`, so every *positive*
//! capability path of the facade (refresh CAS + retry, the coalesced
//! re-read branch, the concurrent-refresh version-conflict branch, `on_refresh`
//! emission) is otherwise unexercised. [`RefreshableFixtureCredential`]
//! is a minimal refreshable type that drives those paths.
//!
//! # test-util gating
//!
//! This module is gated `cfg(any(test, feature = "test-util"))` and is
//! **never** part of the production surface. It wires only over the
//! already-dev-only in-memory / `StaticKeyProvider` backends (the same
//! ones [`crate::service::test_support`] uses) and `unwrap`/`expect` is
//! acceptable here — this is test-support code, not a release path.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use chrono::{DateTime, Utc};
use nebula_credential::contract::plugin_capability_report;
use nebula_credential::contract::resolve::{RefreshOutcome, ResolveResult};
use nebula_credential::error::{RefreshErrorKind, RefreshFailedContext, RetryAdvice};
use nebula_credential::scheme::SecretToken;
use nebula_credential::{
    AuthPattern, Credential, CredentialContext, CredentialError, CredentialMetadata,
    CredentialState, ProviderErrorContext, ProviderErrorKind, Refreshable, SecretFreeMessage,
    SecretString,
};
use nebula_schema::{FieldValues, Schema};
use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;

/// Setup-form shape for the refreshable fixture credential.
#[derive(Schema, Deserialize, Default)]
pub struct RefreshableFixtureProperties {
    /// The initial token value.
    #[field(secret, label = "Token")]
    #[validate(required)]
    pub token: String,
}

/// Stored state for [`RefreshableFixtureCredential`]: the current token,
/// a monotonically-increasing refresh counter (so a test can observe
/// that `refresh` actually mutated and re-persisted state), and an
/// `expires_at` so the type carries an expiry like a real rotating
/// credential.
///
/// `ZeroizeOnDrop` is mandatory for any [`CredentialState`] (credential secrecy
/// deterministic plaintext drop). The counter / timestamp are not
/// secret, but the derive zeroizes the whole struct uniformly.
#[derive(Clone, Serialize, Deserialize, ZeroizeOnDrop)]
pub struct RefreshableFixtureState {
    /// Current token value (rotated on each `refresh`).
    token: String,
    /// How many times `refresh` has run for this state.
    #[zeroize(skip)]
    refresh_count: u32,
    /// Synthetic expiry (not enforced by the fixture; present so the
    /// type behaves like an expiring credential).
    #[zeroize(skip)]
    expires_at: Option<DateTime<Utc>>,
}

impl RefreshableFixtureState {
    /// The current token (test assertion seam).
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    /// The refresh counter (test assertion seam).
    #[must_use]
    pub fn refresh_count(&self) -> u32 {
        self.refresh_count
    }
}

impl CredentialState for RefreshableFixtureState {
    const KIND: &'static str = "refreshable_fixture_state";
    const VERSION: u32 = 1;

    fn expires_at(&self) -> Option<DateTime<Utc>> {
        self.expires_at
    }
}

/// Optional rendezvous a test installs to make the refresh CAS race
/// deterministic. When set, [`RefreshableFixtureCredential::refresh`]
/// `.wait()`s on it exactly once *after* mutating its local state but
/// *before* returning (i.e. before the service's compare-and-swap
/// re-persist). The concurrent task can then land a version-bumping
/// write while refresh is parked, guaranteeing the CAS observes a stale
/// version. Default `None` → refresh never blocks.
static REFRESH_RENDEZVOUS: std::sync::Mutex<Option<Arc<tokio::sync::Barrier>>> =
    std::sync::Mutex::new(None);

/// Install (or clear with `None`) the refresh rendezvous barrier. The
/// barrier should have 2 parties: the fixture's `refresh` and the test's
/// concurrent writer.
pub fn set_refresh_rendezvous(barrier: Option<Arc<tokio::sync::Barrier>>) {
    *REFRESH_RENDEZVOUS.lock().unwrap() = barrier;
}

/// Scripted failure for the next refresh call. Consumed once per call.
/// Used by the fallback-on-interrupt probe to inject transient vs
/// terminal refresh outcomes deterministically.
#[derive(Debug, Clone, Copy)]
pub enum RefreshFailureScript {
    /// Emit a transient refresh error (`TransientNetwork`).
    Transient,
    /// Emit a terminal refresh error (`TokenExpired`).
    Terminal,
}

static FAIL_NEXT_REFRESH: std::sync::Mutex<Option<RefreshFailureScript>> =
    std::sync::Mutex::new(None);

/// Install (or clear with `None`) a scripted failure for the next call
/// to [`RefreshableFixtureCredential::refresh`]. The script is consumed
/// (set back to `None`) by the next refresh invocation.
pub fn set_refresh_failure(script: Option<RefreshFailureScript>) {
    let mut guard = match FAIL_NEXT_REFRESH.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    *guard = script;
}

/// A minimal refreshable credential used only by the test suite. Static
/// in shape (token-bearing) but it implements [`Refreshable`]: each
/// `refresh` bumps the counter and rotates the token to
/// `"{original}-r{n}"`, returning [`RefreshOutcome::Refreshed`].
///
/// Projecting to a [`SecretToken`] keeps the consumer-facing scheme
/// identical to the real `bearer_token` builtin, so the facade's
/// snapshot/redaction path is exercised unchanged.
pub struct RefreshableFixtureCredential;

impl RefreshableFixtureCredential {
    /// The registered key for this fixture.
    pub const KEY: &'static str = "refreshable_fixture";
}

impl Credential for RefreshableFixtureCredential {
    type Properties = RefreshableFixtureProperties;
    type Scheme = SecretToken;
    type State = RefreshableFixtureState;

    const KEY: &'static str = "refreshable_fixture";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("refreshable_fixture"))
            .name("Refreshable Fixture")
            .description("Test-only refreshable credential (rotates its token on refresh).")
            .schema(nebula_schema::schema_of::<Self::Properties>())
            .pattern(AuthPattern::SecretToken)
            .icon("key")
            .build()
            .expect("refreshable_fixture metadata is valid")
    }

    fn project(state: &RefreshableFixtureState) -> SecretToken {
        SecretToken::new(SecretString::new(state.token.clone()))
    }

    async fn resolve(
        values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<RefreshableFixtureState, ()>, CredentialError> {
        let token = values.get_string_by_str("token").ok_or_else(|| {
            CredentialError::Provider(Box::new(ProviderErrorContext::new(
                ProviderErrorKind::Schema,
                SecretFreeMessage::new("missing required field 'token'"),
            )))
        })?;
        Ok(ResolveResult::Complete(RefreshableFixtureState {
            token: token.to_owned(),
            refresh_count: 0,
            expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
        }))
    }
}

impl Refreshable for RefreshableFixtureCredential {
    async fn refresh(
        state: &mut RefreshableFixtureState,
        _ctx: &CredentialContext,
    ) -> Result<RefreshOutcome, CredentialError> {
        // Honor any installed scripted failure — fires on EVERY refresh
        // call (not just once) so retry budgets cannot mask it. The test
        // clears it explicitly via `set_refresh_failure(None)` between
        // probe arms. Avoids local-state mutation if firing so the
        // cached row stays as last successful refresh wrote it.
        let scripted = {
            let guard = match FAIL_NEXT_REFRESH.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            *guard
        };
        if let Some(script) = scripted {
            return Err(match script {
                RefreshFailureScript::Transient => {
                    CredentialError::RefreshFailed(Box::new(RefreshFailedContext::new(
                        RefreshErrorKind::TransientNetwork,
                        RetryAdvice::After(std::time::Duration::from_millis(50)),
                        SecretFreeMessage::new("scripted transient refresh failure"),
                    )))
                },
                RefreshFailureScript::Terminal => {
                    CredentialError::RefreshFailed(Box::new(RefreshFailedContext::new(
                        RefreshErrorKind::TokenExpired,
                        RetryAdvice::Never,
                        SecretFreeMessage::new("scripted terminal refresh failure"),
                    )))
                },
            });
        }

        // Rotate the token deterministically and bump the counter so a
        // test can prove the *mutated* state was the thing re-persisted
        // under CAS (not the pre-refresh copy).
        state.refresh_count += 1;
        // Re-derive from the stable base (strip any prior `-rN` suffix)
        // so repeated refreshes produce `base-r1`, `base-r2`, … rather
        // than compounding.
        let base = state
            .token
            .split_once("-r")
            .map_or(state.token.as_str(), |(b, _)| b)
            .to_owned();
        state.token = format!("{base}-r{}", state.refresh_count);

        // If a test installed a rendezvous, park here — after the local
        // mutation, before the service's compare-and-swap re-persist — so
        // the test can deterministically land a concurrent version bump
        // and exercise the lost-CAS branch. Take the Arc out under the
        // lock and drop the guard before awaiting (the std Mutex guard is
        // not Send across the await).
        let rendezvous = REFRESH_RENDEZVOUS.lock().unwrap().clone();
        if let Some(barrier) = rendezvous {
            barrier.wait().await;
        }
        Ok(RefreshOutcome::Refreshed)
    }
}

// Capability report: required by `CredentialRegistry::register`. The
// fixture is refreshable only.
impl plugin_capability_report::IsInteractive for RefreshableFixtureCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsRefreshable for RefreshableFixtureCredential {
    const VALUE: bool = true;
}
impl plugin_capability_report::IsRevocable for RefreshableFixtureCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsTestable for RefreshableFixtureCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsDynamic for RefreshableFixtureCredential {
    const VALUE: bool = false;
}
