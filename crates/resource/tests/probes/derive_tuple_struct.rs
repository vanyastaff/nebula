//! Compile-fail probe: `#[derive(Resource)]` rejects `#[credential]`
//! on a tuple-struct field. Named-field structs are the only accepted form
//! for slot declarations.

use nebula_credential::{
    Credential, CredentialContext, CredentialError, CredentialGuard, Metadata,
    SecretString, SecretToken, StaticResolveResult,
};
use nebula_credential::{Component, kind};
use nebula_resource::{Resource, SlotCell};
use zeroize::Zeroize;

#[derive(Resource)]
struct TupleResource(#[credential(key = "auth")] SlotCell<CredentialGuard<FakeCred>>);

struct FakeCred;
impl Zeroize for FakeCred {
    fn zeroize(&mut self) {}
}
impl Component for FakeCred {
    type Kind = kind::Credential;
    type Config = ();

    const KEY: &'static str = "fake.cred";

    fn metadata() -> Metadata {
        Metadata::new(nebula_credential::metadata_name!("FakeCred"),
            "fixture",
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
            SecretString::new("t"),
        )))
    }
}

fn main() {}
