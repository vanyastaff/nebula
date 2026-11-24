//! Compile-fail probe: `#[slot(key = "...")]` with an invalid slot key is
//! rejected at expansion time, at the literal. Invalid: a trailing `_`.

use nebula_credential::{
    Component, Credential, CredentialContext, CredentialError, Metadata, SecretString,
    SecretToken, StaticResolveResult, kind,
};
use nebula_resource::{CredentialSlot, Resource};

#[derive(Resource)]
#[resource(key = "probe.bad-key", config = ())]
struct Demo {
    #[slot(key = "bad_key_")]
    auth: CredentialSlot<FakeCred>,
}

struct FakeCred;

impl Component for FakeCred {
    type Kind = kind::Credential;
    type Config = ();

    const KEY: &'static str = "probe.fake";

    fn metadata() -> Metadata {
        Metadata::new(nebula_credential::metadata_name!("FakeCred"), "probe fixture")
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
