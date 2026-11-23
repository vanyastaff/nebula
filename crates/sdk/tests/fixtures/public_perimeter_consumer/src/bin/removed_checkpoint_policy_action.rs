use nebula_sdk::action::{ActionMetadata, CheckpointPolicy};

fn main() {
    let _ = CheckpointPolicy::Inherit;
    let _ = core::mem::size_of::<ActionMetadata>();
}
