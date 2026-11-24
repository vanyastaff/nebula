use nebula_sdk::action::{CheckpointPolicy, Metadata};

fn main() {
    let _ = CheckpointPolicy::Inherit;
    let _ = core::mem::size_of::<Metadata>();
}
