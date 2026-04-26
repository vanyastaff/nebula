//! Probe - Pattern 2 wrong-scheme rejection. See parent driver
//! `compile_fail_pattern2_service_reject.rs`.
//!
//! Self-contained: defines its own `AcceptsBearer` marker, `BasicScheme`,
//! and service supertrait. Uses `#[capability]` to emit the real /
//! sealed / phantom triple. A credential with `Scheme = BasicScheme`
//! is wired into `&dyn MyServiceBearerPhantom` - rejected by E0277
//! through the phantom-shim chain.

use std::future::Future;

use nebula_credential::{
    AuthScheme, AuthPattern, Credential, CredentialContext, CredentialMetadata,
    error::CredentialError,
    resolve::ResolveResult,
};
use nebula_schema::FieldValues;
use serde::{Deserialize, Serialize};

// Per ADR-0035 4.1, the crate author declares `mod sealed_caps`
// manually at crate root with one inner trait per capability. The
// fixture is the "crate" here.
mod sealed_caps {
    pub trait BearerSealed {}
}

// Local capability marker - in production this would come from
// nebula_credential at a later Stage.
pub trait AcceptsBearer: AuthScheme {}

// A scheme that does NOT implement AcceptsBearer.
#[derive(Clone, Serialize, Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
struct BasicScheme {
    user: String,
    pass: String,
}

impl AuthScheme for BasicScheme {
    fn pattern() -> AuthPattern {
        AuthPattern::IdentityPassword
    }
}
// Deliberately NOT impl AcceptsBearer for BasicScheme.

// Service supertrait - a fake "MyService" credential family.
pub trait MyService: Credential {}

// Capability sub-trait - emitted by #[capability] into:
//  - real trait + scheme blanket
//  - sealed_caps::BearerSealed blanket
//  - phantom companion + phantom blanket
#[nebula_credential_macros::capability(scheme_bound = AcceptsBearer, sealed = BearerSealed)]
pub trait MyServiceBearer: MyService {}

// State carrier (must impl ZeroizeOnDrop per Stage 2).
#[derive(Clone, Serialize, Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
struct WrongState {
    user: String,
    pass: String,
}

impl nebula_credential::CredentialState for WrongState {
    const KIND: &'static str = "wrong_state";
    const VERSION: u32 = 1;
}

struct WrongCredential;

impl Credential for WrongCredential {
    type Input = FieldValues;
    type Scheme = BasicScheme;
    type State = WrongState;

    const KEY: &'static str = "wrong";

    fn metadata() -> CredentialMetadata {
        unimplemented!()
    }

    fn project(state: &WrongState) -> BasicScheme {
        BasicScheme {
            user: state.user.clone(),
            pass: state.pass.clone(),
        }
    }

    fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> impl Future<Output = Result<ResolveResult<WrongState, ()>, CredentialError>> + Send {
        async { unimplemented!() }
    }
}

impl MyService for WrongCredential {}

fn _wire(_c: &dyn MyServiceBearerPhantom) {}

fn main() {
    let cred = WrongCredential;
    // E0277 - BasicScheme: AcceptsBearer not satisfied
    //   -> WrongCredential: MyServiceBearer not satisfied
    //   -> WrongCredential: MyServiceBearerPhantom not satisfied.
    _wire(&cred);
}
