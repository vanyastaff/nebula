//! Compile-fail probe: same field cannot have both `#[resource]` and `#[credential]`.

use nebula_action::{Action, CredentialGuard};

// Fake credential type for the field signature; the macro errors out
// before any trait checking on the inner type, so the probe only needs
// the field to *parse* as `CredentialGuard<T>`.
struct FakeCred;

#[derive(Action)]
#[action(
    key = "bad.both_slots",
    input = serde_json::Value,
    output = serde_json::Value,
)]
struct BothSlots {
    #[resource]
    #[credential]
    overloaded: CredentialGuard<FakeCred>,
}

fn main() {}
