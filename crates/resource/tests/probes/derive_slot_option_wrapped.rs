//! Compile-fail probe: `#[derive(Resource)]` rejects an `Option<…>`-wrapped
//! `#[credential]` slot. The generated accessor emits a single fixed body that
//! only fits the plain `SlotCell<CredentialGuard<C>>` shape, so wrapper shapes
//! must be a compile error with a clear span at the derive site.

use nebula_credential::{
    AuthPattern, Credential, CredentialContext, CredentialError, CredentialGuard,
    CredentialMetadata, ResolveResult, SecretString, SecretToken,
};
use nebula_resource::{Resource, SlotCell};
use nebula_schema::FieldValues;
use zeroize::Zeroize;

#[derive(Resource)]
#[resource(key = "demo", topology = "resident", config = DemoCfg)]
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
impl nebula_resource::ResourceConfig for DemoCfg {}

/// Minimal static credential fixture. `Zeroize` is a no-op: `FakeCred` is a
/// unit type carrying no secret bytes — the bound exists only so the derived
/// accessor return type can name `CredentialGuard<FakeCred>`.
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
