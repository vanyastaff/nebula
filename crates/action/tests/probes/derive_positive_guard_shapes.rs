//! Compile-pass probe: `#[derive(Action)]` accepts unit and named-field
//! structs without slots, and every slot shape an action holds:
//! `ResourceHandle<R>`, `CredentialSlot<C>`, and `Option` of either.

use nebula_action::{Action, Component};
use nebula_credential::{Credential, SecretToken};
use nebula_resource::{CredentialSlot, call::ResourceHandle, resource::Provider};

#[derive(Action)]
#[action(
    key = "positive.unit",
    name = "Unit",
    description = "smoke compile-pass for unit struct",
    config = serde_json::Value,
    output = serde_json::Value,
)]
struct UnitAction;

#[derive(Action)]
#[action(
    key = "positive.named",
    name = "Named",
    description = "smoke compile-pass for named-field struct without slots",
    config = serde_json::Value,
    output = serde_json::Value,
)]
struct NamedAction {
    #[expect(dead_code, reason = "the probe only compiles the field")]
    plain: u32,
}

#[derive(Action)]
#[action(
    key = "positive.slots",
    config = serde_json::Value,
    output = serde_json::Value,
)]
struct SlottedAction<R: Provider, C: Credential<Scheme = SecretToken>> {
    #[slot]
    db: ResourceHandle<R>,
    #[slot(key = "replica", purpose = "Read replica")]
    replica: Option<ResourceHandle<R>>,
    #[slot]
    token: CredentialSlot<C>,
    #[slot(key = "fallback_token")]
    fallback: Option<CredentialSlot<C>>,
}

/// The slot list follows the fields, in order.
#[expect(dead_code, reason = "the probe only compiles the generic check")]
fn slots_of<R: Provider, C: Credential<Scheme = SecretToken>>() -> Vec<&'static str> {
    <SlottedAction<R, C> as Component>::dependencies()
        .slots()
        .iter()
        .map(|slot| slot.key())
        .collect()
}

fn main() {
    let _meta = <UnitAction as Component>::metadata();
    let _named_meta = <NamedAction as Component>::metadata();
    assert!(<UnitAction as Component>::dependencies().is_empty());
    assert!(<NamedAction as Component>::dependencies().is_empty());
}
