//! Probe 5 — §15.6 fatal duplicate-KEY registration (runtime).
//!
//! Per Tech Spec §16.1.1 — duplicate keys across crates are not
//! statically detectable by rustc alone (the colliding string is data,
//! not a type). This probe exercises the runtime contract on the
//! KEY-keyed [`CredentialRegistry`]: second registration of a credential
//! sharing an existing `KEY` returns
//! [`RegisterError::DuplicateKey`](nebula_credential::RegisterError),
//! the registry's first entry remains authoritative, and `resolve` of
//! the colliding KEY returns the first-registered concrete type — never
//! the rejected second.
//!
//! Closes security-lead N7 — supply-chain credential takeover via
//! duplicate KEY collision now blocks startup with an
//! operator-actionable error in BOTH debug and release builds.
//!
//! [`CredentialRegistry`]: nebula_credential::CredentialRegistry

use nebula_credential::{
    Credential, CredentialContext, CredentialMetadata, CredentialRegistry, RegisterError,
    SecretString, error::CredentialError, resolve::ResolveResult, scheme::SecretToken,
};
use nebula_schema::FieldValues;

const SHARED_KEY: &str = "shared.duplicate";

// ── Two distinct credential types sharing the same KEY ─────────────

/// First credential — registered first, expected to win.
pub struct CredA;

impl Credential for CredA {
    type Input = ();
    type Scheme = SecretToken;
    type State = SecretToken;

    const KEY: &'static str = SHARED_KEY;

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("shared.duplicate"))
            .name("CredA")
            .description("first credential — wins on collision")
            .schema(Self::schema())
            .pattern(nebula_credential::AuthPattern::SecretToken)
            .build()
            .expect("CredA metadata is valid")
    }

    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }

    async fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<SecretToken, ()>, CredentialError> {
        Ok(ResolveResult::Complete(SecretToken::new(
            SecretString::new("cred-a-token"),
        )))
    }
}

/// Second credential — registered second, expected to be rejected.
pub struct CredB;

impl Credential for CredB {
    type Input = ();
    type Scheme = SecretToken;
    type State = SecretToken;

    const KEY: &'static str = SHARED_KEY;

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("shared.duplicate"))
            .name("CredB")
            .description("second credential — rejected on collision")
            .schema(Self::schema())
            .pattern(nebula_credential::AuthPattern::SecretToken)
            .build()
            .expect("CredB metadata is valid")
    }

    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }

    async fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<SecretToken, ()>, CredentialError> {
        Ok(ResolveResult::Complete(SecretToken::new(
            SecretString::new("cred-b-token"),
        )))
    }
}

// ── Probe assertions ───────────────────────────────────────────────

#[test]
fn duplicate_key_returns_error_not_panic() {
    let mut registry = CredentialRegistry::new();
    let r1 = registry.register(CredA, env!("CARGO_CRATE_NAME"));
    assert!(r1.is_ok(), "first registration must succeed: {r1:?}");

    let r2 = registry.register(CredB, env!("CARGO_CRATE_NAME"));
    let err = r2.expect_err("second registration must error, not overwrite");

    match err {
        RegisterError::DuplicateKey {
            key,
            existing_crate,
            new_crate,
        } => {
            assert_eq!(
                key, SHARED_KEY,
                "DuplicateKey must carry the colliding KEY verbatim"
            );
            assert_eq!(
                existing_crate,
                env!("CARGO_CRATE_NAME"),
                "existing_crate must point to the first registrar"
            );
            assert_eq!(
                new_crate,
                env!("CARGO_CRATE_NAME"),
                "new_crate must point to the rejected registrar"
            );
        },
        // `RegisterError` is `#[non_exhaustive]` to extend with future
        // registration-time validations (Tech Spec §15.6 + post-MVP
        // `arch-signing-infra`). Stage 5 has only the one variant; the
        // wildcard documents intent to fail loud if the enum grows.
        other => panic!("expected DuplicateKey, got {other:?}"),
    }

    // Registry state unchanged after the rejected duplicate.
    assert_eq!(
        registry.len(),
        1,
        "registry must still contain exactly one entry"
    );
    assert!(
        registry.contains(SHARED_KEY),
        "first registration must remain present after rejected duplicate"
    );
}

#[test]
fn duplicate_key_first_wins() {
    let mut registry = CredentialRegistry::new();
    registry
        .register(CredA, env!("CARGO_CRATE_NAME"))
        .expect("first registration must succeed");
    let _rejected = registry.register(CredB, env!("CARGO_CRATE_NAME"));

    // Resolve by KEY — must return CredA (the first registration), not CredB.
    let resolved_a: Option<&CredA> = registry.resolve::<CredA>(SHARED_KEY);
    assert!(
        resolved_a.is_some(),
        "first registration (CredA) must remain authoritative — \
         downcast<CredA> must succeed"
    );

    // Concrete-type mismatch: downcast to CredB must fail because the
    // entry is a CredA. This pins first-wins at the type-erasure level
    // (and is the load-bearing guarantee for §15.6 supply-chain risk:
    // a malicious second registration cannot replace the original even
    // via stale `Any::type_id` reuse).
    let resolved_b: Option<&CredB> = registry.resolve::<CredB>(SHARED_KEY);
    assert!(
        resolved_b.is_none(),
        "rejected registration (CredB) must NOT be reachable via downcast"
    );
}

#[test]
fn duplicate_error_message_is_operator_actionable() {
    let mut registry = CredentialRegistry::new();
    registry
        .register(CredA, env!("CARGO_CRATE_NAME"))
        .expect("first registration must succeed");
    let err = registry
        .register(CredB, env!("CARGO_CRATE_NAME"))
        .expect_err("second registration must error");
    let msg = err.to_string();
    assert!(
        msg.contains("duplicate credential key"),
        "error message must identify the failure class"
    );
    assert!(
        msg.contains(SHARED_KEY),
        "error message must include the colliding KEY"
    );
    assert!(
        msg.contains("Tech Spec §15.6"),
        "error message must cite the closing spec section for operator escalation"
    );
}
