//! Iteration-1 pause artifact — converged trait shape so far.
//!
//! Distilled snapshot of the shape proven to compile in
//! `spike/credential-proto/` and `spike/credential-proto-builtin/`.
//! Single-file readable summary for orchestrator review.
//!
//! What's covered in iteration 1:
//!   - Q1 (§3.3 blanket sub-trait pattern on Bitbucket triad):
//!       PASSES — both positive (OAuth2/PAT) and negative (AppPassword)
//!       cases compile-enforce as Strategy §3.3 prescribes.
//!   - Q3 H1 (PhantomData + TypeId registry):
//!       Compiles + 5 runtime resolve tests pass. NO PERF BENCH yet.
//!   - Q7 (two-crate split):
//!       Compiles. builtin -> contract direction; reverse impossible
//!       by construction (contract has no concrete types).
//!
//! What's NOT yet done (deferred to iteration 2 — explicit blocker list):
//!   - Q2 (`#[action]` 0/2+ slot ambiguity compile-error proofs).
//!   - Q3 H2 + H3 (binding table / typed accessors).
//!   - Q3 perf benches (baseline + h1 + h2 + h3 with Criterion).
//!   - Q3a §3.5 cap↔resource cross-check (trait-resolution OR registry).
//!   - Q4 DualAuth resource (mTLS + Bearer).
//!   - 4 of 7 credential types (Slack, Anthropic, AwsSigV4+Sts, Postgres,
//!     Mtls, SalesforceJwt).
//!   - 2 of 3 resources (Postgres, MtlsHttp).
//!   - 2 of 3 actions (GenericSlackAction, GenericHttpBearerAction).
//!   - tests/e2e.rs integration.
//!
//! Iteration-1 ADJUSTMENTS to the starting shapes (rationale in NOTES.md §3):
//!   - H1 PhantomData type signature kept (`PhantomData<fn() -> C>`),
//!     but the `dyn BoundedTrait` projection layer needed a phantom-trait
//!     workaround because `dyn BitbucketBearer` is not a well-formed type
//!     (Credential has 4 assoc types, all required to materialize a dyn).
//!   - Workaround: `BitbucketBearerPhantom: 'static + Send + Sync` with
//!     blanket `impl<T: BitbucketBearer> BitbucketBearerPhantom for T`.
//!     §3.3 semantic guarantee SURVIVES the workaround (verified).
//!
//! ─────────────────────────────────────────────────────────────────────
//! CONTRACT CRATE — credential-proto
//! ─────────────────────────────────────────────────────────────────────

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::marker::PhantomData;

// ── AuthScheme + capability markers ──────────────────────────────────

pub trait AuthScheme: Send + Sync + Clone + 'static {}
pub trait AcceptsBearer: AuthScheme {}
pub trait AcceptsBasic: AuthScheme {}
pub trait AcceptsSigning: AuthScheme {}
pub trait AcceptsTlsIdentity: AuthScheme {}

// ── State / pending opaque traits (shape-faithful stand-ins) ─────────

pub trait CredentialState: Send + Sync + Clone + 'static {}
pub trait PendingState: Send + Sync + 'static {}
pub struct NoPendingState;
impl PendingState for NoPendingState {}

pub trait HasInputSchema: Send + Sync + 'static {}
impl HasInputSchema for () {}

// ── Credential trait — 4 assoc types, all methods `where Self: Sized` ─
//
// `where Self: Sized` is what allows `dyn Credential<Input=…, …, …, …>`
// to be a (technically) well-formed type at all (vtable holds zero
// methods). But it does NOT make `dyn Credential` (without all four
// assoc types specified) a well-formed type — that's the §3.2 dyn
// caveat the spike confirmed.

pub trait Credential: Send + Sync + 'static {
    type Input: HasInputSchema;
    type Scheme: AuthScheme;
    type State: CredentialState;
    type Pending: PendingState;
    const KEY: &'static str;
    const INTERACTIVE: bool = false;
    const REFRESHABLE: bool = false;
    const REVOCABLE: bool = false;
    const TESTABLE: bool = false;

    fn project(state: &Self::State) -> Self::Scheme
    where
        Self: Sized;

    fn resolve_stub(_input: &Self::Input) -> Result<Self::State, &'static str>
    where
        Self: Sized,
    {
        Err("not implemented in spike")
    }
}

// ── AnyCredential — narrower object-safe trait (the actual runtime path) ─

