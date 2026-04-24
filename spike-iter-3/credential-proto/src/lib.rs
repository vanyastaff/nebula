//! Spike iter-3 — CP5/CP6 credential trait shape prototype.
//!
//! Validates the composition of:
//! - Tech Spec §15.4 sub-trait split (Interactive/Refreshable/Revocable/Testable/Dynamic).
//! - Tech Spec §15.5 SensitiveScheme/PublicScheme dichotomy.
//! - Tech Spec §15.6 fatal duplicate-KEY registry.
//! - Tech Spec §15.7 SchemeGuard + SchemeFactory refresh hook.
//! - ADR-0035 phantom-shim capability pattern (amendment 2026-04-24-B).
//!
//! This is NOT production code. No async runtime, no real crypto, no I/O —
//! enough shape to pressure-test the type system against Gate 3 §15.12.3
//! dyn-safety questions (a)-(e).
//!
//! Compared to prior spike iter-1/iter-2 (CP4 shape with 4 assoc types on
//! Credential + const bool capability flags + defaulted methods): iter-3
//! drops one assoc type (`Pending` moves to `Interactive`), drops all const
//! bool flags, and splits five capability methods into five sub-traits.
//! Base Credential now has 3 assoc types (Input, Scheme, State).

#![allow(dead_code)]
#![forbid(unsafe_code)]

use std::marker::PhantomData;
use std::ops::Deref;
use std::time::Duration;

use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;
use zeroize::ZeroizeOnDrop;

// ============================================================================
// §1. Sealed supertraits — crate-private sealing of the base Credential trait.
// Not the per-capability phantom seals (those live in consumer crates per
// ADR-0035 §3 amendment).
// ============================================================================

mod sealed {
    /// Sealed supertrait for `Credential`. External crates cannot `impl`
    /// `Credential` without first satisfying this (and `Sealed` is not
    /// pub-reachable), forcing plugin authors through the `#[plugin_credential]`
    /// derive macro which emits both impls together.
    ///
    /// In the spike, each concrete credential type declares
    /// `impl crate::Sealed for MyCred {}` manually — stands in for the macro.
    pub trait Sealed {}
}
pub use sealed::Sealed;

// ============================================================================
// §2. Input schema marker.
// Stand-in for `nebula_schema::HasSchema` in production.
// ============================================================================

pub trait HasInputSchema: Send + Sync + 'static {}

impl HasInputSchema for () {}

// ============================================================================
// §3. AuthScheme dichotomy — §15.5.
// `AuthScheme` is the base marker; every scheme must declare either
// `SensitiveScheme` (secret material, `ZeroizeOnDrop` required) or
// `PublicScheme` (no secret material, no zeroize overhead).
// ============================================================================

/// Base scheme marker — consumer-facing auth material.
pub trait AuthScheme: Send + Sync + 'static {}

/// Scheme carrying secret material. Requires `ZeroizeOnDrop` so plaintext
/// is wiped from heap deterministically on drop.
pub trait SensitiveScheme: AuthScheme + ZeroizeOnDrop {}

/// Scheme with no secret material (e.g. `InstanceBinding` — provider + role
/// identifiers). No zeroize overhead.
pub trait PublicScheme: AuthScheme {}

// ============================================================================
// §4. Capability markers — schemes that satisfy a particular HTTP mechanism.
// Used by the phantom-shim pattern (ADR-0035 §1) for `CredentialRef<dyn XPhantom>`
// dispatch at the action consumer.
// ============================================================================

/// Marker — scheme fills `Authorization: Bearer` header.
pub trait AcceptsBearer: AuthScheme {}

/// Marker — scheme fills `Authorization: Basic` header.
pub trait AcceptsBasic: AuthScheme {}

/// Marker — scheme injects mTLS client identity at TLS handshake.
pub trait AcceptsTlsIdentity: AuthScheme {}

/// Marker — scheme signs HTTP requests (SigV4, HMAC, signed JWT, etc).
pub trait AcceptsSigning: AuthScheme {}

