//! Compile-pass probe: `#[derive(Action)]` accepts unit and named-field
//! structs without slot fields.
//!
//! This is a smoke pass-probe for the derive macro. The full slot-shape
//! matrix (`CredentialGuard<C>`, `Option<...>`, `Lazy<...>`,
//! `Option<Lazy<...>>`) is exercised by macro-internal unit tests in
//! `crates/action/macros/src/field_slots.rs` because the production
//! `resolve_credential_by_id::<C>` returns `CredentialGuard<C::Scheme>`,
//! not `CredentialGuard<C>` — the field-type and the resolver-return
//! type relationship is not currently expressible as a complete passing
//! integration probe without leaking the macro's internal scheme/Credential
//! type relationship into a public-API positive probe.

use nebula_action::Action;

#[derive(Action)]
#[action(
    key = "positive.unit",
    name = "Unit",
    description = "smoke compile-pass for unit struct",
    input = serde_json::Value,
    output = serde_json::Value,
)]
struct UnitAction;

#[derive(Action)]
#[action(
    key = "positive.named",
    name = "Named",
    description = "smoke compile-pass for named-field struct without slots",
    input = serde_json::Value,
    output = serde_json::Value,
)]
struct NamedAction {
    #[allow(dead_code)]
    plain: u32,
}

impl Default for NamedAction {
    fn default() -> Self {
        Self { plain: 0 }
    }
}

fn main() {
    let _meta = <UnitAction as Action>::metadata();
    let _named_meta = <NamedAction as Action>::metadata();
    assert!(
        <UnitAction as Action>::dependencies()
            .slot_fields()
            .is_empty()
    );
    assert!(
        <NamedAction as Action>::dependencies()
            .slot_fields()
            .is_empty()
    );
}
