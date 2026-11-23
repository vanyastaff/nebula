use nebula_sdk::prelude::{MetadataDraft, resource_key};

fn main() {
    let _ = MetadataDraft::from_key(resource_key!("example.resource"));
}
