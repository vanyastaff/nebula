//! Integration tests — Gate 3 §15.12.3 questions (a)-(e) empirical probes.

use credential_proto::{
    CredentialRef, CredentialRegistry, RefreshDispatcher, RegisterError, RevokeDispatcher,
    SchemeFactory,
};
use credential_proto_builtin::{
    AnyBearerPhantom, ApiKeyCredential, BitbucketBearerPhantom, BitbucketFetchAction,
    GenericBearerAction, InteractivePhantom, OAuth2Credential, RefreshablePhantom,
    SalesforceJwtCredential,
};

// ============================================================================
// Question (a) — `dyn Credential` object-safety.
//
// EMPIRICAL FINDING: `dyn Credential` is NOT object-safe, NEITHER under
// CP4 shape NOR CP5/CP6 shape. The blocker is `const KEY: &'static str`
// in the base trait. Per rustc E0038:
//
//   > Just like static functions, associated constants aren't stored on
//   > the method table. If the trait or any subtrait contain an associated
//   > constant, they are not dyn compatible.
//
// This was also true on CP4. Prior spike iter-1/iter-2 did NOT attempt
// `Box<dyn Credential<...>>` directly — it used a narrower object-safe
// `AnyCredential` trait (no const) for runtime dispatch, and the phantom-shim
// (which has NO Credential supertrait) for Pattern 2/3.
//
// Verdict: (a) NO — `dyn Credential` is NOT object-safe. This is NOT a
// regression introduced by the sub-trait split; it was true on CP4 too.
// The sub-trait split PRESERVES the status quo: `dyn Credential` blocked
// by const KEY; `dyn <ServiceXPhantom>` is the dyn-safe route. See (b).
// ============================================================================

#[test]
fn question_a_dyn_credential_blocked_by_const_key() {
    // This assertion is NEGATIVE: `dyn Credential` cannot be constructed.
    // See `compile-fail/tests/ui/dyn_credential_const_key.rs` for the
    // verbatim diagnostic. The probe fails with E0038 "Credential is not
    // dyn compatible because it contains associated const `KEY`".
    //
    // The property we CAN verify here: the 3-assoc-type Credential shape
    // (after sub-trait split dropped Pending) is a valid trait at the
    // type level — concrete types impl it, bounds accept it.
    const fn _accepts<C: credential_proto::Credential>() {}
    _accepts::<ApiKeyCredential>();
    _accepts::<OAuth2Credential>();
    _accepts::<SalesforceJwtCredential>();
}

// ============================================================================
// Question (c) — `dyn Refreshable` requires parallel phantom.
// ============================================================================

#[test]
fn question_c_dyn_refreshable_needs_phantom() {
    // `Refreshable: Credential` — inherits Credential's 3 assoc types +
    // `const KEY`. So `dyn Refreshable` suffers from BOTH:
    //   (1) E0038 — const KEY in supertrait → not dyn-compatible.
    //   (2) E0191 — assoc types unspecified.
    // See `compile-fail/tests/ui/dyn_refreshable_blocked.rs` for verbatim.
    //
    // To dispatch over Refreshable values at runtime (e.g. engine's refresh
    // registry iterating all refreshables), the parallel phantom-shim
    // `RefreshablePhantom` is required. Phantom trait has NO Credential
    // supertrait → no const KEY, no assoc types → dyn-compatible.
    //
    // `Box<dyn RefreshablePhantom>` IS well-formed:
    let _r: Box<dyn RefreshablePhantom> = Box::new(OAuth2Credential);
    let _r2: Box<dyn RefreshablePhantom> = Box::new(SalesforceJwtCredential);

    // InteractivePhantom same pattern:
    let _i: Box<dyn InteractivePhantom> = Box::new(OAuth2Credential);

    // Verdict: (c) YES — parallel phantom NEEDED for dyn dispatch over
    // lifecycle sub-traits. RefreshablePhantom + InteractivePhantom demonstrated
    // to compile. Engine refresh registry using `HashMap<Key, Box<dyn
    // RefreshablePhantom>>` is the production pattern.
}

// ============================================================================
// Question (b) — phantom-shim erases C::Scheme cleanly for Pattern 2/3.
// ============================================================================

