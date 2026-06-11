//! Compile-fail probe: `#[credential]` on a field whose type is neither
//! `SlotCell<CredentialGuard<C>>` nor `CredentialSlot<C>` is rejected at the
//! field type span, naming both accepted shapes.

use nebula_resource::ResourceSlots;

#[derive(ResourceSlots)]
struct Demo {
    #[credential(key = "auth")]
    auth: String,
}

fn main() {}
