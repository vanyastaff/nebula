//! `SchemeGuard<'a, C>` — borrowed wrapper for refreshed Scheme material.
//!
//! Per Tech Spec §15.7 (closes security-lead N8 + tech-lead gap (i)) and
//! spike iter-3 secondary finding (lifetime-gap refinement, commit
//! `f36f3739` 2026-04-24).
//!
//! Long-lived resources (connection pools, daemons) need to react to
//! credential refresh events. The naive shape — `&new_scheme: &Scheme` —
//! invites a class of correctness bugs:
//!
//! - clone-into-side-channel via `Arc<Scheme>` retention if `Scheme: Clone`,
//! - lifetime-smuggling via `unsafe` / `transmute`,
//! - stale-borrow under async cancellation if the resource captured `&Scheme` pre-await and the
//!   future drops mid-call.
//!
//! `SchemeGuard<'a, C>` closes those holes structurally:
//!
//! - **`!Clone`** — no implicit copy of plaintext scheme material. Verified by Probe 7
//!   (`compile_fail_scheme_guard_clone.rs`).
//! - **`ZeroizeOnDrop`** — drop deterministically zeroizes plaintext via the wrapped scheme's own
//!   `ZeroizeOnDrop` (mandated for `SensitiveScheme` per §15.5).
//! - **Lifetime-pinned** via `PhantomData<&'a ()>`. Engine passes `SchemeGuard<'a, C>` alongside a
//!   borrow that shares `'a`, so any attempt to retain the guard past the call forces the borrow to
//!   outlive its source — an `E0597` lifetime error. Verified by Probe 6
//!   (`compile_fail_scheme_guard_retention.rs`).
//!
//! `SchemeFactory<C>` is the companion re-acquisition mechanism for
//! resources that need a fresh guard per request rather than cached
//! retention.

use std::{future::Future, marker::PhantomData, ops::Deref, pin::Pin, sync::Arc};

use crate::{Credential, error::CredentialError};

// ── SchemeGuard ─────────────────────────────────────────────────────────────

/// Borrowed wrapper for a refreshed `Scheme`.
///
/// The `'a` lifetime is shared with a borrow that the engine passes alongside
/// the guard at the call site (typically `&'a CredentialContext`). Any
/// attempt to retain the guard past the call forces the engine borrow to
/// outlive its source, which the borrow checker rejects (`E0597`).
///
/// # Invariants enforced at compile time
///
/// - `!Clone` — no `Clone` impl on this type. Verified by Probe 7.
/// - Drop semantics depend on `<C as Credential>::Scheme`:
///   - `SensitiveScheme: AuthScheme + ZeroizeOnDrop` (per §15.5) — drop zeroizes the wrapped
///     scheme's plaintext deterministically through the scheme's own `ZeroizeOnDrop` impl. This is
///     the path that closes N4 / N8 (no plaintext lives past the call boundary).
///   - `PublicScheme: AuthScheme` (per §15.5) — wrapped scheme has no secrets, so no zeroize is
///     needed; drop is a no-op beyond the field destructor. The guard pattern still applies because
///     the **lifetime invariant** (`!Clone` + `'a` pinning) is independent of the scheme's
///     sensitivity tier.
/// - **`!Send + !Sync`** (SEC-06 hardening 2026-04-27 Stage 2) — `_thread_marker:
///   PhantomData<*const ()>` field opts out of `Send + Sync` regardless of the wrapped scheme's
///   auto-traits. Plaintext scheme material cannot cross thread boundaries (no `tokio::spawn`, no
///   `spawn_blocking` move-into-closure), closing PRODUCT_CANON §4.2 invariant N10's
///   blocking-pool-thread vector.
///
/// # Construction
///
/// Crate-private — only the engine constructs guards. Resource impls
/// receive guards via the refresh hook; they cannot fabricate their own.
pub struct SchemeGuard<'a, C: Credential> {
    scheme: <C as Credential>::Scheme,
    _lifetime: PhantomData<&'a ()>,
    /// SEC-06 (security hardening 2026-04-27 Stage 2). Raw-pointer `PhantomData`
    /// strips both `Send` and `Sync` from the guard, so plaintext scheme
    /// material cannot be moved to a blocking-pool worker via `spawn_blocking`
    /// or shared via `Arc` across `tokio::spawn` boundaries.
    _thread_marker: PhantomData<*const ()>,
}