pub trait AnyCredential: Any + Send + Sync + 'static {
    fn credential_key(&self) -> &'static str;
    fn type_id_marker(&self) -> TypeId;
    fn as_any(&self) -> &dyn Any;
}

impl<C: Credential> AnyCredential for C {
    fn credential_key(&self) -> &'static str {
        C::KEY
    }
    fn type_id_marker(&self) -> TypeId {
        TypeId::of::<C>()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ── CredentialKey — opaque handle ─────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CredentialKey(pub String);

// ── CredentialRef<C> — H1 (PhantomData + TypeId registry path) ────────
// `C: ?Sized` so `CredentialRef<dyn BitbucketBearerPhantom>` is legal.

pub struct CredentialRef<C: ?Sized> {
    pub key: CredentialKey,
    _t: PhantomData<fn() -> C>,
}

// ── CredentialRegistry — HashMap<(Key, TypeId), Box<dyn AnyCredential>> ─

pub struct CredentialRegistry {
    entries: HashMap<(CredentialKey, TypeId), Box<dyn AnyCredential>>,
}

// (impl methods elided in this snapshot — see crate source.)

// ─────────────────────────────────────────────────────────────────────
// BUILTIN CRATE — credential-proto-builtin
// (depends on credential-proto)
// ─────────────────────────────────────────────────────────────────────

// ── Schemes + capability impls ────────────────────────────────────────

pub struct BearerScheme { pub token: String }
impl AuthScheme for BearerScheme {}
impl AcceptsBearer for BearerScheme {}

pub struct BasicScheme { pub user: String, pub pass: String }
impl AuthScheme for BasicScheme {}
impl AcceptsBasic for BasicScheme {}

// ── Service marker (Strategy §3.2 pure marker) ───────────────────────

pub trait BitbucketCredential: Credential {}

// ── Capability sub-trait (Strategy §3.3 blanket impl) ────────────────
//
// THIS IS THE LOAD-BEARING SHAPE. AppPassword must NOT satisfy this.

pub trait BitbucketBearer: BitbucketCredential {}

impl<T> BitbucketBearer for T
where
    T: BitbucketCredential,
    T::Scheme: AcceptsBearer,
{}

// ── Phantom projection trait (iteration-1 adjustment) ────────────────
//
// `CredentialRef<dyn BitbucketBearer>` is NOT a well-formed type
// (Credential's 4 assoc types must be specified to materialize a dyn).
// Workaround: a phantom trait with no Credential supertrait, blanket-
// impl-tied to BitbucketBearer. §3.3 semantic guarantee preserved
// (verified by compile-fail test `compile_fail_app_password_via_phantom`).

pub trait BitbucketBearerPhantom: Send + Sync + 'static {}
impl<T: BitbucketBearer> BitbucketBearerPhantom for T {}

// ── The Bitbucket triad ──────────────────────────────────────────────

pub struct BitbucketOAuth2;
// impl Credential for BitbucketOAuth2 { Scheme=BearerScheme, … }
// impl BitbucketCredential for BitbucketOAuth2 {}

pub struct BitbucketPat;
// impl Credential for BitbucketPat   { Scheme=BearerScheme, … }
// impl BitbucketCredential for BitbucketPat {}

pub struct BitbucketAppPassword;
// impl Credential for BitbucketAppPassword { Scheme=BasicScheme, … }
// impl BitbucketCredential for BitbucketAppPassword {}

// ── Hand-expanded action shape (Pattern 2 consumer) ──────────────────

pub struct GenericBitbucketAction {
    pub bb: CredentialRef<dyn BitbucketBearerPhantom>,
}

// ── §3.3 compile assertions (in real source — not callable here) ─────
//
//   const _: () = {
//       _assert_bearer::<BitbucketOAuth2>();        // PASSES
//       _assert_bearer::<BitbucketPat>();           // PASSES
//       _assert_basic::<BitbucketAppPassword>();    // PASSES (Basic, not Bearer)
//   };
//
// Negative case (in compile_fail example, MUST FAIL TO COMPILE):
//
//   const _: () = {
//       _assert_bearer::<BitbucketAppPassword>();   // ← E0277, BasicScheme: AcceptsBearer not satisfied
//   };
//
// Verified diagnostic chain:
//   BasicScheme: AcceptsBearer not satisfied
//     → required for BitbucketAppPassword to implement BitbucketBearer
//     → required for BitbucketAppPassword to implement BitbucketBearerPhantom

fn main() {} // placate `cargo build` if anyone tries to run this as a bin
