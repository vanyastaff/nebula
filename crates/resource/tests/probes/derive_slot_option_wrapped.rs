//! Compile-fail probe: `#[derive(Resource)]` rejects a `#[credential]`
//! field whose type is `Option<SlotCell<CredentialGuard<C>>>`. The slot field
//! must be exactly `SlotCell<CredentialGuard<C>>` or the alias `CredentialSlot<C>`.

use nebula_credential::{
    Credential, CredentialContext, CredentialError, CredentialGuard, Metadata,
    SecretString, SecretToken, StaticResolveResult,
};
use nebula_credential::{Component, kind};
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
impl Component for FakeCred {
    type Kind = kind::Credential;
    type Config = ();

    const KEY: &'static str = "demo.fake";

    fn metadata() -> Metadata {
        Metadata::new(nebula_credential::metadata_name!("FakeCred"),
            "trybuild slot-accessor fixture",
        )
    }
}

impl Credential for FakeCred {
    type Scheme = SecretToken;
    type State = SecretToken;
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
