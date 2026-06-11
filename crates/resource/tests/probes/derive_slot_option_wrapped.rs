//! Compile-fail probe: `#[derive(ResourceSlots)]` rejects a `#[credential]`
//! field whose type is `Option<SlotCell<CredentialGuard<C>>>`. The slot field
//! must be exactly `SlotCell<CredentialGuard<C>>` or the alias `CredentialSlot<C>`.

use nebula_credential::{
    AuthPattern, Credential, CredentialContext, CredentialError, CredentialGuard,
    CredentialMetadata, ResolveResult, SecretString, SecretToken,
};
use nebula_resource::{ResourceSlots, SlotCell};
use nebula_schema::FieldValues;
use zeroize::Zeroize;

#[derive(ResourceSlots)]
struct Demo {
    #[credential(key = "db")]
    db: Option<SlotCell<CredentialGuard<FakeCred>>>,
}

#[derive(Clone, Default)]
struct DemoCfg;
impl nebula_schema::HasSchema for DemoCfg {
    fn schema() -> nebula_schema::ValidSchema {
        nebula_schema::ValidSchema::empty()
    }
}
impl nebula_resource::ResourceConfig for DemoCfg {
    fn fingerprint(&self) -> u64 {
        0
    }
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
            .description("trybuild slot-accessor fixture")
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
