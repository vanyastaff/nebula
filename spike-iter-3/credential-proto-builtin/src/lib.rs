//! Spike iter-3 — 3 concrete credentials + phantom-shim portfolio.
//!
//! Validates:
//! - (a) `Box<dyn Credential<Input = I, Scheme = S, State = St>>` constructs.
//! - (b) Phantom-shim erases `C::Scheme` cleanly for Pattern 2 consumers.
//! - (c) `dyn Refreshable<…>` — can it be used directly, or need parallel phantom?
//! - Composition with sub-trait split: `ApiKey` (no sub-trait), `OAuth2`
//!   (Refreshable + Revocable + Interactive), `SalesforceJwt` (Interactive +
//!   Refreshable) — structurally hypothetical but valid shape.

#![allow(dead_code)]
#![forbid(unsafe_code)]

use credential_proto::{
    AcceptsBasic, AcceptsBearer, AcceptsSigning, AcceptsTlsIdentity, AuthScheme, Credential,
    CredentialContext, CredentialMetadata, CredentialRef, CredentialState, Interactive,
    PendingState, PublicScheme, RefreshError, RefreshOutcome, RefreshPolicy, Refreshable,
    ResolveError, ResolveResult, RevokeError, Revocable, Sealed, SensitiveScheme, UserInput,
};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

// ============================================================================
// §1. Schemes — each declares SensitiveScheme or PublicScheme per §15.5.
// ============================================================================

/// Bearer token scheme (sensitive).
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct BearerScheme {
    pub token: String,
}
impl AuthScheme for BearerScheme {}
impl SensitiveScheme for BearerScheme {}
impl AcceptsBearer for BearerScheme {}

/// Basic auth scheme (sensitive).
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct BasicScheme {
    pub user: String,
    pub pass: String,
}
impl AuthScheme for BasicScheme {}
impl SensitiveScheme for BasicScheme {}
impl AcceptsBasic for BasicScheme {}

/// Signing-key scheme for JWT assertions (sensitive).
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SigningKeyScheme {
    pub private_key_pem: String,
    pub kid: String,
}
impl AuthScheme for SigningKeyScheme {}
impl SensitiveScheme for SigningKeyScheme {}
impl AcceptsSigning for SigningKeyScheme {}

/// mTLS identity scheme (sensitive).
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct TlsIdentityScheme {
    pub cert_pem: String,
    pub key_pem: String,
}
impl AuthScheme for TlsIdentityScheme {}
impl SensitiveScheme for TlsIdentityScheme {}
impl AcceptsTlsIdentity for TlsIdentityScheme {}

/// Public scheme — e.g. instance binding (no secret). No zeroize needed.
#[derive(Clone)]
pub struct InstanceBindingScheme {
    pub provider: String,
    pub role: String,
}
impl AuthScheme for InstanceBindingScheme {}
impl PublicScheme for InstanceBindingScheme {}

// ============================================================================
// §2. Sealed module — per-capability inner seals (ADR-0035 §3 amendment).
//
// Each phantom chain has its own `XSealed` inner trait. External crates
// cannot import `credential_proto_builtin::sealed_caps::*` (module is
// crate-private) — they cannot forge phantom membership.
// ============================================================================

mod sealed_caps {
    pub trait BearerSealed {}
    pub trait BasicSealed {}
    pub trait SigningSealed {}
    pub trait TlsIdentitySealed {}
    pub trait RefreshableSealed {}
    pub trait InteractiveSealed {}
}

// ============================================================================
// §3. State types for the 3 credentials.
// ============================================================================

#[derive(Serialize, Deserialize, Clone, Zeroize, ZeroizeOnDrop)]
pub struct ApiKeyState {
    pub token: String,
}
impl CredentialState for ApiKeyState {}

#[derive(Serialize, Deserialize, Clone, Zeroize, ZeroizeOnDrop)]
pub struct OAuth2State {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_epoch: u64,
}
impl CredentialState for OAuth2State {}

