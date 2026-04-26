//! Probe 4 — §15.4: engine dispatcher bound on `Refreshable` rejects
//! a non-`Refreshable` credential at the dispatch site (`E0277`).
//!
//! The probe uses a stub dispatcher (mirroring
//! `nebula_engine::credential::CredentialResolver::resolve_with_refresh`'s
//! `where C: Refreshable` bound) to keep `nebula-engine` out of the
//! `nebula-credential` dev-deps. The structural guarantee verified
//! here applies identically to the engine's real dispatcher.

#[test]
fn compile_fail_engine_dispatch_capability() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/engine_dispatch_non_refreshable.rs");
}
