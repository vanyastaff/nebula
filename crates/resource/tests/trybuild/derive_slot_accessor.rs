//! Compile-pass probe: `#[derive(Resource)]` accepts a named `#[credential]`
//! field of shape `SlotCell<CredentialGuard<C>>` and emits an inherent read
//! accessor `<field>_slot(&self) -> Option<Arc<CredentialGuard<C>>>` that
//! delegates to `SlotCell::load` (slot model, two-derive pattern).

use std::sync::Arc;

use nebula_credential::{
    AuthPattern, Credential, CredentialContext, CredentialError, CredentialGuard,
    CredentialMetadataDraft, SecretString, SecretToken, StaticResolveResult,
};
use nebula_resource::{Resource, SlotCell};
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

fn main() {
    let d = Demo {
        db: SlotCell::empty(),
    };
    // The derive-generated inherent accessor exists, type-checks, and returns
    // `None` while the slot is unresolved.
    let _maybe: Option<Arc<CredentialGuard<FakeCred>>> = d.db_slot();
    assert!(_maybe.is_none());
}
