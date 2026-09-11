//! Compile-fail probe: `#[derive(Resource)]` rejects a `#[credential]`
//! field whose type is `Option<SlotCell<CredentialGuard<C>>>`. The slot field
//! must be exactly `SlotCell<CredentialGuard<C>>` or the alias `CredentialSlot<C>`.

use nebula_credential::{
    AuthPattern, Credential, CredentialContext, CredentialError, CredentialGuard,
    CredentialMetadataDraft, SecretString, SecretToken, StaticResolveResult,
};
use nebula_resource::{Resource, SlotCell};
use zeroize::Zeroize;

#[derive(Resource)]
struct Demo {
    #[credential(key = "db")]
    db: Option<SlotCell<CredentialGuard<FakeCred>>>,
}

#[derive(Clone, Default)]
struct DemoCfg;
impl nebula_schema::HasSchema for DemoCfg {
    fn schema() -> Result<nebula_schema::ValidSchema, nebula_schema::ValidationReport> {
        Ok(nebula_schema::ValidSchema::empty())
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
    fn metadata() -> CredentialMetadataDraft {
        CredentialMetadataDraft::new(
            nebula_core::credential_key!("demo.fake"),
            nebula_credential::metadata_name!("FakeCred"),
            "trybuild slot-accessor fixture",
            AuthPattern::SecretToken,
        )
    }
    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }
    async fn resolve(
        _properties: &(),
        _ctx: &CredentialContext,
    ) -> Result<StaticResolveResult<SecretToken>, CredentialError> {
        Ok(StaticResolveResult::Complete(SecretToken::new(
            SecretString::new("fake-token"),
        )))
    }
}

fn main() {}
