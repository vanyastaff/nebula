//! Compile-fail probe: a resource slot cannot be `Option<CredentialSlot<C>>`.
//! `CredentialSlot<C>` is already empty until the engine binds it, so the
//! `Option` would only add a second way to say "unbound".

use nebula_credential::{
    Component, Credential, CredentialContext, CredentialError, Metadata, SecretString,
    SecretToken, StaticResolveResult, kind,
};
use nebula_resource::{CredentialSlot, Resource};

#[derive(Resource)]
#[resource(key = "probe.option", config = ())]
struct Demo {
    #[slot]
    db: Option<CredentialSlot<FakeCred>>,
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
