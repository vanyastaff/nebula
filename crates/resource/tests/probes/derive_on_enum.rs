//! Compile-fail probe: `#[derive(Resource)]` rejects enums.
//! Only structs are accepted.

use nebula_resource::Resource;

#[derive(Resource)]
enum NotAStruct {
    A,
    B,
}

fn main() {}
