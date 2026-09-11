//! Compile-fail probe: `#[derive(Resource)]` rejects `#[credential]`
//! on a tuple-struct field. Named-field structs are the only accepted form
//! for slot declarations.

use nebula_credential::{
    AuthPattern, Credential, CredentialContext, CredentialError, CredentialGuard,
    CredentialMetadataDraft, SecretString, SecretToken, StaticResolveResult,
};
use nebula_resource::{Resource, SlotCell};
use zeroize::Zeroize;

#[derive(Resource)]
struct TupleResource(#[credential(key = "auth")] SlotCell<CredentialGuard<FakeCred>>);

struct FakeCred;
impl Zeroize for FakeCred {
    fn zeroize(&mut self) {}
}
impl Credential for FakeCred {
    type Properties = ();
    type Scheme = SecretToken;
    type State = SecretToken;
    const KEY: &'static str = "fake.cred";
    fn metadata() -> CredentialMetadataDraft {
        CredentialMetadataDraft::new(
            nebula_core::credential_key!("fake.cred"),
            nebula_credential::metadata_name!("FakeCred"),
            "fixture",
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
            SecretString::new("t"),
        )))
    }
}

fn main() {}