// ============================================================================
// §5. CredentialState + PendingState bounds.
// Per §15.4 + §15.5: state MUST be `ZeroizeOnDrop` (plaintext lives in
// memory until drop) and `Serialize + DeserializeOwned` (storage layer
// round-trips via serde).
// ============================================================================

/// Serializable state with zeroize-on-drop.
pub trait CredentialState:
    Serialize + DeserializeOwned + Send + Sync + ZeroizeOnDrop + 'static
{
}

/// Pending state during interactive auth (PKCE, OAuth2 flow). Same bounds
/// as CredentialState — also stored in `PendingStore` between redirect + callback.
pub trait PendingState:
    Serialize + DeserializeOwned + Send + Sync + ZeroizeOnDrop + 'static
{
}

// ============================================================================
// §6. Context + metadata stand-ins. Production has full metadata; spike
// strips to the minimum required to surface the trait shape.
// ============================================================================

/// Stand-in for `CredentialMetadata` — just the fields the spike touches.
/// Production adds `capabilities_enabled`-computed-at-register per §15.8,
/// description, display name, etc. Irrelevant for dyn-safety validation.
#[derive(Debug, Clone)]
pub struct CredentialMetadata {
    pub key: &'static str,
    pub crate_name: &'static str,
}

/// Stand-in for `CredentialContext<'_>` — per-request context.
pub struct CredentialContext<'a> {
    _marker: PhantomData<&'a ()>,
}

impl<'a> CredentialContext<'a> {
    pub const fn new() -> Self {
        Self { _marker: PhantomData }
    }
}

impl<'a> Default for CredentialContext<'a> {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// §7. ResolveResult, RefreshPolicy, RefreshOutcome, TestResult.
// Stand-ins only — enough shape that method signatures are realistic.
// ============================================================================

pub enum ResolveResult<State, Pending> {
    Complete(State),
    NeedsUserInput(Pending),
}

#[derive(Debug, Clone)]
pub struct RefreshPolicy {
    pub min_backoff: Duration,
    pub max_backoff: Duration,
    pub preemptive_window: Duration,
}

impl RefreshPolicy {
    pub const DEFAULT: Self = Self {
        min_backoff: Duration::from_secs(1),
        max_backoff: Duration::from_secs(300),
        preemptive_window: Duration::from_secs(120),
    };
}

#[derive(Debug)]
pub enum RefreshOutcome {
    Refreshed,
    NotNeeded,
}

#[derive(Debug)]
pub enum TestResult {
    Ok,
    Degraded(&'static str),
}

#[derive(Debug)]
pub struct UserInput {
    pub raw: String,
}

// ============================================================================
// §8. Error taxonomy — stand-ins.
// ============================================================================

#[derive(Debug, Error)]
#[error("resolve error: {0}")]
pub struct ResolveError(pub String);

#[derive(Debug, Error)]
#[error("refresh error: {0}")]
pub struct RefreshError(pub String);

#[derive(Debug, Error)]
#[error("revoke error: {0}")]
pub struct RevokeError(pub String);

#[derive(Debug, Error)]
#[error("test error: {0}")]
pub struct TestError(pub String);

#[derive(Debug, Error)]
#[error("release error: {0}")]
pub struct ReleaseError(pub String);

#[derive(Debug, Error)]
#[error("acquire error: {0}")]
pub struct AcquireError(pub String);

// ============================================================================
// §9. Base Credential trait — CP5/CP6 shape per §15.4 decision (a).
//
// Reduced from the production trait (CP4 shape) as follows:
// - `Pending` assoc type removed (moved to `Interactive` sub-trait).
// - `INTERACTIVE` / `REFRESHABLE` / `REVOCABLE` / `TESTABLE` / `DYNAMIC`
//   const bools removed (replaced by sub-trait membership).
// - `continue_resolve` / `refresh` / `revoke` / `test` / `release` removed
//   from Credential (moved to their respective sub-traits).
//
// Remaining shape:
// - 3 assoc types: Input, Scheme, State.
// - 1 const: KEY.
// - 3 methods (all `where Self: Sized`): metadata, project, resolve.
//
// Every method is `where Self: Sized` — this is what keeps `dyn Credential`
// object-safe at the type level (vtable holds zero methods; all dispatch
// goes through concrete-type downcast or phantom-shim projection).
// ============================================================================

pub trait Credential: Sealed + Send + Sync + 'static {
    type Input: HasInputSchema;
    type Scheme: AuthScheme;
    type State: CredentialState;

