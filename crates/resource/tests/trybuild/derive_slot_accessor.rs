//! Compile-pass probe: `#[derive(Resource)]` accepts a `#[slot]` field of
//! type `CredentialSlot<C>` and emits an inherent read accessor
//! `<field>_slot(&self) -> Option<Arc<CredentialGuard<C::Scheme>>>`, the
//! `SLOT_<KEY>` constant, the pinned `<field>()` accessor and the resolver
//! install bridge.

use std::sync::Arc;

use nebula_credential::{
    Component, Credential, CredentialContext, CredentialError, CredentialGuard, Metadata,
    SecretString, SecretToken, StaticResolveResult, kind,
};
use nebula_resource::{CredentialSlot, HasCredentialSlots, PinSlots, Resource};

#[derive(Resource)]
#[resource(key = "probe.demo", config = ())]
struct Demo {
    #[slot(key = "api", purpose = "API token")]
    token: CredentialSlot<FakeCred>,
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
    let demo = Demo {
        token: CredentialSlot::empty(),
    };
    // The derive-generated inherent accessor returns `None` while the slot
    // is unbound.
    let current: Option<Arc<CredentialGuard<SecretToken>>> = demo.token_slot();
    assert!(current.is_none());
    let _: &str = Demo::SLOT_API;

    // The derive-emitted `PinSlots` snapshot borrows each pinned guard.
    let pinned: DemoPinnedSlots = demo.pin_slots();
    let pinned_token: Option<&CredentialGuard<SecretToken>> = pinned.token();
    assert!(pinned_token.is_none());

    // The slot is declared on the component.
    let dependencies = <Demo as nebula_resource::Component>::dependencies();
    assert_eq!(dependencies.slots().len(), 1);

    fn install_from_resolver(
        resource: &Demo,
        guard: nebula_credential::ErasedCredentialGuard,
    ) -> Result<nebula_resource::SlotUpdate, nebula_resource::SlotInstallError> {
        resource.install_credential_slot("api", guard)
    }
    let _object_safe_bridge_contract = install_from_resolver;
}
