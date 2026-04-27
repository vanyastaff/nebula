//! Probe 6 analogue driver — `Resource::on_credential_refresh` cannot
//! retain `SchemeGuard<'a, _>` past the engine call.
//!
//! Mirrors the credential-side `compile_fail_scheme_guard_retention`
//! driver at the `Resource` trait layer. The driver passes when the
//! inner probe `tests/probes/resource_refresh_retention.rs` FAILS to
//! compile (which is the desired outcome — the borrow checker must
//! reject any `SchemeGuard<'a, _>` retention attempt).

#[test]
fn compile_fail_resource_refresh_retention() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/resource_refresh_retention.rs");
}
