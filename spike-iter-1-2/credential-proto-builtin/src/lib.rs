//! Spike builtin crate — ADR-0035 canonical form + Bitbucket triad validation.
//!
//! Depends on `credential-proto` (contract). Validates Q7: the dependency
//! direction `builtin -> contract` (and never the reverse) compiles cleanly.
//!
//! Iter-2: migrated from iter-1 plain `Phantom: Send + Sync + 'static` to
//! the ADR-0035 §1 canonical two-trait form: `Phantom: sealed::Sealed +
//! Send + Sync + 'static` with a crate-private `mod sealed` at crate root.
//! Bounds are the ADR-0035 §5 starting form — iter-2 §6 below records the
//! empirical minimum-bounds verification outcome.

#![allow(dead_code)]
#![forbid(unsafe_code)]

use credential_proto::{
    AcceptsBasic, AcceptsBearer, AuthScheme, Credential, CredentialRef, CredentialRegistry,
    CredentialState, NoPendingState,
};

// ─────────────────────────────────────────────────────────────────────────────
// §1. Crate-private sealed module (ADR-0035 §3).
//
// Canonical form: `mod sealed { pub trait Sealed {} }` at crate root.
//   - Outer `mod` has no `pub` prefix → `credential_proto_builtin::sealed::*`
//     is unreachable from outside this crate.
//   - Inner `pub trait Sealed` is visible-within-crate, so `pub trait
//     FooPhantom: sealed::Sealed` does not trip `private_in_public` lint.
//
// Per ADR-0035 §4.1: this module is declared MANUALLY, not macro-emitted.
// Proc-macros cannot share state across invocations, so a hypothetical
// `#[capability]` macro emitting `mod sealed` once-per-crate is not
// implementable. One-line convention; no macro magic.
//
// Plugin crates follow the same convention — each declares its own local
// `mod sealed`. No cross-crate sharing. Each crate protects only its own
// phantom traits (ADR-0035 §3 plugin paragraph).
// ─────────────────────────────────────────────────────────────────────────────

mod sealed {
    pub trait Sealed {}
}

// ─────────────────────────────────────────────────────────────────────────────
// Schemes — concrete AuthScheme types, capability traits attached.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct BearerScheme {
    pub token: String,
}
impl AuthScheme for BearerScheme {}
impl AcceptsBearer for BearerScheme {}

#[derive(Clone, Debug)]
pub struct BasicScheme {
    pub user: String,
    pub pass: String,
}
impl AuthScheme for BasicScheme {}
impl AcceptsBasic for BasicScheme {}

// ─────────────────────────────────────────────────────────────────────────────
// State types. `CredentialState` bound is Send + Sync + Clone + 'static.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct OAuth2State {
    pub access_token: String,
    pub refresh_token: String,
}
impl CredentialState for OAuth2State {}

#[derive(Clone)]
pub struct PatState {
    pub token: String,
}
impl CredentialState for PatState {}

#[derive(Clone)]
pub struct AppPasswordState {
    pub user: String,
    pub pass: String,
}
impl CredentialState for AppPasswordState {}

// ─────────────────────────────────────────────────────────────────────────────
// Service marker — Strategy §3.2. Pure marker. Sealing of the service trait
// itself is Q5 (out of spike scope). The phantom sealing below (ADR-0035) is
// orthogonal — it seals the phantom shim, not the service trait.
// ─────────────────────────────────────────────────────────────────────────────

pub trait BitbucketCredential: Credential {}

// ─────────────────────────────────────────────────────────────────────────────
// §2. ADR-0035 canonical two-trait + sealed chain.
//
// Layer 1: "real" capability trait (supertrait-chained to service marker).
//   Used for blanket-impl eligibility. NOT usable in `dyn` positions.
// Layer 2: sealed blanket — only BitbucketBearer-satisfying types gain
//   sealed::Sealed membership. External crates cannot impl sealed::Sealed
//   (module-private) so cannot forge phantom membership.
// Layer 3: phantom capability trait — dyn-safe (no Credential supertrait).
//   Well-formed as `dyn BitbucketBearerPhantom` type.
// ─────────────────────────────────────────────────────────────────────────────

// Layer 1 — the real trait (same as iter-1).
pub trait BitbucketBearer: BitbucketCredential {}

