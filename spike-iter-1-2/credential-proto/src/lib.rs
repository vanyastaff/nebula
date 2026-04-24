//! Spike contract crate — proves trait shape from Strategy §3 compiles.
//!
//! NOT production code. Throwaway. See `../NOTES.md` for iteration log.
//!
//! Mirrors the real `nebula-credential` shape (4 assoc types, `where Self: Sized`
//! on methods) but stripped of async/tokio/serde to keep the spike self-contained
//! and to focus pressure on the type-system question only.
//!
//! What this crate intentionally does NOT have:
//! - No `Sealed` enforcement (Q5 is out of spike scope per prompt).
//! - No real crypto, no real HTTP, no async runtime.
//! - No `CredentialMetadata` / `CredentialContext` / `RefreshPolicy`.
//! - Methods returning `()` rather than `impl Future<...>`. The `where Self: Sized`
//!   pattern is what governs dyn-safety; sync vs async does not change that
//!   for the §3.3 question.

#![allow(dead_code)]
#![forbid(unsafe_code)]

use std::any::{Any, TypeId};
use std::marker::PhantomData;
use std::sync::Arc;

/// ahash-backed HashMap. Strategy §3.4 "200–500ns typical" baseline
/// assumes perf-grade hasher; default SipHash would dominate bench noise.
/// Constructed with fixed seed in the spike for deterministic benches;
/// production would use `ahash::RandomState::new()` (runtime-rng feature).
type FastMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;

// ─────────────────────────────────────────────────────────────────────────────
// AuthScheme — mirrors `crates/credential/src/scheme/auth.rs` shape.
// Stripped: no serde, no chrono. Just the marker trait + the capability
// sub-traits the spike needs for §3.3.
// ─────────────────────────────────────────────────────────────────────────────

/// Consumer-facing auth material. Resources declare what shape they need;
/// credentials produce it via [`Credential::project`].
pub trait AuthScheme: Send + Sync + Clone + 'static {}

/// Capability marker — schemes that can fill an Authorization: Bearer header.
pub trait AcceptsBearer: AuthScheme {}

/// Capability marker — schemes that can fill an Authorization: Basic header.
pub trait AcceptsBasic: AuthScheme {}

/// Capability marker — schemes that sign HTTP requests (SigV4, HMAC, etc).
pub trait AcceptsSigning: AuthScheme {}

/// Capability marker — schemes that inject mTLS client identity at TLS handshake.
pub trait AcceptsTlsIdentity: AuthScheme {}

// ─────────────────────────────────────────────────────────────────────────────
// CredentialState / PendingState — opaque traits to keep `Credential`
// shape-faithful without dragging in real types.
// ─────────────────────────────────────────────────────────────────────────────

pub trait CredentialState: Send + Sync + Clone + 'static {}
pub trait PendingState: Send + Sync + 'static {}

/// Sentinel for non-interactive credentials. Mirrors `NoPendingState` in real code.
#[derive(Debug, Clone, Copy)]
pub struct NoPendingState;
impl PendingState for NoPendingState {}

// ─────────────────────────────────────────────────────────────────────────────
// Credential trait — shape held per Strategy §3.1.
// 4 assoc types. Every method has `where Self: Sized` — this is what allows
// `dyn Credential<...>` to exist as a type at all (vtable holds zero methods).
// ─────────────────────────────────────────────────────────────────────────────

