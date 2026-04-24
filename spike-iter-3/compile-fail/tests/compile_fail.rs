//! Compile-fail harness — runs trybuild against `tests/ui/*.rs` probes.
//!
//! Each probe in `tests/ui/` has a `.rs` source and a `.stderr` expected
//! diagnostic snapshot. trybuild verifies the diagnostic matches.
//!
//! Per §16.1.1 + Gate 3 §15.12.3(e), the spike includes 4 minimum probes:
//!   1. compile_fail_state_zeroize — CredentialState without ZeroizeOnDrop → E0277
//!   2. compile_fail_capability_subtrait — impl Refreshable without refresh() → E0046
//!   3. compile_fail_engine_dispatch_capability — RefreshDispatcher::for_credential::<ApiKey>() → E0277
//!   4. compile_fail_scheme_guard_retention — SchemeGuard stored in field → E0597
//!
//! Plus bonus:
//!   5. compile_fail_scheme_guard_clone — guard.clone() → E0599
//!   6. compile_fail_pattern2_reject — Pattern 2 rejects non-service credentials → E0277

#[test]
fn compile_fail_probes() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