impl<C: Credential> SchemeGuard<'_, C> {
    /// Crate-private constructor — only the engine creates these.
    ///
    /// `#[allow(dead_code)]` is deliberate: the engine wiring that calls
    /// this constructor lands in a follow-up cascade (engine `refresh()`
    /// hook + `SchemeFactory` driver). Stage 6 lands the type surface so
    /// the engine has something to call into; the call site is wired in
    /// the subsequent stage.
    #[allow(dead_code)]
    pub(crate) fn new(scheme: <C as Credential>::Scheme) -> Self {
        Self {
            scheme,
            _lifetime: PhantomData,
            _thread_marker: PhantomData,
        }
    }

    /// Test-only constructor for resource-side integration tests.
    ///
    /// Mirrors [`SchemeGuard::new`] but is publicly callable so external
    /// integration tests (notably `nebula-resource`'s rotation dispatch
    /// suite) can fabricate guards for fixture resources. Gated behind
    /// `#[cfg(any(test, feature = "test-util"))]` per ADR-0023 — this
    /// constructor MUST NOT be exposed in a production release build.
    ///
    /// Production code paths use the engine-driven flow that calls
    /// [`SchemeFactory::acquire`], which hands out a borrow-pinned guard
    /// the borrow checker rejects retention on (Probe 6).
    #[cfg(any(test, feature = "test-util"))]
    pub fn for_test(scheme: <C as Credential>::Scheme) -> Self {
        Self::new(scheme)
    }
}

impl<C: Credential> Deref for SchemeGuard<'_, C> {
    type Target = <C as Credential>::Scheme;

    fn deref(&self) -> &Self::Target {
        &self.scheme
    }
}

// `Drop` is implicit on the wrapped field — `SensitiveScheme: ZeroizeOnDrop`
// fires when `self.scheme` drops. We do not declare an explicit `Drop` impl
// because doing so disables the borrow checker's drop-order optimisation
// without changing observable behaviour. The `ZeroizeOnDrop` derive on the
// wrapped scheme is what actually zeroes the bytes.

// IMPORTANT: NO `Clone` impl — Probe 7 verifies `clone()` is rejected at
// the call site (`E0599: no method named 'clone' found`).

// ── SchemeFactory ───────────────────────────────────────────────────────────

/// Pinned-future return type for the closure stored inside a
/// [`SchemeFactory`].
///
/// The inner closure produces a `'static` guard — the factory's public
/// [`SchemeFactory::acquire`] then re-binds that guard's lifetime to
/// `&self` via subtyping, so callers receive `SchemeGuard<'_, C>` and
/// cannot hoist it out of the factory's scope.
///
/// `'static` on the inner future side keeps the `Arc<dyn Fn>` HRTB-free,
/// which is required: `for<'a> Fn() -> Future<'a>` is rejected by the
/// compiler because `'a` does not appear in the `Fn` input types
/// (`E0582`). Tightening to `'static` on the closure and downcasting in
/// the public method is the standard workaround per the closure-HRTB
/// pattern.
// SEC-06 hardening: SchemeGuard is `!Send`, so the future cannot be `+ Send`
// either. Callers must `.await` `acquire()` from the same task that holds
// the SchemeFactory; cross-runtime-thread movement is structurally rejected.
type AcquireFuture<C> =
    Pin<Box<dyn Future<Output = Result<SchemeGuard<'static, C>, CredentialError>>>>;

/// Factory for fresh `SchemeGuard` acquisition.
///
/// Long-lived resources (connection pools, daemons) invoke `acquire()` per
/// request rather than retaining a guard. The factory is `Clone` (cheap
/// `Arc` refcount bump) so it can be stashed inside a pool struct and
/// shared across worker tasks. The yielded `SchemeGuard` is `!Clone` —
/// the divergence is deliberate: the factory itself is fan-out, the
/// produced guard is single-use.
///
/// # Lifecycle
///
/// ```text
/// engine.refresh()        ──▶ SchemeFactory<C>      ──▶ resource pool stash
///                                  │
///                                  │   (per request)
///                                  ▼
///                               SchemeGuard<'_, C>  ──▶ scoped use, drop, zeroize
/// ```
///
/// See Tech Spec §15.7's worked `OAuth2HttpPool` example for the canonical
/// usage pattern.
pub struct SchemeFactory<C: Credential> {
    inner: Arc<dyn Fn() -> AcquireFuture<C> + Send + Sync>,
}

