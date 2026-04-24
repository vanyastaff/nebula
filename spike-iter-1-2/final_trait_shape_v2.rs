//! Iteration-2 pause artifact — converged trait shape after ADR-0035
//! canonical form applied + indexed registry + Pattern 2 / Q4 / Q3a / Q2
//! all validated end-to-end.
//!
//! Supersedes `final_trait_shape_v1.rs`.
//!
//! What's covered in iteration 2:
//!   - Q1 §3.3 — PASSES; positive + negative cases compile-enforce.
//!   - Q2 `#[action]` macro ambiguity — PASSES; 0/2+-slot shorthand
//!     rejected, 1-slot works.
//!   - Q3 H1/H2/H3 — all three COMPILE; all three p95 << 1µs.
//!     Decision: H1 (with H3 inline form as macro sugar). H2 rejected
//!     on ergonomics (fn-pointer table solves a non-problem at our scale).
//!   - Q3a §3.5 mechanism (i) — PASSES. `C: Credential<Scheme = …>`
//!     where-clause cross-check compile-enforces resource ↔ credential
//!     scheme match. §3.5 mechanism (ii) not attempted — (i) suffices.
//!   - Q4 DualAuth — PASSES. MtlsHttpResource (TlsIdentity + Bearer)
//!     compiles; wrong-scheme AppPassword in bearer slot rejected.
//!   - Q7 two-crate split — PASSES.
//!   - ADR-0035 §1 canonical form — APPLIED. Two-trait + sealed chain,
//!     crate-private sealed module, external-forge compile-rejected with
//!     rustc's built-in "sealed trait" diagnostic hint.
//!   - ADR-0035 §5 minimum bounds — VERIFIED. `'static` droppable;
//!     `Send + Sync` kept as forward-compat stability promise.
//!   - Iter-2 refinement not in ADR-0035: PER-CAPABILITY inner Sealed
//!     traits (BearerSealed, BasicSealed, TlsIdentitySealed, GenericBearerSealed)
//!     to sidestep orphan-rule coherence collision between two blanket
//!     impls of a single Sealed trait for different capability supertraits.
//!     This is a real constraint the ADR-0035 §3 "mod sealed { pub trait
//!     Sealed {} }" form does not address — flagged for ADR-0035 addendum.
//!
//! What's NOT yet done (deferred to iter-3 if orchestrator approves;
//!  iter-2 covers ≥4 of 5 scope questions — prompt's DONE threshold):
//!   - 4 of 7 credential types still not modeled (Slack, Anthropic,
//!     AwsSigV4+Sts, Postgres, SalesforceJwt).
//!   - 2 of 3 actions (GenericSlackAction, GenericHttpBearerAction).
//!   - tests/e2e.rs unified integration (per-test files cover same ground).
//!   - §3.5 mechanism (ii) compile-time capability registry — (i) suffices;
//!     not required to land both mechanisms.
//!   - Sealed service trait (Q5) — explicitly OOS per prompt.
//!
//! ─────────────────────────────────────────────────────────────────────
//! CONTRACT CRATE — credential-proto (iter-2)
//! ─────────────────────────────────────────────────────────────────────

use std::any::{Any, TypeId};
use std::marker::PhantomData;
use std::sync::Arc;

// Perf-grade hasher — ahash with fixed seed in spike (production: runtime-rng).
type FastMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;

// ── AuthScheme + capability markers ──────────────────────────────────

pub trait AuthScheme: Send + Sync + Clone + 'static {}
pub trait AcceptsBearer: AuthScheme {}
pub trait AcceptsBasic: AuthScheme {}
pub trait AcceptsSigning: AuthScheme {}
pub trait AcceptsTlsIdentity: AuthScheme {}

// ── Credential trait — unchanged from iter-1 ──────────────────────────

pub trait CredentialState: Send + Sync + Clone + 'static {}
pub trait PendingState: Send + Sync + 'static {}
pub struct NoPendingState;
impl PendingState for NoPendingState {}

pub trait HasInputSchema: Send + Sync + 'static {}
impl HasInputSchema for () {}

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
}

// ── AnyCredential — narrower object-safe shadow ──────────────────────

pub trait AnyCredential: Any + Send + Sync + 'static {
    fn credential_key(&self) -> &'static str;
    fn type_id_marker(&self) -> TypeId;
    fn as_any(&self) -> &dyn Any;
}

