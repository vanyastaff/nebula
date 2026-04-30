//! Compile-fail probe: `#[credential]` on a non-`CredentialGuard` field type.

use nebula_action::Action;

#[derive(Action)]
#[action(
    key = "bad.credential_wrong_type",
    input = serde_json::Value,
    output = serde_json::Value,
)]
struct CredentialWrongType {
    #[credential]
    not_a_guard: u32,
}

fn main() {}
