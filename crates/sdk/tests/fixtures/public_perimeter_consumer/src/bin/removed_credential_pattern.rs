use nebula_sdk::prelude::{CredentialMetadata, metadata_name};

fn main() {
    let metadata = CredentialMetadata::new(metadata_name!("Token"), "");
    let _ = metadata.pattern();
}
