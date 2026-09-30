//! Compile-fail probe: a `#[resource]` field spelled with the facade's
//! pre-0.21.0 name is refused with a hint naming `ResourceHandle`.

use nebula_action::Action;

/// Stands in for the removed type, so the only error is the derive's.
struct ManagedRow<R>(R);

#[derive(Action)]
#[action(
    key = "bad.renamed-resource-handle",
    input = serde_json::Value,
    output = serde_json::Value,
)]
struct OldRow<R: nebula_resource::resource::Provider> {
    #[resource]
    db: ManagedRow<R>,
}

fn main() {}
