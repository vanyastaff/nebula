use nebula_sdk::prelude::{ActionMetadata, CheckpointPolicy};

fn main() {
    let _ = CheckpointPolicy::Inherit;
    let _ = core::mem::size_of::<ActionMetadata>();
}
