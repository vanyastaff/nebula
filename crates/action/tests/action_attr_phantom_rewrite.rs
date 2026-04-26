//! Integration test - `#[action] + #[derive(Action)]` end-to-end.
//!
//! Verifies the documented "apply attribute first, derive sees rewritten
//! struct" ordering behavior end-to-end (Tech Spec 2.7 / ADR-0035 §4.3).
//! Closes I3 from the Stage 4 review.
//!
//! Approach: a regular cargo integration test rather than a `trybuild`
//! probe. The test compiles a fixture struct that uses both macros
//! together; if it compiles and metadata round-trips, the macro pair
//! coexists correctly. trybuild was considered but adds no signal here -
//! the test must build the fixture as part of the action test crate
//! anyway, and a `pass` probe is just a slower indirection.
//!
//! The fixture is self-contained:
//!
//! - local `mod sealed_caps` per ADR-0035 §4.1
//! - local `AcceptsBearer` marker
//! - local service supertrait + `#[capability]` invocation
//! - stand-in `CredentialRef<C: ?Sized>` (the real type does not ship in Stage 4; the `#[action]`
//!   rewriter operates on syntax, not on the type's semantics, so a stub suffices to exercise the
//!   rewrite)
//! - a Pattern 2 struct that wires `#[action] #[derive(Action)]` over a `CredentialRef<dyn
//!   LocalServiceBearer>` field
//!
//! The compile-fail companion is already covered by the standalone
//! `compile_fail_pattern2_service_reject` probe - that probe exercises
//! the phantom chain on a wrong-scheme credential. This file's test
//! covers the orthogonal axis: "macro pair compiles together when the
//! credential is correct".

#![allow(
    dead_code,
    reason = "fixture types exist only to type-check the macro expansion"
)]

use std::marker::PhantomData;

use nebula_action::Action;
use nebula_credential::{
    AuthPattern, AuthScheme, Credential, CredentialContext, CredentialMetadata,
    error::CredentialError, resolve::ResolveResult,
};
use nebula_schema::FieldValues;
use serde::{Deserialize, Serialize};

// ADR-0035 §4.1 - the crate author declares the sealed module manually
// at "crate root". For this fixture the test file is the crate root.
mod sealed_caps {
    pub trait BearerSealed {}
}

/// Local marker - `Scheme: AcceptsBearer` is the bound the capability
/// requires. In production this comes from `nebula_credential`.
pub trait AcceptsBearer: AuthScheme {}

/// A scheme that satisfies AcceptsBearer.
#[derive(Clone, Serialize, Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
pub struct LocalBearerScheme {
    token: String,
}

impl AuthScheme for LocalBearerScheme {
    fn pattern() -> AuthPattern {
        AuthPattern::SecretToken
    }
}

impl AcceptsBearer for LocalBearerScheme {}

/// State carrier (Stage 2 ZeroizeOnDrop bound).
#[derive(Clone, Serialize, Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
pub struct LocalState {
    token: String,
}

impl nebula_credential::CredentialState for LocalState {
    const KIND: &'static str = "local_state";
    const VERSION: u32 = 1;
}

/// Local service supertrait - declares the credential family.
pub trait LocalService: Credential {}

/// Capability sub-trait declared via `#[capability]`. Emits the real
/// trait + scheme blanket + sealed blanket + phantom companion + phantom
/// blanket. With ADR-0035 §1 visibility-symmetry, `pub trait` here
/// produces `pub trait LocalServiceBearerPhantom`.
#[nebula_credential_macros::capability(scheme_bound = AcceptsBearer, sealed = BearerSealed)]
pub trait LocalServiceBearer: LocalService {}

/// A concrete credential that wires LocalBearerScheme - satisfies
/// `LocalServiceBearer` (and therefore `LocalServiceBearerPhantom`).
pub struct LocalCredential;

impl Credential for LocalCredential {
    type Input = FieldValues;
    type Scheme = LocalBearerScheme;
    type State = LocalState;

