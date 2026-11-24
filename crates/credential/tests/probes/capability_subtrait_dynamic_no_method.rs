//! Probe 3d — `impl Dynamic for Dummy {}` without `release()` body
//! fails with `E0046`. Per Tech Spec §15.4 the sub-trait carries no
//! defaulted method body; a credential declaring dynamic-lease
//! semantics cannot silently leak the lease at runtime.

use std::future::Future;

use nebula_credential::{
    Credential, CredentialContext, Metadata, Dynamic, SecretString,
    error::CredentialError, resolve::StaticResolveResult, scheme::SecretToken,
};
use nebula_credential::{Component, kind};
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

impl Component for Dummy {
    type Kind = kind::Credential;
    type Config = serde_json::Value;

    const KEY: &'static str = "dummy";

    fn metadata() -> Metadata {
        unimplemented!()
    }
}

impl Credential for Dummy {
    type Scheme = SecretToken;
    type State = DummyState;

    fn project(state: &DummyState) -> SecretToken {
        SecretToken::new(SecretString::new(state.token.clone()))
    }

    fn resolve(
        _properties: &Self::Config,
        _ctx: &CredentialContext,
    ) -> impl Future<Output = Result<StaticResolveResult<DummyState>, CredentialError>> + Send {
        async { unimplemented!() }
    }
}

// E0046 — `release` missing.
impl Dynamic for Dummy {}

fn main() {}
