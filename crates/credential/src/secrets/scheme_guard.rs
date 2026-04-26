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

use crate::{Credential, context::CredentialContext, error::CredentialError};

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
/// - `Send + Sync` are inherited from `<C as Credential>::Scheme` via the wrapped field. Per §15.5,
///   `AuthScheme: Send + Sync + 'static`, so every `SchemeGuard<'a, C>` is `Send + Sync` — no
///   conditional language needed. The `PhantomData<&'a ()>` carries no auto-trait bounds of its own
///   beyond what `&'a ()` carries (which is `Send + Sync` for any `'a`).
///
/// # Construction
///
/// Crate-private — only the engine constructs guards. Resource impls
/// receive guards via the refresh hook; they cannot fabricate their own.
pub struct SchemeGuard<'a, C: Credential> {
    scheme: <C as Credential>::Scheme,
    _lifetime: PhantomData<&'a ()>,
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
        }
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
type AcquireFuture<C> =
    Pin<Box<dyn Future<Output = Result<SchemeGuard<'static, C>, CredentialError>> + Send>>;

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

// ── OnCredentialRefresh ─────────────────────────────────────────────────────

/// Refresh-notification hook for credential-bound resources.
///
/// Per Tech Spec §15.7 spike iter-3: when the engine refreshes a
/// credential, it notifies bound resources by calling
/// [`on_credential_refresh`](OnCredentialRefresh::on_credential_refresh)
/// with a fresh [`SchemeGuard`] plus a shared-lifetime [`CredentialContext`]
/// borrow. The shared `'a` lifetime between guard and context is the
/// structural barrier that prevents retention — see Probe 6
/// (`compile_fail_scheme_guard_retention.rs`).
///
/// # Why a parallel trait, not a method on `nebula_resource::Resource`
///
/// The `Resource` trait in `nebula-resource` carries 5 associated types
/// (`Config` / `Runtime` / `Lease` / `Error` / `Auth`) and currently links
/// to credentials via `Auth: AuthScheme`, not `Credential: Credential`.
/// Adding a required `type Credential: Credential` would force every
/// existing `Resource` impl (28+ test impls in `nebula-resource` alone)
/// to either nominate a real `Credential` type or accept a
/// no-op-credential placeholder. Default associated types are unstable
/// on stable Rust 1.95, so the placeholder approach is closed.
///
/// `OnCredentialRefresh<C>` is the spec-canonical signature shape —
/// resources that do react to credential refresh implement this trait
/// alongside `Resource`. Resources that don't (the common case) leave it
/// unimplemented. The trait lives in `nebula-credential` so it sees
/// `SchemeGuard` + `CredentialContext` directly.
///
/// # Cancellation safety
///
/// Implementations MUST be cancel-safe: if the future is dropped
/// mid-await, the wrapped scheme must remain consistent. `SchemeGuard`'s
/// `ZeroizeOnDrop` semantics fire deterministically across the
/// cancellation boundary, so plaintext does not survive a dropped
/// future.
///
/// # Example
///
/// ```ignore
/// use nebula_credential::{OnCredentialRefresh, SchemeGuard, CredentialContext};
///
/// struct MyPool { /* ... */ }
///
/// impl OnCredentialRefresh<MyOAuth2Credential> for MyPool {
///     type Error = MyPoolError;
///
///     async fn on_credential_refresh<'a>(
///         &self,
///         new_scheme: SchemeGuard<'a, MyOAuth2Credential>,
///         ctx: &'a CredentialContext,
///     ) -> Result<(), Self::Error> {
///         let _ = (new_scheme, ctx); // no retention possible
///         Ok(())
///     }
/// }
/// ```
pub trait OnCredentialRefresh<C: Credential>: Send + Sync {
    /// Resource-specific error type for refresh hooks.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Called by the engine when `C` is refreshed.
    ///
    /// `new_scheme` and `ctx` share the lifetime `'a`. The shared
    /// lifetime is the compile-time retention barrier — see Probe 6.
    /// Implementations MUST NOT store either argument past this call.
    ///
    /// # Default
    ///
    /// The default body is a no-op (returns `Ok(())` after dropping
    /// `new_scheme` and `ctx`). Per Tech Spec §15.7 lines 3422-3429:
    /// resources that don't react to credential refresh opt out via
    /// the default rather than implementing an empty body. The wrapped
    /// `SchemeGuard` zeroizes deterministically when the function
    /// returns; no retention is possible across the default path.
    fn on_credential_refresh<'a>(
        &'a self,
        new_scheme: SchemeGuard<'a, C>,
        ctx: &'a CredentialContext,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a {
        async move {
            let _ = new_scheme;
            let _ = ctx;
            Ok(())
        }
    }
}
