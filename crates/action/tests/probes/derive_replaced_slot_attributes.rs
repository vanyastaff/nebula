//! Compile-fail probe: the pre-0.42 `#[resource]` and `#[credential]` field
//! attributes are refused with a pointer to `#[slot]`.

use nebula_action::Action;
use nebula_resource::call::ResourceHandle;

#[derive(Action)]
#[action(
    key = "bad.replaced_resource_attribute",
    config = serde_json::Value,
    output = serde_json::Value,
)]
struct OldResource<R: nebula_resource::resource::Provider> {
    #[resource]
    db: ResourceHandle<R>,
}

#[derive(Action)]
#[action(
    key = "bad.replaced_credential_attribute",
    config = serde_json::Value,
    output = serde_json::Value,
)]
struct OldCredential {
    #[credential(key = "auth")]
    token: u32,
}

fn main() {}
