//! Compile-fail probe: `#[credential(key = "...")]` with an invalid key literal
//! is rejected at expansion time with a compile error at the literal span.
//! Invalid: trailing separator (`foo_`) violates CredentialKey rules.

use nebula_credential::{
    Credential, CredentialContext, CredentialError, CredentialGuard, Metadata,
    SecretString, SecretToken, StaticResolveResult,
};
use nebula_credential::{Component, kind};
use nebula_resource::{Resource, SlotCell};
use zeroize::Zeroize;

#[derive(Resource)]
struct Demo {
    #[credential(key = "bad_key_")]
    auth: SlotCell<CredentialGuard<FakeCred>>,
}

struct FakeCred;
impl Zeroize for FakeCred {
    fn zeroize(&mut self) {}
}
impl Component for FakeCred {
    type Kind = kind::Credential;
    type Config = ();

    const KEY: &'static str = "demo.fake";

    fn metadata() -> Metadata {
        Metadata::new(nebula_credential::metadata_name!("FakeCred"),
            "trybuild bad-key fixture",
        )
    }
}

impl Credential for FakeCred {
    type Scheme = SecretToken;
    type State = SecretToken;
    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }
    async fn resolve(
        _properties: &(),
        _ctx: &CredentialContext,
    ) -> Result<StaticResolveResult<SecretToken>, CredentialError> {
        Ok(StaticResolveResult::Complete(SecretToken::new(
            SecretString::new("fake-token"),
        )))
    }
}

fn main() {}
