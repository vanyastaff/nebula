use nebula_sdk::prelude::{Metadata, resource_key};

fn main() {
    let _ = Metadata::from_key(resource_key!("example.resource"));
}