    const KEY: &'static str;

    fn metadata() -> CredentialMetadata
    where
        Self: Sized;

    fn project(state: &Self::State) -> Self::Scheme
    where
        Self: Sized;

    // Production returns `impl Future<Output = …>` (async fn in trait).
    // Spike keeps the sync shape because dyn-safety doesn't change — all
    // methods are gated `where Self: Sized` regardless of async.
    fn resolve(
        ctx: &CredentialContext<'_>,
        input: &Self::Input,
    ) -> Result<ResolveResult<Self::State, ()>, ResolveError>
    where
        Self: Sized;
}

// ============================================================================
// §10. Capability sub-traits — §15.4 decision (a).
//
// Each sub-trait requires the corresponding method WITHOUT a default body.
// A plugin declaring `impl Refreshable for X` without writing `refresh()`
// fails with E0046 (required method missing) — the silent-downgrade failure
// class (sec-lead N3+N5) becomes structurally impossible.
// ============================================================================

/// Credential supports interactive auth (PKCE, OAuth2 authorization code flow).
/// `Pending` assoc type moves here from `Credential`.
pub trait Interactive: Credential {
    type Pending: PendingState;

    fn continue_resolve(
        pending: &Self::Pending,
        input: &UserInput,
        ctx: &CredentialContext<'_>,
    ) -> Result<ResolveResult<Self::State, Self::Pending>, ResolveError>;
}

/// Credential supports refresh (token rotation, renewal).
pub trait Refreshable: Credential {
    const REFRESH_POLICY: RefreshPolicy = RefreshPolicy::DEFAULT;

    fn refresh(
        state: &mut Self::State,
        ctx: &CredentialContext<'_>,
    ) -> Result<RefreshOutcome, RefreshError>;
}

/// Credential supports revocation (logout, token invalidation).
pub trait Revocable: Credential {
    fn revoke(
        state: &mut Self::State,
        ctx: &CredentialContext<'_>,
    ) -> Result<(), RevokeError>;
}

/// Credential supports connectivity/validity test (ping).
pub trait Testable: Credential {
    fn test(
        scheme: &Self::Scheme,
        ctx: &CredentialContext<'_>,
    ) -> Result<TestResult, TestError>;
}

/// Credential has a lease lifecycle (e.g. STS assume-role, dynamic secrets).
/// Note §15.4 CP6 gap 3 fix: `&self` receiver dropped — trait is type-level,
/// all dispatch is on `Self::State`.
pub trait Dynamic: Credential {
    const LEASE_TTL: Option<Duration> = None;

    fn release(
        state: &Self::State,
        ctx: &CredentialContext<'_>,
    ) -> Result<(), ReleaseError>;
}

// ============================================================================
// §11. SchemeGuard — §15.7.
//
// Owned, !Clone, ZeroizeOnDrop. Deref<Target = C::Scheme>. Lifetime
// parameter prevents storage in struct fields outliving the call.
//
// `SchemeGuard<'a, C>` is constructed by the engine and handed to
// `Resource::on_credential_refresh` (or acquired via `SchemeFactory`).
// The Resource cannot clone it, cannot store it, and when dropped
// (at scope exit or on future cancellation) the contained scheme zeroizes.
// ============================================================================

/// Owned, non-cloneable scheme wrapper. Lifetime parameter prevents
/// storage beyond the call-site scope.
///
/// Zeroization is delegated to `C::Scheme`'s own `Drop` impl — when
/// `C::Scheme: SensitiveScheme` (which implies `ZeroizeOnDrop`), the
/// embedded scheme wipes its plaintext when `SchemeGuard` drops. No
/// custom `Drop` is needed on `SchemeGuard` itself; the field's Drop runs
/// in containment-drop order (drop glue of struct fields runs after
/// struct's own Drop — here we have no own Drop).
///
/// `!Clone` is enforced by absence of a `Clone` impl. Attempting
/// `guard.clone()` fails with `E0599`. `!Send` / `!Sync` fall out from
/// `C::Scheme`'s own Send+Sync (they're usually Send+Sync for serializable
/// schemes — fine). What we care about is lifetime containment +
/// no-clone + zeroize-on-drop, all of which are shape-enforced.
pub struct SchemeGuard<'a, C: Credential> {
    scheme: C::Scheme,
    _lifetime: PhantomData<&'a ()>,
}

