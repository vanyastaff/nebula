//! Compile-fail probe: a `#[slot]` field whose type is not a resource slot
//! value is rejected at the field type, through the `SlotValue` and
//! `ResourceSlot` traits.

use nebula_resource::Resource;

#[derive(Resource)]
#[resource(key = "probe.wrong-type", config = ())]
struct Demo {
    #[slot]
    auth: String,
}

fn main() {}