#[derive(Serialize, Deserialize, Clone, Zeroize, ZeroizeOnDrop)]
pub struct OAuth2PendingState {
    pub pkce_verifier: String,
    pub state_nonce: String,
}
impl PendingState for OAuth2PendingState {}

#[derive(Serialize, Deserialize, Clone, Zeroize, ZeroizeOnDrop)]
pub struct SalesforceJwtState {
    pub signing_key_pem: String,
    pub kid: String,
    pub access_token: String,
    pub expires_at_epoch: u64,
}
impl CredentialState for SalesforceJwtState {}

#[derive(Serialize, Deserialize, Clone, Zeroize, ZeroizeOnDrop)]
pub struct SalesforceJwtPending {
    pub auth_code: String,
}
impl PendingState for SalesforceJwtPending {}

// ============================================================================
// §4. The 3 credential types.
// ============================================================================

/// Static API key credential — no sub-trait capability (baseline).
pub struct ApiKeyCredential;

impl Sealed for ApiKeyCredential {}

impl Credential for ApiKeyCredential {
    type Input = ();
    type Scheme = BearerScheme;
    type State = ApiKeyState;

    const KEY: &'static str = "api_key";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata { key: Self::KEY, crate_name: "credential-proto-builtin" }
    }

    fn project(state: &Self::State) -> Self::Scheme {
        BearerScheme { token: state.token.clone() }
    }

    fn resolve(
        _ctx: &CredentialContext<'_>,
        _input: &Self::Input,
    ) -> Result<ResolveResult<Self::State, ()>, ResolveError> {
        Err(ResolveError("spike stub".into()))
    }
}

// No Refreshable / Revocable / Interactive impl — static credential.

/// OAuth2 credential — Refreshable + Revocable + Interactive.
pub struct OAuth2Credential;

impl Sealed for OAuth2Credential {}

impl Credential for OAuth2Credential {
    type Input = ();
    type Scheme = BearerScheme;
    type State = OAuth2State;

    const KEY: &'static str = "oauth2";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata { key: Self::KEY, crate_name: "credential-proto-builtin" }
    }

    fn project(state: &Self::State) -> Self::Scheme {
        BearerScheme { token: state.access_token.clone() }
    }

    fn resolve(
        _ctx: &CredentialContext<'_>,
        _input: &Self::Input,
    ) -> Result<ResolveResult<Self::State, ()>, ResolveError> {
        Err(ResolveError("spike stub".into()))
    }
}

impl Refreshable for OAuth2Credential {
    const REFRESH_POLICY: RefreshPolicy = RefreshPolicy::DEFAULT;

    fn refresh(
        _state: &mut Self::State,
        _ctx: &CredentialContext<'_>,
    ) -> Result<RefreshOutcome, RefreshError> {
        Ok(RefreshOutcome::Refreshed)
    }
}

impl Revocable for OAuth2Credential {
    fn revoke(
        _state: &mut Self::State,
        _ctx: &CredentialContext<'_>,
    ) -> Result<(), RevokeError> {
        Ok(())
    }
}

impl Interactive for OAuth2Credential {
    type Pending = OAuth2PendingState;

    fn continue_resolve(
        _pending: &Self::Pending,
        _input: &UserInput,
        _ctx: &CredentialContext<'_>,
    ) -> Result<ResolveResult<Self::State, Self::Pending>, ResolveError> {
        Err(ResolveError("spike stub".into()))
    }
}

/// Salesforce JWT bearer credential — Interactive + Refreshable (hypothetical
/// shape: user selects key, then refreshes access_token from assertion).
pub struct SalesforceJwtCredential;

impl Sealed for SalesforceJwtCredential {}

impl Credential for SalesforceJwtCredential {
    type Input = ();
    type Scheme = SigningKeyScheme;
    type State = SalesforceJwtState;

