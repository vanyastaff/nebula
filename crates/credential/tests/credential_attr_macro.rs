//! Integration coverage for the `#[nebula_credential::credential]` attribute
//! macro (ADR-0088 D1).
//!
//! The built-in credentials (api_key / basic_auth / oauth2) are dogfooded onto
//! the macro and their own suites are the primary oracle, but they only
//! exercise the `StaticSecret` and `RefreshPair` shapes. These synthetic
//! credentials cover the paths the built-ins do not:
//!
//! - capability **inference from method presence** for every sub-trait, folded
//!   through [`compute_capabilities`],
//! - the **synthesized** `CredentialLifecycle::policy()` for the
//!   refresh / lease / revoke strategy branches (the built-ins either are
//!   static or hand-write their policy),
//! - the **synthesized** `metadata()` (the built-ins all hand-write theirs).
//!
//! Every state is `SecretToken` (itself a `CredentialState` via
//! `identity_state!`), so the fixtures need no bespoke state type.

use nebula_credential::{
    Capabilities, Credential, CredentialCategory, CredentialContext, CredentialLifecycle,
    RefreshStrategy, RevokeStrategy, SecretString, compute_capabilities,
    error::CredentialError,
    resolve::{RefreshOutcome, ResolveResult, TestResult},
    scheme::SecretToken,
};
use nebula_metadata::Metadata;
use nebula_schema::FieldValues;

fn token() -> SecretToken {
    SecretToken::new(SecretString::new("t"))
}

// ── Refreshable-only: synth metadata + synth policy (RefreshToken) ────────

struct RefreshOnly;

#[nebula_credential::credential(
    key = "test_refresh_only",
    category = RefreshPair,
    name = "Refresh Only",
    description = "fixture",
    icon = "sync"
)]
impl RefreshOnly {
    type Properties = FieldValues;
    type Scheme = SecretToken;
    type State = SecretToken;

    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }

    async fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<SecretToken, ()>, CredentialError> {
        Ok(ResolveResult::Complete(token()))
    }

    async fn refresh(
        _state: &mut SecretToken,
        _ctx: &CredentialContext,
    ) -> Result<RefreshOutcome, CredentialError> {
        Ok(RefreshOutcome::Refreshed)
    }
}

#[test]
fn refresh_only_infers_refreshable_and_synthesizes_refresh_token_policy() {
    assert_eq!(
        compute_capabilities::<RefreshOnly>(),
        Capabilities::REFRESHABLE,
        "only `fn refresh` is present, so only REFRESHABLE should be reported"
    );
    let p = RefreshOnly::policy(&token());
    assert_eq!(p.category, CredentialCategory::RefreshPair);
    assert_eq!(p.refresh, RefreshStrategy::RefreshToken);
    assert_eq!(p.revoke, RevokeStrategy::None);
    assert!(p.is_auto_renewable());
}

#[test]
fn refresh_only_synthesizes_metadata_from_args() {
    let meta = RefreshOnly::metadata();
    assert_eq!(meta.name(), "Refresh Only");
}

// ── Dynamic (leased): synth policy (Lease) ────────────────────────────────

struct LeasedThing;

#[nebula_credential::credential(key = "test_leased", category = Leased, name = "Leased Thing")]
impl LeasedThing {
    type Properties = FieldValues;
    type Scheme = SecretToken;
    type State = SecretToken;

    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }

    async fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<SecretToken, ()>, CredentialError> {
        Ok(ResolveResult::Complete(token()))
    }

    async fn release(
        _state: &SecretToken,
        _ctx: &CredentialContext,
    ) -> Result<(), CredentialError> {
        Ok(())
    }
}

#[test]
fn release_infers_dynamic_and_synthesizes_lease_policy() {
    assert_eq!(
        compute_capabilities::<LeasedThing>(),
        Capabilities::DYNAMIC,
        "only `fn release` is present, so only DYNAMIC should be reported"
    );
    let p = LeasedThing::policy(&token());
    assert_eq!(p.category, CredentialCategory::Leased);
    assert_eq!(p.refresh, RefreshStrategy::Lease);
    assert_eq!(p.revoke, RevokeStrategy::None);
}

// ── Revocable + Testable: multi-flag inference + synth revoke (HandleBased) ─

struct RevTest;

#[nebula_credential::credential(key = "test_revtest", category = StaticSecret, name = "Rev Test")]
impl RevTest {
    type Properties = FieldValues;
    type Scheme = SecretToken;
    type State = SecretToken;

    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }

    async fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<SecretToken, ()>, CredentialError> {
        Ok(ResolveResult::Complete(token()))
    }

    async fn revoke(
        _state: &mut SecretToken,
        _ctx: &CredentialContext,
    ) -> Result<(), CredentialError> {
        Ok(())
    }

    async fn test(
        _scheme: &SecretToken,
        _ctx: &CredentialContext,
    ) -> Result<TestResult, CredentialError> {
        Ok(TestResult::Success)
    }
}

#[test]
fn revoke_and_test_infer_both_flags_and_synthesize_handle_based_revoke() {
    assert_eq!(
        compute_capabilities::<RevTest>(),
        Capabilities::REVOCABLE | Capabilities::TESTABLE,
        "`fn revoke` + `fn test` present, neither refresh nor release"
    );
    let p = RevTest::policy(&token());
    assert_eq!(p.category, CredentialCategory::StaticSecret);
    assert_eq!(p.refresh, RefreshStrategy::Static);
    assert_eq!(p.revoke, RevokeStrategy::HandleBased);
    assert!(!p.is_auto_renewable());
}