    const KEY: &'static str = "local";

    fn metadata() -> CredentialMetadata {
        unimplemented!("fixture - never invoked at runtime")
    }

    fn project(state: &LocalState) -> LocalBearerScheme {
        LocalBearerScheme {
            token: state.token.clone(),
        }
    }

    async fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<LocalState, ()>, CredentialError> {
        unimplemented!("fixture - never invoked at runtime")
    }
}

impl LocalService for LocalCredential {}

/// Stand-in `CredentialRef<C: ?Sized>` for the test - the production
/// type does not ship in Stage 4. The `#[action]` rewriter matches on
/// the path-tail identifier `CredentialRef`, so this local stub exercises
/// the rewrite end-to-end without depending on a future Stage's
/// definition.
pub struct CredentialRef<C: ?Sized>(PhantomData<C>);

impl<C: ?Sized> Default for CredentialRef<C> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

// --- The test struct -------------------------------------------------------
//
// `#[action]` is applied FIRST, then `#[derive(Action)]`. The rewriter
// turns `CredentialRef<dyn LocalServiceBearer>` into
// `CredentialRef<dyn LocalServiceBearerPhantom>` BEFORE the derive sees
// the struct. If the ordering were wrong (or the rewriter missed the
// field) the type would be `CredentialRef<dyn LocalServiceBearer>` -
// which fails to compile because `dyn LocalServiceBearer` triggers the
// E0191 "unspecified associated types" gate from the `Credential`
// supertrait closure (the whole reason ADR-0035 exists).

#[nebula_action_macros::action]
#[derive(Action)]
#[action(
    key = "local.bearer.fetch",
    name = "Fetch Local Bearer",
    description = "Test fixture exercising #[action] + #[derive(Action)] coexistence"
)]
pub struct LocalBearerAction {
    /// Pattern 2 field - typed against the capability trait. The
    /// `#[action]` macro rewrites `dyn LocalServiceBearer` to
    /// `dyn LocalServiceBearerPhantom` before the derive runs.
    pub bearer: CredentialRef<dyn LocalServiceBearer>,
}

// --- Tests -----------------------------------------------------------------

#[test]
fn action_attr_with_derive_pattern2_compiles_and_metadata_roundtrips() {
    // Constructing the struct exercises the full type-check chain.
    // If `dyn LocalServiceBearer` had reached the derive unrewritten,
    // this `Default::default()` call would not compile (E0191 on the
    // `dyn` projection). The fact that it compiles is the proof.
    let action = LocalBearerAction {
        bearer: CredentialRef::default(),
    };
    let meta = action.metadata();
    assert_eq!(meta.base.key.as_str(), "local.bearer.fetch");
    assert_eq!(meta.base.name, "Fetch Local Bearer");
}

#[test]
fn action_attr_with_derive_pattern2_dependencies_are_empty() {
    // Pattern 2 fields don't auto-register as #[action(credential = ...)]
    // dependencies (the type is `dyn`, not a concrete Credential). The
    // dependency list reflects the `#[action(...)]` attribute args, which
    // declared none. Verifies the derive's DeclaresDependencies impl
    // executed against the rewritten struct without rejecting it.
    use nebula_core::DeclaresDependencies;
    assert!(LocalBearerAction::dependencies().credentials().is_empty());
    assert!(LocalBearerAction::dependencies().resources().is_empty());
}

#[test]
fn local_credential_satisfies_phantom_via_blanket() {
    // Sanity check on the phantom chain - the local credential satisfies
    // `LocalServiceBearer` (its `Scheme = LocalBearerScheme: AcceptsBearer`)
    // therefore it also satisfies `LocalServiceBearerPhantom` via the
    // blanket impl. If the macro emitted the phantom with wrong
    // visibility or skipped a blanket, this would fail.
    fn accepts_phantom_dispatch(_c: &dyn LocalServiceBearerPhantom) {}
    accepts_phantom_dispatch(&LocalCredential);
}
