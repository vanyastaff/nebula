use nebula_sdk::prelude::{ResourceMetadata, resource_key};

fn main() {
    let _ = ResourceMetadata::from_key(resource_key!("example.resource"));
}