impl<C: Credential> AnyCredential for C {
    fn credential_key(&self) -> &'static str { C::KEY }
    fn type_id_marker(&self) -> TypeId { TypeId::of::<C>() }
    fn as_any(&self) -> &dyn Any { self }
}

// ── CredentialKey — Arc<str> (iter-2: cheap clone, zero-alloc lookup) ─

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CredentialKey(pub Arc<str>);

impl CredentialKey {
    pub fn new(s: impl Into<Arc<str>>) -> Self { Self(s.into()) }
    pub fn as_str(&self) -> &str { &self.0 }
}

impl std::borrow::Borrow<str> for CredentialKey {
    fn borrow(&self) -> &str { &self.0 }
}

// ── CredentialRef<C> — phantom nominal bound, `?Sized` for dyn ────────

pub struct CredentialRef<C: ?Sized> {
    pub key: CredentialKey,
    _t: PhantomData<fn() -> C>,
}

// ── CredentialRegistry — iter-2 indexed, ahash-backed ─────────────────

pub struct CredentialRegistry {
    entries: FastMap<Arc<str>, Box<dyn AnyCredential>>,
}

impl CredentialRegistry {
    pub fn new() -> Self { /* impl in crate source */ todo!() }
    pub fn insert<C: Credential>(&mut self, _key: CredentialKey, _cred: C) { todo!() }
    #[inline]
    pub fn resolve_concrete<C: Credential>(&self, key: &str) -> Option<&C> {
        self.entries.get(key)?.as_any().downcast_ref::<C>()
    }
    #[inline]
    pub fn resolve_any(&self, key: &str) -> Option<&dyn AnyCredential> {
        self.entries.get(key).map(|b| b.as_ref() as &dyn AnyCredential)
    }
}

// ─────────────────────────────────────────────────────────────────────
// BUILTIN CRATE — credential-proto-builtin (iter-2)
// (depends on credential-proto)
// ─────────────────────────────────────────────────────────────────────

// ── Crate-private sealed module (ADR-0035 §3, per-capability variant) ─

mod sealed_caps {
    pub trait BearerSealed {}
    pub trait BasicSealed {}
    pub trait TlsIdentitySealed {}
    pub trait GenericBearerSealed {}
}

// ── Schemes + capability impls ────────────────────────────────────────

pub struct BearerScheme { pub token: String }
impl AuthScheme for BearerScheme {}
impl AcceptsBearer for BearerScheme {}

pub struct BasicScheme { pub user: String, pub pass: String }
impl AuthScheme for BasicScheme {}
impl AcceptsBasic for BasicScheme {}

pub struct TlsIdentityScheme { pub cert_pem: String, pub key_pem: String }
impl AuthScheme for TlsIdentityScheme {}
impl AcceptsTlsIdentity for TlsIdentityScheme {}

// ── Service marker + Pattern-2 capability chain (Bitbucket) ──────────

pub trait BitbucketCredential: Credential {}

// Layer 1 — real capability traits (supertrait-chained).
pub trait BitbucketBearer: BitbucketCredential {}
impl<T> BitbucketBearer for T where T: BitbucketCredential, T::Scheme: AcceptsBearer {}

pub trait BitbucketBasic: BitbucketCredential {}
impl<T> BitbucketBasic for T where T: BitbucketCredential, T::Scheme: AcceptsBasic {}

// Layer 2 — per-capability sealed blankets (sidesteps orphan coherence).
impl<T: BitbucketBearer> sealed_caps::BearerSealed for T {}
impl<T: BitbucketBasic> sealed_caps::BasicSealed for T {}

// Layer 3 — phantom traits (dyn-safe; `'static` dropped per §6 verification).
pub trait BitbucketBearerPhantom: sealed_caps::BearerSealed + Send + Sync {}
impl<T: BitbucketBearer> BitbucketBearerPhantom for T {}

pub trait BitbucketBasicPhantom: sealed_caps::BasicSealed + Send + Sync {}
impl<T: BitbucketBasic> BitbucketBasicPhantom for T {}

// ── Pattern-3 capability chain (service-agnostic TLS/Bearer) ─────────