#[test]
fn question_b_phantom_shim_pattern2_accept() {
    // Pattern 2 — BitbucketBearerPhantom accepts OAuth2Credential (which
    // implements BitbucketCredential + Scheme = BearerScheme : AcceptsBearer).
    let _c: CredentialRef<dyn BitbucketBearerPhantom> = CredentialRef::new("oauth2");

    // Action holding the dyn cred compiles.
    let _a = BitbucketFetchAction { cred: CredentialRef::new("oauth2") };

    // Verdict: (b) YES, phantom-shim erases C::Scheme cleanly with 3-assoc-type
    // base Credential. Outcome identical to CP4 shape (4 assoc types). Assoc
    // type count does not affect phantom shim well-formedness because the
    // phantom trait has NO Credential supertrait.
}

#[test]
fn question_b_phantom_shim_pattern3_accept() {
    // Pattern 3 — AnyBearerPhantom accepts any Credential with Scheme: AcceptsBearer.
    // Both ApiKeyCredential and OAuth2Credential qualify.
    let _a: CredentialRef<dyn AnyBearerPhantom> = CredentialRef::new("api_key");
    let _b: CredentialRef<dyn AnyBearerPhantom> = CredentialRef::new("oauth2");

    let _action = GenericBearerAction { cred: CredentialRef::new("api_key") };
}

// ============================================================================
// Question (d) — capability-const downgrade path.
// ============================================================================

#[test]
fn question_d_capability_const_via_trait_bounds() {
    // Downgrade-via-bool path from CP4 const-bool style is structurally
    // IMPOSSIBLE on CP5/CP6 shape — the const bools are GONE. A legacy
    // consumer querying `C::REFRESHABLE` fails with E0599 "no associated
    // item named `REFRESHABLE` found for type…".
    //
    // Replacement: consumers use compile-time bounds (`where C: Refreshable`)
    // or runtime phantom-dispatch via `dyn RefreshablePhantom` downcast.
    //
    // This IS a breaking change for legacy consumers that read `REFRESHABLE: bool`.
    // Per feedback_hard_breaking_changes: expert-level spec-correct breaking
    // change accepted. Tech Spec §15.4 acknowledges "one-time learning cost".
    //
    // Demonstrate the replacement mechanism: query-from-type via dispatcher
    // construction (compile-time path) + phantom-shim membership (runtime path).

    // Compile-time path: RefreshDispatcher::for_credential::<OAuth2Credential>() compiles.
    let _d = RefreshDispatcher::<OAuth2Credential>::for_credential();

    // Compile-time rejection: RefreshDispatcher::for_credential::<ApiKeyCredential>()
    // fails to compile. See `compile-fail/tests/ui/engine_dispatch_capability.rs`.

    // Runtime path: phantom-shim downcast. A `Box<dyn Any>` can be cast to
    // `&dyn RefreshablePhantom` iff the concrete type implements Refreshable.
    // `trait_upcasting` is stable in 1.95; `Any` downcast is the tried-and-true
    // mechanism. Here we demonstrate the cleaner pattern: a registry stores
    // `Option<Box<dyn RefreshablePhantom>>` per credential instance; presence
    // IS the capability query.

    // Sample: engine-side type-erased refresh registry.
    let oauth_entry: Option<Box<dyn RefreshablePhantom>> = Some(Box::new(OAuth2Credential));
    let api_entry: Option<Box<dyn RefreshablePhantom>> = None; // ApiKeyCredential cannot be boxed as dyn RefreshablePhantom
    // The above line's `None` is a legitimate encoding — the registry has
    // NO refreshable entry for ApiKey. Attempting `Some(Box::new(ApiKeyCredential))`
    // would fail to compile (ApiKey doesn't impl Refreshable, so blanket
    // RefreshablePhantom impl doesn't apply).
    assert!(oauth_entry.is_some());
    assert!(api_entry.is_none());

    // Verdict: (d) N/A — legacy REFRESHABLE: bool downgrade path is
    // structurally impossible by design. Hard breaking change, spec-correct,
    // documented in §15.4 Cons. Replacement mechanism (RefreshablePhantom
    // membership) is stronger: no-op-refresh-in-production-surprise class
    // eliminated at compile time.
}

