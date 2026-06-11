//! Compile-fail probe: `#[derive(ResourceSlots)]` rejects enums.
//! Only structs are accepted.

use nebula_resource::ResourceSlots;

#[derive(ResourceSlots)]
enum NotAStruct {
    A,
    B,
}

fn main() {}
