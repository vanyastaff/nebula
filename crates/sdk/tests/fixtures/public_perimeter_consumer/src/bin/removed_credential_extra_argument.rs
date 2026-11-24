use nebula_sdk::prelude::{Metadata, metadata_name};

fn main() {
    let _ = Metadata::new(metadata_name!("Token"), "", ());
}
