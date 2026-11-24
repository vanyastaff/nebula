//! Compile-fail probe: the `<Name>PinnedSlots` snapshot that
//! `#[derive(Resource)]` emits for the managed call facade keeps its guard
//! `Arc`s private. A unit reads a pinned slot only through the `<field>()`
//! accessor, which borrows the guard; it cannot take the `Arc` out and keep
//! the material past the unit.

use nebula_credential::{
    Component, Credential, CredentialContext, CredentialError, Metadata, SecretString,
    SecretToken, StaticResolveResult, kind,
};
use nebula_resource::{CredentialSlot, PinSlots};

mod resources {
    use nebula_resource::{CredentialSlot, Resource};

    #[derive(Resource)]
    #[resource(key = "probe.demo", config = ())]
    pub struct Demo {
        #[slot]
        pub db: CredentialSlot<super::FakeCred>,
    }
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

fn main() {
    let demo = resources::Demo {
        db: CredentialSlot::empty(),
    };
    let pinned = demo.pin_slots();
    let _kept = pinned.db;
}
