//! Compile-fail probe: `#[derive(Resource)]` rejects `#[credential]`
//! on a tuple-struct field. Named-field structs are the only accepted form
//! for slot declarations.

use nebula_credential::{
    AuthPattern, Credential, CredentialContext, CredentialError, CredentialGuard,
    CredentialMetadata, ResolveResult, SecretString, SecretToken,
};
use nebula_resource::{Resource, SlotCell};
use nebula_schema::FieldValues;
use zeroize::Zeroize;

#[derive(Resource)]
struct TupleResource(#[credential(key = "auth")] SlotCell<CredentialGuard<FakeCred>>);

struct FakeCred;
impl Zeroize for FakeCred {
    fn zeroize(&mut self) {}
}
impl Credential for FakeCred {
    type Properties = ();
    type Scheme = SecretToken;
    type State = SecretToken;
    const KEY: &'static str = "fake.cred";
    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("fake.cred"))
            .name("FakeCred")
            .description("fixture")
            .schema(nebula_credential::schema_of::<Self::Properties>())
            .pattern(AuthPattern::SecretToken)
            .build()
            .expect("FakeCred metadata is valid")
    }
    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }
    async fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<SecretToken, ()>, CredentialError> {
        Ok(ResolveResult::Complete(SecretToken::new(
            SecretString::new("t"),
        )))
    }
}

fn main() {}
