//! Compile-fail probe: `#[credential(key = "...")]` with an invalid key literal
//! is rejected at expansion time with a compile error at the literal span.
//! Invalid: trailing separator (`foo_`) violates CredentialKey rules.

use nebula_credential::{
    AuthPattern, Credential, CredentialContext, CredentialError, CredentialGuard,
    CredentialMetadata, ResolveResult, SecretString, SecretToken,
};
use nebula_resource::{ResourceSlots, SlotCell};
use nebula_schema::FieldValues;
use zeroize::Zeroize;

#[derive(ResourceSlots)]
struct Demo {
    #[credential(key = "bad_key_")]
    auth: SlotCell<CredentialGuard<FakeCred>>,
}

struct FakeCred;
impl Zeroize for FakeCred {
    fn zeroize(&mut self) {}
}
impl Credential for FakeCred {
    type Properties = ();
    type Scheme = SecretToken;
    type State = SecretToken;
    const KEY: &'static str = "demo.fake";
    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("demo.fake"))
            .name("FakeCred")
            .description("trybuild bad-key fixture")
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
            SecretString::new("fake-token"),
        )))
    }
}

fn main() {}