/// Stand-in for `nebula_schema::HasSchema`. Spike doesn't validate schema bits.
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

    // Real trait has `resolve`, `continue_resolve`, `refresh`, `revoke`, `test`
    // returning `impl Future<Output = ...>`. For the spike, signatures
    // collapsed to sync `Result<()>` — doesn't change the dyn-safety shape
    // because all of these are gated `where Self: Sized` exactly as in real code.
    fn resolve_stub(_input: &Self::Input) -> Result<Self::State, &'static str>
    where
        Self: Sized,
    {
        Err("not implemented in spike")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AnyCredential — narrower object-safe trait.
// Per Strategy §3.2 last paragraph: runtime never holds a vtable pointer to
// `dyn BitbucketBearer` directly. The handle is `CredentialKey` + lookup
// returns `&dyn AnyCredential`, downcast at use site.
// ─────────────────────────────────────────────────────────────────────────────

pub trait AnyCredential: Any + Send + Sync + 'static {
    fn credential_key(&self) -> &'static str;
    fn type_id_marker(&self) -> TypeId;

    // Required for downcast. `Any` provides the machinery; this exposes
    // it through the trait-object boundary.
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

// ─────────────────────────────────────────────────────────────────────────────
// CredentialKey — opaque handle backed by Arc<str>.
//
// Iter-2 change (per orchestrator step 2): Arc<str> instead of String so
// Clone is a refcount bump (~ns) rather than a heap allocation. Registry
// lookups use `&str` via `Borrow` — no `CredentialKey::clone()` on the
// hot path at all.
//
// Why Arc<str> over &'static str: credential keys are tenant-supplied
// strings bound to runtime-loaded credentials; `&'static` would require
// leaking or a string interner. Arc<str> is the idiomatic owned-but-cheap-
// to-clone shape for this.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CredentialKey(pub Arc<str>);

impl CredentialKey {
    pub fn new(s: impl Into<Arc<str>>) -> Self {
        Self(s.into())
    }

    /// Cheap borrow for registry lookups.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for CredentialKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CredentialRef<C> — H1 (PhantomData + TypeId registry).
// Per Strategy §3.4 H1. `C: ?Sized` so `CredentialRef<dyn BitbucketBearer>`
// is a legal type — the `dyn` bound is purely nominal for compile-time
// signature checking; the runtime `key` carries no type information.
// ─────────────────────────────────────────────────────────────────────────────

pub struct CredentialRef<C: ?Sized> {
    pub key: CredentialKey,
    _t: PhantomData<fn() -> C>,
}

impl<C: ?Sized> CredentialRef<C> {
    pub const fn new(key: CredentialKey) -> Self {
        Self { key, _t: PhantomData }
    }
}

// `Send + Sync` are auto-derived: `PhantomData<fn() -> C>` is always
// Send+Sync (function pointer types carry no C even when C: ?Sized) and
// `CredentialKey` is Send+Sync. No manual unsafe impl needed — verified
// by build of `_send_sync` const fn below.
const _: fn() = || {
    fn _assert_send<T: Send>() {}
    fn _assert_sync<T: Sync>() {}
    _assert_send::<CredentialRef<dyn AnyCredential>>();
    _assert_sync::<CredentialRef<dyn AnyCredential>>();
};

impl<C: ?Sized> Clone for CredentialRef<C> {
    fn clone(&self) -> Self {
        Self { key: self.key.clone(), _t: PhantomData }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CredentialRegistry — iter-2 indexed H1 form.
//
// Changes from iter-1 (per orchestrator step 2):
//   - Keyed by Arc<str> (CredentialKey's backing) alone. Realistic shape:
//     one credential per key. TypeId safety preserved via downcast-ref on
//     the stored Box<dyn AnyCredential> (downcast checks TypeId internally).
//   - Lookup API accepts `&str` via Borrow — NO CredentialKey::clone() on
//     the hot path.
//   - ahash-backed FastMap — default SipHash would dominate bench noise
//     below the Strategy §3.4 200–500ns baseline assumption.
//   - resolve_any is now O(1) indexed lookup, not O(n) linear scan.
// ─────────────────────────────────────────────────────────────────────────────

pub struct CredentialRegistry {
    entries: FastMap<Arc<str>, Box<dyn AnyCredential>>,
}

impl Default for CredentialRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CredentialRegistry {
    pub fn new() -> Self {
        // ahash::RandomState's Default impl requires the `runtime-rng`
        // feature (which brings `getrandom`). For a spike that never leaves
        // the local process, explicit construction with a fixed seed is
        // sufficient AND ensures deterministic benches. Production would
        // use runtime-rng — call out in NOTES.md.
        let hasher = ahash::RandomState::with_seeds(0, 0, 0, 0);
        Self {
            entries: FastMap::with_hasher(hasher),
        }
    }

    pub fn insert<C: Credential>(&mut self, key: CredentialKey, cred: C) {
        self.entries.insert(key.0, Box::new(cred));
    }

    /// Concrete resolve. Lookup by `&str` (no CredentialKey allocation).
    /// Returns `None` if the key is absent or the registered credential's
    /// concrete type does not match `C` — the second case via downcast_ref's
    /// internal TypeId check.
    ///
    /// Hot-path cost: one ahash lookup + one TypeId compare (in downcast_ref).
    #[inline]
    pub fn resolve_concrete<C: Credential>(&self, key: &str) -> Option<&C> {
        self.entries.get(key)?.as_any().downcast_ref::<C>()
    }

    /// Capability-bound resolve — for Pattern 2 / Pattern 3 dyn consumers.
    /// Returns `&dyn AnyCredential`; caller (typically macro-generated) is
    /// responsible for concrete-type downcast.
    ///
    /// Hot-path cost: one ahash lookup. Per iter-2 §3 Pattern-2-dispatch
    /// evaluation: H1's caller-downcast model does NOT scale to plugin-
    /// registered concrete types (see NOTES.md §3).
    #[inline]
    pub fn resolve_any(&self, key: &str) -> Option<&dyn AnyCredential> {
        self.entries.get(key).map(|b| b.as_ref() as &dyn AnyCredential)
    }
}
