//! `#[credential]` must reject `fn continue_resolve` without a matching
//! `type Pending` — the Interactive capability needs its typed pending state,
//! so the two are required as a pair.

// The macro rejects the impl, so its imports go unused — silence that noise so
// the captured .stderr pins only the macro diagnostic.
#![allow(unused_imports)]

use nebula_credential::{
    CredentialContext, SecretString, error::CredentialError,
    resolve::{ResolveResult, UserInput},
    scheme::SecretToken,
};
use nebula_schema::FieldValues;

struct Bad;

#[nebula_credential::credential(key = "bad", name = "Bad")]
impl Bad {
    type Properties = FieldValues;
    type Scheme = SecretToken;
    type State = SecretToken;
    // No `type Pending` — the macro must reject the interactive method below.

    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }

    async fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<SecretToken, ()>, CredentialError> {
        Ok(ResolveResult::Complete(SecretToken::new(SecretString::new("t"))))
    }

    async fn continue_resolve(
        _pending: &SecretToken,
        _input: &UserInput,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<SecretToken, SecretToken>, CredentialError> {
        unimplemented!()
    }
}

fn main() {}
