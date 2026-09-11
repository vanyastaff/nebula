use std::any::Any;

use nebula_credential::{AnyCredential, Capabilities, CredentialMetadata};

struct SelfAttested;

impl AnyCredential for SelfAttested {
    fn credential_key(&self) -> &str {
        "self_attested"
    }

    fn metadata(
        &self,
    ) -> Result<CredentialMetadata, nebula_credential::CredentialMetadataAdmissionError> {
        unimplemented!()
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::empty()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn main() {}