impl<C: Credential> SchemeFactory<C> {
    /// Crate-private constructor — only the engine creates factories.
    ///
    /// Engine code wires the closure to the credential resolution pipeline
    /// (`refresh` → `project` → `SchemeGuard::new`).
    ///
    /// `#[allow(dead_code)]` is deliberate (same rationale as
    /// [`SchemeGuard::new`]): the engine driver lands in a follow-up cascade.
    #[allow(dead_code)]
    pub(crate) fn new<F>(f: F) -> Self
    where
        F: Fn() -> AcquireFuture<C> + Send + Sync + 'static,
    {
        Self { inner: Arc::new(f) }
    }

    /// Test-only constructor for resource-side integration tests.
    ///
    /// Mirrors [`SchemeFactory::new`] but is publicly callable so external
    /// integration tests (notably `nebula-resource`'s rotation dispatch
    /// suite) can fabricate factories for fixture resources. Gated behind
    /// `#[cfg(any(test, feature = "test-util"))]` per ADR-0023 — this
    /// constructor MUST NOT be exposed in a production release build.
    ///
    /// The closure shape matches the canonical engine wiring: per call it
    /// must produce a pinned future yielding a fresh `SchemeGuard<'static, C>`
    /// (which `acquire` re-binds to `&self`). Tests typically capture an
    /// owned scheme prototype and clone it inside the closure body — see
    /// `crates/resource/tests/rotation.rs` for the canonical fixture form.
    #[cfg(any(test, feature = "test-util"))]
    pub fn for_test<F>(f: F) -> Self
    where
        F: Fn() -> AcquireFuture<C> + Send + Sync + 'static,
    {
        Self::new(f)
    }

    /// Acquire a fresh guard tied to the factory's lifetime.
    ///
    /// The returned guard borrows from the factory; it cannot be hoisted
    /// out of scope. Call once per logical request — never cache the
    /// returned guard.
    pub async fn acquire(&self) -> Result<SchemeGuard<'_, C>, CredentialError> {
        // Inner closure produces `SchemeGuard<'static, C>` so the boxed
        // future stays HRTB-free. Subtyping shrinks `'static` to the
        // borrow lifetime of `&self`, so callers see the factory-bound
        // guard and the borrow checker rejects retention attempts past
        // the factory's lifetime.
        (self.inner)().await
    }
}

#[cfg(any(test, feature = "test-util"))]
impl<C: Credential> SchemeFactory<C>
where
    <C as Credential>::Scheme: Clone,
{
    /// Test-only convenience: construct a factory that yields clones of the
    /// supplied scheme on every [`acquire`](Self::acquire) call.
    ///
    /// Wraps [`SchemeFactory::for_test`] with the common test pattern of "I
    /// already have a scheme value, hand it out N times." Requires
    /// `Scheme: Clone` (most schemes — `PublicScheme` mocks and
    /// `SensitiveScheme` carrying `SecretString` clones — satisfy this).
    /// Production code never relies on `Scheme: Clone`; only the test path
    /// requires it.
    ///
    /// Gated behind `#[cfg(any(test, feature = "test-util"))]` per ADR-0023.
    pub fn for_test_static(scheme: <C as Credential>::Scheme) -> Self {
        Self::for_test(move || {
            let scheme = scheme.clone();
            Box::pin(async move { Ok(SchemeGuard::for_test(scheme)) })
        })
    }
}

impl<C: Credential> Clone for SchemeFactory<C> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<C: Credential> std::fmt::Debug for SchemeFactory<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchemeFactory")
            .field("credential", &std::any::type_name::<C>())
            .finish_non_exhaustive()
    }
}

// Refresh-notification hook lives on `nebula_resource::Resource` itself
// (`Resource::on_credential_refresh`) per ADR-0036 + Tech Spec §15.4. The
// previously-defined parallel `OnCredentialRefresh<C>` trait was a
// transitional bridge while `Resource` still bound `Auth: AuthScheme`; the
// П1 reshape moved the canonical hook onto `Resource`, П2 wired Manager
// dispatch on the method, and the parallel trait was removed in П2 Task 12.
//
// SEC-06 (security hardening 2026-04-27 Stage 2) note: SchemeGuard is now
// `!Send + !Sync` regardless of the wrapped scheme's auto-traits. Any
// future on `Resource::on_credential_refresh` that captures a SchemeGuard
// is therefore `!Send`. Manager dispatch must `.await` per-task; resources
// cannot `tokio::spawn` per-resource handlers that hold the guard across
// the spawn boundary. See `stage6-followup-resource-integration` in
// `docs/tracking/credential-concerns-register.md` for the dispatch shape.
