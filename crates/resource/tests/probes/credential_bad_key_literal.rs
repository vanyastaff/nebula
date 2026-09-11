//! Compile-fail probe: `#[credential(key = "...")]` with an invalid key literal
//! is rejected at expansion time with a compile error at the literal span.
//! Invalid: trailing separator (`foo_`) violates CredentialKey rules.

use nebula_credential::{
    AuthPattern, Credential, CredentialContext, CredentialError, CredentialGuard,
    CredentialMetadataDraft, SecretString, SecretToken, StaticResolveResult,
};
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
impl Credential for FakeCred {
    type Properties = ();
    type Scheme = SecretToken;
    type State = SecretToken;
    const KEY: &'static str = "demo.fake";
    fn metadata() -> CredentialMetadataDraft {
        CredentialMetadataDraft::new(
            nebula_core::credential_key!("demo.fake"),
            nebula_credential::metadata_name!("FakeCred"),
            "trybuild bad-key fixture",
            AuthPattern::SecretToken,
        )
    }
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
