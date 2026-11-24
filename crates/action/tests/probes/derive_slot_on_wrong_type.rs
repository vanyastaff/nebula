//! Compile-fail probe: a `#[slot]` field must hold a slot value. The derive
//! never reads the field type; the `SlotValue` and `FromSlot` traits refuse
//! anything else at the field type:
//!
//! - a plain value;
//! - a `ResourceGuard<R>` lease, removed in 0.27.0, since a lease bypasses
//!   the effect journal;
//! - `Lazy<...>`, since a slot is resolved before the action runs;
//! - a bare `CredentialGuard<S>`, which an action holds as `CredentialSlot<C>`.

use nebula_action::{Action, CredentialGuard};
use nebula_core::sync::Lazy;
use nebula_resource::{ResourceGuard, call::ResourceHandle, resource::Provider};

#[derive(Action)]
#[action(
    key = "bad.slot_plain_value",
    config = serde_json::Value,
    output = serde_json::Value,
)]
struct PlainValue {
    #[slot]
    count: u32,
}

#[derive(Action)]
#[action(
    key = "bad.slot_resource_guard",
    config = serde_json::Value,
    output = serde_json::Value,
)]
struct Lease<R: Provider> {
    #[slot]
    db: ResourceGuard<R>,
}

#[derive(Action)]
#[action(
    key = "bad.slot_lazy_handle",
    config = serde_json::Value,
    output = serde_json::Value,
)]
struct LazyRow<R: Provider> {
    #[slot]
    db: Lazy<ResourceHandle<R>>,
}

#[derive(Action)]
#[action(
    key = "bad.slot_bare_guard",
    config = serde_json::Value,
    output = serde_json::Value,
)]
struct BareGuard {
    #[slot]
    token: CredentialGuard<nebula_credential::SecretToken>,
}

fn main() {}