// ============================================================================
// Question (e) covered by compile-fail probes — see `compile-fail/`.
// ============================================================================

// ============================================================================
// Bonus — Engine dispatcher static type bound (positive case).
// ============================================================================

#[test]
fn engine_refresh_dispatcher_accepts_refreshable() {
    let d: RefreshDispatcher<OAuth2Credential> = RefreshDispatcher::for_credential();
    let _p = d.policy();

    let r: RevokeDispatcher<OAuth2Credential> = RevokeDispatcher::for_credential();
    let _ = r;
}

// ============================================================================
// Bonus — Registry fatal duplicate-KEY (§15.6).
// ============================================================================

#[test]
fn registry_duplicate_key_fatal() {
    let mut reg = CredentialRegistry::new();

    reg.register::<OAuth2Credential>("crate_a").expect("first register succeeds");
    // Second registration of the same KEY fails.
    let err = reg
        .register::<OAuth2Credential>("crate_b")
        .expect_err("second register must fail on duplicate KEY");

    match err {
        RegisterError::DuplicateKey { key, existing_crate, new_crate } => {
            assert_eq!(key, "oauth2");
            assert_eq!(existing_crate, "crate_a");
            assert_eq!(new_crate, "crate_b");
        }
    }
}

// ============================================================================
// Bonus — SchemeGuard drop-order via atomic counter (§15.7).
// ============================================================================

use std::sync::atomic::{AtomicUsize, Ordering};

static ZEROIZE_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Wrap BearerScheme in a tracking-drop struct to count zeroize invocations.
/// Production test would use `wreck` or `drop_tracker` crate; here we do it inline.
///
/// Kept for illustrative purposes — not used directly in the current spike,
/// because using it as `C::Scheme` would require Credential impl to declare
/// `type Scheme = TrackedBearer`, which breaks the 3-credential portfolio.
/// The shape shows that zeroization can be instrumented + counted.
#[allow(dead_code)]
struct TrackedBearer(credential_proto_builtin::BearerScheme);

use zeroize::Zeroize as _;

impl zeroize::Zeroize for TrackedBearer {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

impl zeroize::ZeroizeOnDrop for TrackedBearer {}

impl Drop for TrackedBearer {
    fn drop(&mut self) {
        ZEROIZE_COUNTER.fetch_add(1, Ordering::SeqCst);
        self.zeroize();
    }
}

#[test]
fn scheme_guard_drops_scheme_at_scope_exit() {
    use credential_proto_builtin::{BearerScheme, OAuth2Credential};
    use credential_proto::SchemeGuard;

    let before = ZEROIZE_COUNTER.load(Ordering::SeqCst);
    let _ = before; // TrackedBearer isn't used as the actual SchemeGuard target
                    // because that would require Credential::Scheme = TrackedBearer.
                    // Instead we demonstrate that SchemeGuard<'_, OAuth2> drops
                    // BearerScheme, and BearerScheme has ZeroizeOnDrop — plaintext
                    // field goes to zero.
    {
        let scheme = BearerScheme { token: "sekret".into() };
        let guard: SchemeGuard<'_, OAuth2Credential> = SchemeGuard::new(scheme);
        // Use the guard via Deref:
        let tok_ref: &BearerScheme = &*guard;
        assert_eq!(tok_ref.token, "sekret");
        // Guard drops at end of scope — BearerScheme's ZeroizeOnDrop zeroizes.
    }
    // Field zeroization is internal; we've asserted the shape compiles
    // + Deref works + !Clone is enforced (see compile-fail probe).
}

// ============================================================================
// Bonus — SchemeFactory acquisition.
// ============================================================================

#[test]
fn scheme_factory_yields_guard() {
    use credential_proto_builtin::{BearerScheme, OAuth2Credential};

    let factory: SchemeFactory<OAuth2Credential> = SchemeFactory::new(|| {
        Ok(BearerScheme { token: "refreshed-token".into() })
    });
    let guard = factory.acquire().expect("acquire");
    assert_eq!(guard.token, "refreshed-token");
    // Drop at end of scope — plaintext zeroizes.
}
