use std::any::Any;

use nebula_credential::{AnyCredential, Capabilities, CredentialDefinition};

struct SelfAttested;

impl AnyCredential for SelfAttested {
    fn key(&self) -> nebula_core::CredentialKey {
        unimplemented!()
    }

    fn definition(
        &self,
    ) -> Result<CredentialDefinition, nebula_credential::CredentialAdmissionError> {
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
