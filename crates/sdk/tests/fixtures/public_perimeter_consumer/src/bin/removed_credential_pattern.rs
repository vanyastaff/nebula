use nebula_sdk::prelude::{Metadata, metadata_name};

fn main() {
    let metadata = Metadata::new(metadata_name!("Token"), "");
    let _ = metadata.pattern();
}
