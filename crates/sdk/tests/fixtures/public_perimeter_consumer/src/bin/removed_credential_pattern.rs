use nebula_sdk::prelude::{CredentialMetadataDraft, credential_key, metadata_name};

fn main() {
    let draft = CredentialMetadataDraft::new(
        credential_key!("example.token"),
        metadata_name!("Token"),
        "",
    );
    let _ = draft.pattern();
}
