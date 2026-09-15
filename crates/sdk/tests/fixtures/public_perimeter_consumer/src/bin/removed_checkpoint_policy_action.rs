use nebula_sdk::action::{ActionMetadataDraft, CheckpointPolicy};

fn main() {
    let _ = CheckpointPolicy::Inherit;
    let _ = core::mem::size_of::<ActionMetadataDraft>();
}
