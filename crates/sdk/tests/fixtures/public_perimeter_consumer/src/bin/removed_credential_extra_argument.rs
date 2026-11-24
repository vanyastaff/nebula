use nebula_sdk::prelude::{CredentialMetadata, metadata_name};

fn main() {
    let _ = CredentialMetadata::new(metadata_name!("Token"), "", ());
}
