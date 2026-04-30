//! Compile-fail probe: two `#[credential(key = "...")]` slots with the same
//! key on different fields are rejected at macro expansion.

use nebula_action::{Action, CredentialGuard};

struct FakeCred;

#[derive(Action)]
#[action(
    key = "bad.dup_slot_keys",
    input = serde_json::Value,
    output = serde_json::Value,
)]
struct ConflictingSlotKeys {
    #[credential(key = "shared")]
    cred_a: CredentialGuard<FakeCred>,
    #[credential(key = "shared")]
    cred_b: CredentialGuard<FakeCred>,
}

fn main() {}
