//! Probe 2 — impl Refreshable for X without refresh() body fails.
//!
//! Expected: E0046 — not all trait items implemented.

use credential_proto::{
    Credential, CredentialContext, CredentialMetadata, CredentialState, HasInputSchema,
    Refreshable, ResolveError, ResolveResult, Sealed,
};
use credential_proto_builtin::{BearerScheme, ApiKeyState};

struct NaughtyCred;

impl Sealed for NaughtyCred {}

impl Credential for NaughtyCred {
    type Input = ();
    type Scheme = BearerScheme;
    type State = ApiKeyState;
    const KEY: &'static str = "naughty";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata { key: Self::KEY, crate_name: "probe" }
    }
    fn project(state: &Self::State) -> Self::Scheme {
        BearerScheme { token: state.token.clone() }
    }
    fn resolve(
        _ctx: &CredentialContext<'_>,
        _input: &Self::Input,
    ) -> Result<ResolveResult<Self::State, ()>, ResolveError> {
        Err(ResolveError("stub".into()))
    }
}

// MISSING: fn refresh(...).
impl Refreshable for NaughtyCred {}

fn main() {}
