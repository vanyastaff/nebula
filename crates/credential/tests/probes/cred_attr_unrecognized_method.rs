//! `#[credential]` must reject an unrecognized method (here a typo'd
//! `refrsh`) rather than silently treating it as a non-capability helper.
//!
//! This is the safety property that replaces the old
//! declare-a-`capabilities(refreshable)`-flag-and-forget-the-`refresh`-impl
//! vector: with capability inferred from method presence, a misspelled
//! capability method must fail loudly at the macro site, never produce a
//! credential silently missing the capability.

// The macro rejects the impl, so its imports go unused — silence that noise so
// the captured .stderr pins only the macro diagnostic.
#![allow(unused_imports)]

use nebula_credential::{
    CredentialContext, SecretString, error::CredentialError, resolve::ResolveResult,
    scheme::SecretToken,
};
use nebula_schema::FieldValues;

struct Bad;

#[nebula_credential::credential(key = "bad", category = StaticSecret, name = "Bad")]
impl Bad {
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
        Ok(ResolveResult::Complete(SecretToken::new(SecretString::new("t"))))
    }

    // Typo — not a recognized capability method.
    async fn refrsh(
        _state: &mut SecretToken,
        _ctx: &CredentialContext,
    ) -> Result<(), CredentialError> {
        Ok(())
    }
}

fn main() {}