    const KEY: &'static str = "salesforce_jwt";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata { key: Self::KEY, crate_name: "credential-proto-builtin" }
    }

    fn project(state: &Self::State) -> Self::Scheme {
        SigningKeyScheme {
            private_key_pem: state.signing_key_pem.clone(),
            kid: state.kid.clone(),
        }
    }

    fn resolve(
        _ctx: &CredentialContext<'_>,
        _input: &Self::Input,
    ) -> Result<ResolveResult<Self::State, ()>, ResolveError> {
        Err(ResolveError("spike stub".into()))
    }
}

impl Interactive for SalesforceJwtCredential {
    type Pending = SalesforceJwtPending;

    fn continue_resolve(
        _pending: &Self::Pending,
        _input: &UserInput,
        _ctx: &CredentialContext<'_>,
    ) -> Result<ResolveResult<Self::State, Self::Pending>, ResolveError> {
        Err(ResolveError("spike stub".into()))
    }
}

impl Refreshable for SalesforceJwtCredential {
    const REFRESH_POLICY: RefreshPolicy = RefreshPolicy::DEFAULT;

    fn refresh(
        _state: &mut Self::State,
        _ctx: &CredentialContext<'_>,
    ) -> Result<RefreshOutcome, RefreshError> {
        Ok(RefreshOutcome::Refreshed)
    }
}

// ============================================================================
// §5. Service marker — Bitbucket triad (ADR-0035 §1 worked example).
// ============================================================================

pub trait BitbucketCredential: Credential {}

// ============================================================================
// §6. Phantom-shim pattern for BEARER capability (service-bound Pattern 2).
// ADR-0035 §1 canonical form.
//
// Layer 1: "real" capability trait — `BitbucketBearer: BitbucketCredential`,
//          supertrait-chained. Blanket-impl eligibility gate. NOT usable in
//          `dyn` positions (inherits Credential's 3 unspecified assoc types).
// Layer 2: Sealed blanket — `impl<T: BitbucketBearer> sealed_caps::BearerSealed for T {}`.
//          External crates cannot impl `BearerSealed` (module-private).
// Layer 3: Phantom — `BitbucketBearerPhantom: sealed_caps::BearerSealed + Send + Sync`.
//          dyn-safe (no Credential supertrait → no unspecified assoc types).
// ============================================================================

pub trait BitbucketBearer: BitbucketCredential {}

impl<T> BitbucketBearer for T
where
    T: BitbucketCredential,
    T::Scheme: AcceptsBearer,
{
}

impl<T: BitbucketBearer> sealed_caps::BearerSealed for T {}

pub trait BitbucketBearerPhantom: sealed_caps::BearerSealed + Send + Sync {}
impl<T: BitbucketBearer> BitbucketBearerPhantom for T {}

// ============================================================================
// §7. Pattern 3 — service-agnostic Bearer phantom.
//
// Pattern 2 (§6) uses `sealed_caps::BearerSealed` scoped to BitbucketBearer.
// Pattern 3 needs its own sealed trait because a second blanket
// `impl<T: SupportsBearer> BearerSealed for T {}` would collide with §6's
// `impl<T: BitbucketBearer> BearerSealed for T {}` under orphan coherence
// (Rust can't reason that bounds are disjoint, treats the impls as
// overlapping even though no concrete type satisfies both simultaneously).
//
// Resolution per ADR-0035 §3 amendment: per-capability inner seals. Pattern
// 3 uses its own seal, named after its capability surface ("AnyBearer" — the
// capability-agnostic variant).
// ============================================================================

pub trait SupportsGenericBearer: Credential {}

impl<T> SupportsGenericBearer for T
where
    T: Credential,
    T::Scheme: AcceptsBearer,
{
}

// Separate sealed trait for the Pattern 3 chain — avoids coherence overlap
// with §6's BearerSealed blanket. Per ADR-0035 §3 amendment.
mod sealed_pattern3 {
    pub trait AnyBearerSealed {}
}

impl<T: SupportsGenericBearer> sealed_pattern3::AnyBearerSealed for T {}

pub trait AnyBearerPhantom: sealed_pattern3::AnyBearerSealed + Send + Sync {}
impl<T: SupportsGenericBearer> AnyBearerPhantom for T {}

