use nebula_sdk::prelude::{ResourceMetadataDraft, resource_key};

fn main() {
    let _ = ResourceMetadataDraft::from_key(resource_key!("example.resource"));
}
