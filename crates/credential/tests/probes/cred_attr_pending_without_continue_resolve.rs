//! `#[credential]` must reject `type Pending` without a `fn continue_resolve`
//! — the orphan half of the Interactive pair (mirror of
//! `cred_attr_continue_resolve_without_pending`).

// The macro rejects the impl, so its imports go unused — silence that noise so
// the captured .stderr pins only the macro diagnostic.
#![allow(unused_imports)]

use nebula_credential::{
    CredentialContext, SecretString, error::CredentialError, resolve::StaticResolveResult,
    scheme::SecretToken,
};

struct Bad;

#[nebula_credential::credential(key = "bad", name = "Bad")]
impl Bad {
    type Properties = serde_json::Value;
    type Scheme = SecretToken;
    type State = SecretToken;
    // Orphan — `type Pending` with no `fn continue_resolve` to consume it.
    type Pending = SecretToken;

    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }

    async fn resolve(
        _properties: &Self::Properties,
        _ctx: &CredentialContext,
    ) -> Result<StaticResolveResult<SecretToken>, CredentialError> {
        Ok(StaticResolveResult::Complete(SecretToken::new(
            SecretString::new("t"),
        )))
    }
}

fn main() {}