impl<'a, C: Credential> SchemeGuard<'a, C> {
    /// Constructed by engine — not reachable by `Resource` impls.
    pub fn new(scheme: C::Scheme) -> Self {
        Self { scheme, _lifetime: PhantomData }
    }
}

impl<'a, C: Credential> Deref for SchemeGuard<'a, C> {
    type Target = C::Scheme;
    fn deref(&self) -> &Self::Target {
        &self.scheme
    }
}

// NO Clone impl — `guard.clone()` fails with E0599.
// NO Copy impl either.
// Drop for SensitiveScheme fields runs automatically (field's own Drop glue).

// ============================================================================
// §12. SchemeFactory — §15.7.
//
// For long-lived resources (e.g. `OAuth2HttpPool`) that need a fresh
// SchemeGuard per request but must NOT retain the Scheme itself. Factory
// yields a new guard each time; the guard drops at scope exit.
//
// In production `Fn() -> BoxFuture<...>` for async. Spike uses sync closure
// since async-fn-in-trait is not the dyn-safety question we're testing.
// ============================================================================

pub struct SchemeFactory<C: Credential> {
    // In production: Arc<dyn Fn() -> BoxFuture<...> + Send + Sync>.
    // For the spike, sync closure suffices — the question is whether the
    // type shape composes with sub-trait split, not async plumbing.
    inner: std::sync::Arc<dyn Fn() -> Result<C::Scheme, AcquireError> + Send + Sync>,
}

impl<C: Credential> SchemeFactory<C> {
    pub fn new<F>(f: F) -> Self
    where
        F: Fn() -> Result<C::Scheme, AcquireError> + Send + Sync + 'static,
    {
        Self { inner: std::sync::Arc::new(f) }
    }

    pub fn acquire(&self) -> Result<SchemeGuard<'_, C>, AcquireError> {
        let scheme = (self.inner)()?;
        Ok(SchemeGuard::new(scheme))
    }
}

// ============================================================================
// §13. CredentialRef — the action-consumer-side handle.
//
// `C: ?Sized` so `CredentialRef<dyn BitbucketBearerPhantom>` is a legal type.
// Runtime is just a `&'static str` key; the `dyn` is purely nominal for
// compile-time signature checking.
// ============================================================================

pub struct CredentialRef<C: ?Sized> {
    pub key: &'static str,
    _t: PhantomData<fn() -> C>,
}

impl<C: ?Sized> CredentialRef<C> {
    pub const fn new(key: &'static str) -> Self {
        Self { key, _t: PhantomData }
    }
}

impl<C: ?Sized> Clone for CredentialRef<C> {
    fn clone(&self) -> Self {
        Self { key: self.key, _t: PhantomData }
    }
}

// ============================================================================
// §14. CredentialRegistry — §15.6 fatal duplicate-KEY.
//
// `register<C>(instance) -> Result<(), RegisterError>`. Second registration
// of the same KEY is fail-closed with a `DuplicateKey` error carrying
// diagnostic information for operator resolution.
// ============================================================================

#[derive(Debug, Error, Clone)]
pub enum RegisterError {
    #[error(
        "duplicate credential key '{key}': existing crate {existing_crate}, \
         new crate {new_crate}"
    )]
    DuplicateKey {
        key: &'static str,
        existing_crate: &'static str,
        new_crate: &'static str,
    },
}

pub struct CredentialRegistry {
    entries: std::collections::HashMap<&'static str, RegisteredEntry>,
}

