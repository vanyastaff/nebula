//! Compile-fail probe: the `<Name>PinnedSlots` snapshot that
//! `#[derive(Resource)]` emits for the managed call facade keeps its guard
//! `Arc`s private. A unit reads a pinned slot only through the `<field>()`
//! accessor, which borrows the guard; it cannot take the `Arc` out and keep
//! the material past the unit.

use nebula_credential::{
    Credential, CredentialContext, CredentialError, CredentialGuard, CredentialMetadataDraft,
    SecretString, SecretToken, StaticResolveResult,
};
use nebula_resource::{PinSlots, SlotCell};
use zeroize::Zeroize;

mod resources {
    use nebula_credential::CredentialGuard;
    use nebula_resource::{Resource, SlotCell};

    #[derive(Resource)]
    pub struct Demo {
        #[credential(key = "db")]
        pub db: SlotCell<CredentialGuard<super::FakeCred>>,
    }
}

struct FakeCred;

impl Zeroize for FakeCred {
    fn zeroize(&mut self) {}
}

impl Credential for FakeCred {
    type Properties = ();
    type Scheme = SecretToken;
    type State = SecretToken;

    const KEY: &'static str = "probe.fake";

    fn metadata() -> CredentialMetadataDraft {
        CredentialMetadataDraft::new(
            nebula_core::credential_key!("probe.fake"),
            nebula_credential::metadata_name!("FakeCred"),
            "pinned-slot privacy probe",
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

fn main() {
    let demo = resources::Demo {
        db: SlotCell::<CredentialGuard<FakeCred>>::empty(),
    };
    let pinned = demo.pin_slots();
    let _kept = pinned.db;
}