impl<T> BitbucketBearer for T
where
    T: BitbucketCredential,
    T::Scheme: AcceptsBearer,
{
}

pub trait BitbucketBasic: BitbucketCredential {}

impl<T> BitbucketBasic for T
where
    T: BitbucketCredential,
    T::Scheme: AcceptsBasic,
{
}

// Layer 2 — sealed blanket, one per capability trait.
// NOTE: two blanket impls of sealed::Sealed for T would collide if we
// naively wrote `impl<T: BitbucketBearer> sealed::Sealed for T {}` AND
// `impl<T: BitbucketBasic> sealed::Sealed for T {}` — the compiler's
// coherence check sees them as "overlap possible" (a single concrete T
// might satisfy both). Coherence rejects even when no actual type does
// both, because orphan rules don't know that.
//
// RESOLUTION (ADR-0035 §3 implicitly assumes): each phantom has its own
// inner Sealed trait, so the seals don't collide. Let's scope the Sealed
// per-phantom:
//
//   mod sealed {
//       pub trait BearerSealed {}
//       pub trait BasicSealed {}
//   }
//
// This keeps ADR-0035 §3 "mod sealed { pub trait Sealed {} }" in spirit
// (single private module) but with per-capability named Sealed traits to
// dodge the coherence collision. ADR-0035 does NOT address this nuance
// directly — flag in NOTES.md iter-2 §2 as a refinement.

mod sealed_caps {
    // Per-capability Sealed traits. Single `pub trait Sealed` in `mod sealed`
    // would cause blanket-impl coherence collision when two capability sub-
    // traits both want `impl<T: CapX> Sealed for T`. Per-capability Sealed
    // sidesteps that — see NOTES.md §2 for the coherence discussion.
    pub trait BearerSealed {}
    pub trait BasicSealed {}
    pub trait TlsIdentitySealed {}
    pub trait GenericBearerSealed {}
}

impl<T: BitbucketBearer> sealed_caps::BearerSealed for T {}
impl<T: BitbucketBasic> sealed_caps::BasicSealed for T {}

// Layer 3 — phantom capability traits (what action code names in `dyn`).
// Bounds per ADR-0035 §1 starting form: `sealed + Send + Sync + 'static`.
// §6 below records the minimum-bounds verification outcome.

// §6 BOUNDS VERIFICATION (ADR-0035 §5):
//   - `'static` → droppable. `CredentialRef<dyn Phantom>` uses PhantomData
//     projection; default-object-lifetime at the use site (struct field,
//     no lifetime param) already defaults to `'static`. Tests pass with
//     `'static` dropped from every phantom trait.
//   - `Send + Sync` → TECHNICALLY droppable in this spike (all tests pass
//     because CredentialRef<C>'s PhantomData<fn() -> C> auto-impls
//     Send+Sync regardless of C). BUT kept as a STABILITY PROMISE for
//     consumers that might use `&dyn Phantom` / `Box<dyn Phantom>` in
//     positions outside `CredentialRef` (e.g. transiently held in a Vec
//     during registration). Dropping Send+Sync here would forward-leak
//     a constraint to every consumer; the tiny cost of keeping the
//     explicit bound now beats breaking changes later.
//
// Decision: drop 'static, keep Send + Sync.

pub trait BitbucketBearerPhantom: sealed_caps::BearerSealed + Send + Sync {}
impl<T: BitbucketBearer> BitbucketBearerPhantom for T {}

pub trait BitbucketBasicPhantom: sealed_caps::BasicSealed + Send + Sync {}
impl<T: BitbucketBasic> BitbucketBasicPhantom for T {}

// ─────────────────────────────────────────────────────────────────────────────
// The Bitbucket triad — three concrete credential types.
// ─────────────────────────────────────────────────────────────────────────────

pub struct BitbucketOAuth2;

impl Credential for BitbucketOAuth2 {
    type Input = ();
    type Scheme = BearerScheme;
    type State = OAuth2State;
    type Pending = NoPendingState;
    const KEY: &'static str = "bitbucket_oauth2";
    const INTERACTIVE: bool = true;
    const REFRESHABLE: bool = true;

    fn project(state: &Self::State) -> Self::Scheme {
        BearerScheme { token: state.access_token.clone() }
    }
}

