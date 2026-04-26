//! Probe 3c — `impl Testable for Dummy {}` without `test()` body fails
//! with `E0046`. Per Tech Spec §15.4 the sub-trait carries no
//! defaulted method body; a credential declaring testability cannot
//! silently return the "not testable" sentinel at runtime.

use std::future::Future;

use nebula_credential::{
    Credential, CredentialContext, CredentialMetadata, Testable,
    error::CredentialError,
    resolve::ResolveResult,
    scheme::SecretToken,
    SecretString,
};
use nebula_schema::FieldValues;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
struct DummyState {
    token: String,
}

impl nebula_credential::CredentialState for DummyState {
    const KIND: &'static str = "dummy_state";
    const VERSION: u32 = 1;
}

struct Dummy;

impl Credential for Dummy {
    type Input = FieldValues;
    type Scheme = SecretToken;
    type State = DummyState;

    const KEY: &'static str = "dummy";

    fn metadata() -> CredentialMetadata {
        unimplemented!()
    }

    fn project(state: &DummyState) -> SecretToken {
        SecretToken::new(SecretString::new(state.token.clone()))
    }

    fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> impl Future<Output = Result<ResolveResult<DummyState, ()>, CredentialError>> + Send {
        async { unimplemented!() }
    }
}

// E0046 — `test` missing.
impl Testable for Dummy {}

fn main() {}