// ============================================================================
// §9. Bitbucket-credential impls — only OAuth2 here, not API key.
// ApiKeyCredential satisfies `Credential` + `Scheme = BearerScheme` but does
// NOT declare `BitbucketCredential` — so it won't satisfy `BitbucketBearerPhantom`.
// It WILL satisfy `AnyBearerPhantom` (Pattern 3).
// ============================================================================

impl BitbucketCredential for OAuth2Credential {}

// ApiKeyCredential does NOT implement BitbucketCredential — by design.
// This lets us verify Pattern 2 rejection (service-level) while Pattern 3
// (capability-only) accepts.

// ============================================================================
// §10. Refreshable phantom — question (c): does `dyn Refreshable` work
// directly, or do we need a parallel phantom-shim?
//
// Attempt 1: `let _r: Box<dyn Refreshable> = …`. `Refreshable: Credential`,
// and Credential has 3 assoc types (Input/Scheme/State). Those propagate
// via the supertrait chain. Expected diagnostic: E0191 — the value of the
// associated types must be specified.
//
// Attempt 2: Parallel phantom `RefreshablePhantom: sealed::RefreshableSealed
// + Send + Sync`. No Credential supertrait → 0 unspecified assoc types →
// well-formed in dyn.
// ============================================================================

impl<T: Refreshable> sealed_caps::RefreshableSealed for T {}

pub trait RefreshablePhantom: sealed_caps::RefreshableSealed + Send + Sync {}
impl<T: Refreshable> RefreshablePhantom for T {}

// ============================================================================
// §11. Interactive phantom — same pattern, for engine's interactive dispatch.
// ============================================================================

impl<T: Interactive> sealed_caps::InteractiveSealed for T {}

pub trait InteractivePhantom: sealed_caps::InteractiveSealed + Send + Sync {}
impl<T: Interactive> InteractivePhantom for T {}

// ============================================================================
// §12. Pattern 2 action — consumer declaring `CredentialRef<dyn BitbucketBearerPhantom>`.
// At compile time, action instantiation with `ApiKeyCredential` rejected
// (not a BitbucketCredential). `OAuth2Credential` accepted.
// ============================================================================

pub struct BitbucketFetchAction {
    pub cred: CredentialRef<dyn BitbucketBearerPhantom>,
}

// Pattern 3 action — accepts ANY credential with Bearer capability.
pub struct GenericBearerAction {
    pub cred: CredentialRef<dyn AnyBearerPhantom>,
}

// ============================================================================
// §13. Compile-time assertions — positive cases.
// ============================================================================

const _: () = {
    // Pattern 2: BitbucketBearerPhantom accepts OAuth2Credential.
    const fn _assert_bb<T: ?Sized + BitbucketBearerPhantom>() {}
    _assert_bb::<OAuth2Credential>();

    // Pattern 3: AnyBearerPhantom accepts both API key AND OAuth2 (both
    // have Scheme: AcceptsBearer). Correctly REJECTS SalesforceJwtCredential
    // (SigningKeyScheme implements AcceptsSigning, not AcceptsBearer) —
    // attempting `_assert_generic::<SalesforceJwtCredential>()` would fail
    // with E0277. Not asserted here because const block is compile-proof-positive only.
    const fn _assert_generic<T: ?Sized + AnyBearerPhantom>() {}
    _assert_generic::<ApiKeyCredential>();
    _assert_generic::<OAuth2Credential>();

    // RefreshablePhantom accepts OAuth2 + SalesforceJwt but not ApiKey.
    const fn _assert_refresh<T: ?Sized + RefreshablePhantom>() {}
    _assert_refresh::<OAuth2Credential>();
    _assert_refresh::<SalesforceJwtCredential>();

    // InteractivePhantom accepts OAuth2 + SalesforceJwt but not ApiKey.
    const fn _assert_interactive<T: ?Sized + InteractivePhantom>() {}
    _assert_interactive::<OAuth2Credential>();
    _assert_interactive::<SalesforceJwtCredential>();
};