pub trait SupportsTlsIdentity: Credential {}
impl<T> SupportsTlsIdentity for T where T: Credential, T::Scheme: AcceptsTlsIdentity {}
impl<T: SupportsTlsIdentity> sealed_caps::TlsIdentitySealed for T {}
pub trait TlsIdentityPhantom: sealed_caps::TlsIdentitySealed + Send + Sync {}
impl<T: SupportsTlsIdentity> TlsIdentityPhantom for T {}

pub trait SupportsBearer: Credential {}
impl<T> SupportsBearer for T where T: Credential, T::Scheme: AcceptsBearer {}
impl<T: SupportsBearer> sealed_caps::GenericBearerSealed for T {}
pub trait BearerPhantom: sealed_caps::GenericBearerSealed + Send + Sync {}
impl<T: SupportsBearer> BearerPhantom for T {}

// ── The Bitbucket triad (only shapes, impls in crate source) ─────────

pub struct BitbucketOAuth2;       // Scheme = BearerScheme
pub struct BitbucketPat;          // Scheme = BearerScheme
pub struct BitbucketAppPassword;  // Scheme = BasicScheme

// ── Dual-auth resource (Q4) ───────────────────────────────────────────

pub trait Resource {
    type AcceptedAuth;
}

pub struct MtlsHttpResource;
impl Resource for MtlsHttpResource {
    type AcceptedAuth = (TlsIdentityScheme, BearerScheme);
}

// ── Pattern-2 action consumer (one-slot form) ────────────────────────

pub struct GenericBitbucketAction {
    pub bb: CredentialRef<dyn BitbucketBearerPhantom>,
}

// ── Dual-auth action (two-slot form) ──────────────────────────────────

pub struct MtlsHttpAction {
    pub tls: CredentialRef<dyn TlsIdentityPhantom>,
    pub bearer: CredentialRef<dyn BearerPhantom>,
}

// ── §3.5 mechanism (i) — where-clause cross-check (resolve side) ─────
//
// Engine-side generic resolve fn. The where-clause `C: Credential<Scheme
// = BearerScheme>` is compile-enforced; wrong-scheme credential at
// invocation fails with E0271 (direct readable "expected X, found Y").
// The macro emits one such fn per (action, credential-slot) pair.

pub fn resolve_mtls_pair<C1, C2>(
    _reg: &CredentialRegistry,
    _tls_key: &str,
    _bearer_key: &str,
    _tls_state: &C1::State,
    _bearer_state: &C2::State,
) -> Option<(TlsIdentityScheme, BearerScheme)>
where
    C1: Credential<Scheme = TlsIdentityScheme>,
    C2: Credential<Scheme = BearerScheme>,
{
    None // impl in crate source
}

// ── Q2 — SingleCredentialAction for 0/1/2+-slot disambiguation ───────
//
// Macro emits this impl ONLY for 1-slot actions. 0/2+-slot actions get
// no impl; `ctx.credential()` fails with E0599 method-not-found.

pub trait SingleCredentialAction {
    type Cred: Credential;
    fn slot_key(&self) -> &str;
}

// Omitted from this snapshot: the ActionContext wrapper holding
// `credential()` method bound on `A: SingleCredentialAction`. See
// tests/action_macro_q2.rs.

// ── Compile assertions (in real source) ──────────────────────────────
//
//   const _: () = {
//       _assert_bearer_phantom::<BitbucketOAuth2>();     // ✓
//       _assert_bearer_phantom::<BitbucketPat>();        // ✓
//       _assert_basic_phantom::<BitbucketAppPassword>(); // ✓
//       _assert_tls_phantom::<MtlsClientCredential>();   // ✓
//       _assert_bearer_phantom_generic::<BitbucketOAuth2>(); // ✓
//   };
//
// Negative cases (compile-fail examples, MUST FAIL):
//   - compile_fail_app_password_is_not_bearer.rs
//   - compile_fail_app_password_via_phantom.rs
//   - compile_fail_external_forge.rs  ← rustc emits "sealed trait" hint
//   - compile_fail_app_password_to_bearer_projection.rs
//   - compile_fail_dualauth_wrong_scheme.rs
//   - compile_fail_zero_slot_credential_shorthand.rs
//   - compile_fail_two_slot_credential_shorthand.rs

fn main() {} // placate `cargo build` if anyone tries to run this as a bin
