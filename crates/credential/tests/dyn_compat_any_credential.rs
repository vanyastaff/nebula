//! Probe: `dyn AnyCredential` and `Arc<dyn AnyCredential>` remain
//! dyn-compatible under the Rust 1.95 next-generation trait solver.
//!
//! Why: 1.95 tightened dyn-compat (formerly "object safety"). The
//! plugin registry holds `Arc<dyn AnyCredential>` per
//! `CredentialRegistry::iter_compatible(...)`; if a trait method gains
//! an associated const, `Self: Sized` requirement on a non-Sized
//! method, or other dyn-incompat feature, plugin loading breaks
//! silently at the registry's trait-object construction.
//!
//! This file compiles iff the trait remains dyn-safe. The body asserts
//! no runtime behaviour — the compile is the assertion.

use std::sync::Arc;

use nebula_credential::AnyCredential;

#[test]
fn dyn_any_credential_compiles() {
    // The mere existence of these type aliases / function signatures
    // is the probe. If `AnyCredential` becomes dyn-incompatible, the
    // file fails to compile.
    fn accepts_ref(_: &dyn AnyCredential) {}
    fn accepts_arc(_: Arc<dyn AnyCredential>) {}
    fn accepts_box(_: Box<dyn AnyCredential>) {}

    let _ = accepts_ref;
    let _ = accepts_arc;
    let _ = accepts_box;
}

#[test]
fn dyn_any_credential_send_sync() {
    // Plugin registry stores `Arc<dyn AnyCredential>` and shares it
    // across tasks; the trait object must be `Send + Sync` for that
    // to compile in practice.
    //
    // `AnyCredential` is declared as `pub trait AnyCredential: Any +
    // Send + Sync + 'static`, so the trait object inherits those bounds.
    fn requires_send_sync<T: Send + Sync + ?Sized>() {}
    requires_send_sync::<dyn AnyCredential>();
}