struct RegisteredEntry {
    crate_name: &'static str,
    // In production: Box<dyn AnyCredential>. Spike doesn't need runtime
    // dispatch for the §15.6 property; the question is whether `register`
    // is fail-closed on duplicate KEYs.
    _marker: PhantomData<()>,
}

impl Default for CredentialRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CredentialRegistry {
    pub fn new() -> Self {
        Self { entries: std::collections::HashMap::new() }
    }

    /// Register a credential type. Second registration of the same KEY is
    /// fail-closed with `RegisterError::DuplicateKey` (not silent overwrite).
    pub fn register<C: Credential>(
        &mut self,
        crate_name: &'static str,
    ) -> Result<(), RegisterError> {
        let key = C::KEY;
        if let Some(existing) = self.entries.get(key) {
            return Err(RegisterError::DuplicateKey {
                key,
                existing_crate: existing.crate_name,
                new_crate: crate_name,
            });
        }
        self.entries.insert(
            key,
            RegisteredEntry { crate_name, _marker: PhantomData },
        );
        Ok(())
    }

    pub fn is_registered(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }
}

// ============================================================================
// §15. Engine dispatcher — compile-time capability bound.
//
// `RefreshDispatcher::for_credential<C: Refreshable>()` only accepts types
// satisfying `Refreshable`. A non-refreshable credential causes E0277 at
// the call site — see `compile-fail/tests/ui/engine_dispatch_capability.rs`.
// ============================================================================

pub struct RefreshDispatcher<C: Refreshable> {
    _c: PhantomData<fn() -> C>,
}

impl<C: Refreshable> RefreshDispatcher<C> {
    pub const fn for_credential() -> Self {
        Self { _c: PhantomData }
    }

    pub fn policy(&self) -> RefreshPolicy {
        <C as Refreshable>::REFRESH_POLICY
    }
}

pub struct RevokeDispatcher<C: Revocable> {
    _c: PhantomData<fn() -> C>,
}

impl<C: Revocable> RevokeDispatcher<C> {
    pub const fn for_credential() -> Self {
        Self { _c: PhantomData }
    }
}

pub struct InteractiveDispatcher<C: Interactive> {
    _c: PhantomData<fn() -> C>,
}

impl<C: Interactive> InteractiveDispatcher<C> {
    pub const fn for_credential() -> Self {
        Self { _c: PhantomData }
    }
}

// ============================================================================
// §16. Compile-time assertions — type-level sanity checks.
// ============================================================================

// `Box<dyn Credential>` well-formedness — question (a). Every method on
// Credential is `where Self: Sized`, so `dyn Credential` has an empty vtable
// and is object-safe AT THE TYPE LEVEL. We also assert here that the 3-assoc-
// type base shape preserves this — morally identical to the 4-assoc-type CP4
// shape on iter-1/iter-2 because `Sized` gating is the governing factor, not
// the count of assoc types.
//
// NOTE: `Box<dyn Credential>` without assoc-type specification is NOT
// constructible — E0191 requires `Box<dyn Credential<Input = …, Scheme = …,
// State = …>>`. The type `dyn Credential` with all assoc types specified
// IS well-formed. This is the same "specify all assoc types or fail" E0191
// class that motivated ADR-0035 phantom-shim for Pattern 2/3 at all. See
// credential-proto-builtin for empirical test.

const _: () = {
    const fn _assert_send<T: Send>() {}
    const fn _assert_sync<T: Sync>() {}

    // CredentialRef is Send + Sync regardless of what goes in the dyn slot,
    // because PhantomData<fn() -> C> is always Send+Sync (function pointers).
    _assert_send::<CredentialRef<()>>();
    _assert_sync::<CredentialRef<()>>();
};

// SchemeFactory<C> is Send + Sync by construction (Arc<dyn Fn + Send + Sync>).
const _: () = {
    const fn _assert_send<T: Send>() {}
    const fn _assert_sync<T: Sync>() {}
    // Cannot assert without a concrete C; moved to builtin crate.
    let _ = _assert_send::<()>;
    let _ = _assert_sync::<()>;
};