impl BitbucketCredential for BitbucketOAuth2 {}

pub struct BitbucketPat;

impl Credential for BitbucketPat {
    type Input = ();
    type Scheme = BearerScheme;
    type State = PatState;
    type Pending = NoPendingState;
    const KEY: &'static str = "bitbucket_pat";

    fn project(state: &Self::State) -> Self::Scheme {
        BearerScheme { token: state.token.clone() }
    }
}

impl BitbucketCredential for BitbucketPat {}

pub struct BitbucketAppPassword;

impl Credential for BitbucketAppPassword {
    type Input = ();
    type Scheme = BasicScheme; // §3.3 negative-case fulcrum.
    type State = AppPasswordState;
    type Pending = NoPendingState;
    const KEY: &'static str = "bitbucket_app_password";

    fn project(state: &Self::State) -> Self::Scheme {
        BasicScheme { user: state.user.clone(), pass: state.pass.clone() }
    }
}

impl BitbucketCredential for BitbucketAppPassword {}

// ─────────────────────────────────────────────────────────────────────────────
// §3.3 POSITIVE-CASE COMPILE PROOFS — ADR-0035 chain.
// Resolution walk for each type must succeed through all layers.
// ─────────────────────────────────────────────────────────────────────────────

const fn _assert_bearer<T: BitbucketBearer>() {}
const fn _assert_basic<T: BitbucketBasic>() {}
const fn _assert_bearer_phantom<T: ?Sized + BitbucketBearerPhantom>() {}
const fn _assert_basic_phantom<T: ?Sized + BitbucketBasicPhantom>() {}

const _: () = {
    // Layer 1 — real trait.
    _assert_bearer::<BitbucketOAuth2>();
    _assert_bearer::<BitbucketPat>();
    _assert_basic::<BitbucketAppPassword>();

    // Layer 3 — phantom (laundered through sealed layer 2).
    _assert_bearer_phantom::<BitbucketOAuth2>();
    _assert_bearer_phantom::<BitbucketPat>();
    _assert_basic_phantom::<BitbucketAppPassword>();
};

// ─────────────────────────────────────────────────────────────────────────────
// Hand-expanded `#[action]` shape for Pattern 2 consumers.
// `CredentialRef<dyn BitbucketBearerPhantom>` is well-formed per ADR-0035.
// ─────────────────────────────────────────────────────────────────────────────

pub struct GenericBitbucketAction {
    pub bb: CredentialRef<dyn BitbucketBearerPhantom>,
}

const _: fn() = || {
    fn _ensure_send_sync<T: Send + Sync>() {}
    _ensure_send_sync::<GenericBitbucketAction>();
};

// ═════════════════════════════════════════════════════════════════════════════
// Q4 DualAuth — mTLS + Bearer
//
// Models a Resource that consumes TWO credential slots: one TLS identity
// (client cert + key) for mTLS handshake, one Bearer for HTTP Authorization.
// Tests that:
//   1. A resource can declare a tuple of AcceptedAuth bounds.
//   2. An action holding both CredentialRef fields compiles.
//   3. Per Q3a §3.5 mechanism (i): an action missing one of the two
//      credential fields fails to satisfy the resource's requirement.
// ═════════════════════════════════════════════════════════════════════════════

// ─── TlsIdentity scheme ──────────────────────────────────────────────────────

use credential_proto::AcceptsTlsIdentity;

#[derive(Clone, Debug)]
pub struct TlsIdentityScheme {
    pub cert_pem: String,
    pub key_pem: String,
}
impl AuthScheme for TlsIdentityScheme {}
impl AcceptsTlsIdentity for TlsIdentityScheme {}

#[derive(Clone)]
pub struct MtlsIdentityState {
    pub cert_pem: String,
    pub key_pem: String,
}
impl CredentialState for MtlsIdentityState {}

pub struct MtlsClientCredential;

impl Credential for MtlsClientCredential {
    type Input = ();
    type Scheme = TlsIdentityScheme;
    type State = MtlsIdentityState;
    type Pending = NoPendingState;
    const KEY: &'static str = "mtls_client";

    fn project(state: &Self::State) -> Self::Scheme {
        TlsIdentityScheme {
            cert_pem: state.cert_pem.clone(),
            key_pem: state.key_pem.clone(),
        }
    }
}

