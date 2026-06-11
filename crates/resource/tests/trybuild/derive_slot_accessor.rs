//! Compile-pass probe: `#[derive(Resource)]` accepts a named `#[credential]`
//! field of shape `SlotCell<CredentialGuard<C>>` and emits an inherent read
//! accessor `<field>_slot(&self) -> Option<Arc<CredentialGuard<C>>>` that
//! delegates to `SlotCell::load` (slot model, two-derive pattern).

use std::sync::Arc;

use nebula_credential::{
    AuthPattern, Credential, CredentialContext, CredentialError, CredentialGuard,
    CredentialMetadata, ResolveResult, SecretString, SecretToken,
};
use nebula_resource::{Resource, SlotCell};
use nebula_schema::FieldValues;
use zeroize::Zeroize;

#[derive(Resource)]
struct Demo {
    #[credential(key = "db")]
    db: SlotCell<CredentialGuard<FakeCred>>,
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

fn main() {
    let d = Demo {
        db: SlotCell::empty(),
    };
    // The derive-generated inherent accessor exists, type-checks, and returns
    // `None` while the slot is unresolved.
    let _maybe: Option<Arc<CredentialGuard<FakeCred>>> = d.db_slot();
    assert!(_maybe.is_none());
}