// ─── Capability sub-trait + phantom for TlsIdentity ──────────────────────────

// TLS identity is NOT service-specific (unlike Bitbucket's Bearer). Model
// as a Pattern-3 (capability-only) trait: no service supertrait required.
// Directly bounded on Credential::Scheme: AcceptsTlsIdentity.

pub trait SupportsTlsIdentity: Credential {}

impl<T> SupportsTlsIdentity for T
where
    T: Credential,
    T::Scheme: AcceptsTlsIdentity,
{
}

impl<T: SupportsTlsIdentity> sealed_caps::TlsIdentitySealed for T {}

pub trait TlsIdentityPhantom:
    sealed_caps::TlsIdentitySealed 
{
}
impl<T: SupportsTlsIdentity> TlsIdentityPhantom for T {}

// Same-named Pattern-3 cap for Bearer (service-agnostic generic bearer).
pub trait SupportsBearer: Credential {}

impl<T> SupportsBearer for T
where
    T: Credential,
    T::Scheme: AcceptsBearer,
{
}

impl<T: SupportsBearer> sealed_caps::GenericBearerSealed for T {}

pub trait BearerPhantom: sealed_caps::GenericBearerSealed + Send + Sync {}
impl<T: SupportsBearer> BearerPhantom for T {}

// ─── Q3a §3.5 mechanism (i): trait-resolution cross-check ─────────────────────
//
// Resource declares an `AcceptedAuth` associated-type tuple. Action body
// is a generic fn bounded on `<Self as HasResource>::Resource::AcceptedAuth`
// — the macro-generated action glue translates the `CredentialRef<dyn Ph>`
// fields into a matching tuple of Scheme projections.
//
// For the spike: model Resource as a generic struct whose `AcceptedAuth`
// is a type-level tuple. The engine-side resolve fn is bounded on
// `C1: Credential<Scheme = TlsIdentityScheme>, C2: Credential<Scheme = BearerScheme>`.
// An action that only provides C2 (missing C1) fails to compile because
// the resolve fn's arity won't match.

pub trait Resource {
    type AcceptedAuth;
}

pub struct MtlsHttpResource;

impl Resource for MtlsHttpResource {
    // Dual-auth: tuple of two scheme types. In production the tuple
    // elements would be SchemeInjector dyn-refs; here concrete types suffice
    // for the compile-enforcement demo.
    type AcceptedAuth = (TlsIdentityScheme, BearerScheme);
}

// ─── Dual-auth action — uses two CredentialRef fields ─────────────────────────

pub struct MtlsHttpAction {
    pub tls: CredentialRef<dyn TlsIdentityPhantom>,
    pub bearer: CredentialRef<dyn BearerPhantom>,
}

const _: fn() = || {
    fn _ensure_send_sync<T: Send + Sync>() {}
    _ensure_send_sync::<MtlsHttpAction>();
};

// ─── Resolve pair — §3.5 mechanism (i) compile-enforced arity + scheme ───────
//
// Generic over (C1, C2). Where-clause pins Scheme of each to the resource's
// AcceptedAuth tuple elements. If an action declares only one CredentialRef
// field, callers cannot satisfy the arity.

pub fn resolve_mtls_pair<C1, C2>(
    reg: &CredentialRegistry,
    tls_key: &str,
    bearer_key: &str,
    tls_state: &C1::State,
    bearer_state: &C2::State,
) -> Option<(TlsIdentityScheme, BearerScheme)>
where
    C1: Credential<Scheme = TlsIdentityScheme>,
    C2: Credential<Scheme = BearerScheme>,
{
    let _c1 = reg.resolve_concrete::<C1>(tls_key)?;
    let _c2 = reg.resolve_concrete::<C2>(bearer_key)?;
    Some((C1::project(tls_state), C2::project(bearer_state)))
}

// Compile assertion: MtlsClientCredential satisfies TlsIdentityPhantom.
const _: () = {
    const fn _assert_tls<T: ?Sized + TlsIdentityPhantom>() {}
    const fn _assert_bearer<T: ?Sized + BearerPhantom>() {}
    _assert_tls::<MtlsClientCredential>();
    _assert_bearer::<BitbucketOAuth2>();
    _assert_bearer::<BitbucketPat>();
};
