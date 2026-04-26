//! Probe (Stage-8 PR #582 review followup) — capability self-attestation
//! parity check.
//!
//! `#[derive(Credential)]` previously emitted
//! `IsRefreshable::VALUE = true` purely from
//! `#[credential(capabilities(refreshable))]` with no validation that
//! the actual `Refreshable` sub-trait was implemented. A developer who
//! forgot the `impl Refreshable for X` block silently shipped a
//! credential that advertised refresh capability and failed only at
//! engine dispatch — recreating the §15.8 self-attestation
//! anti-pattern that capability-from-type was meant to close.
//!
//! The macro now emits a compile-time parity assertion alongside each
//! `IsX` impl. The fixture declares `capabilities(refreshable)` but
//! never writes `impl Refreshable for X`, and trybuild expects the
//! parity assertion to surface as `the trait bound \`X: Refreshable\`
//! is not satisfied`.

#[test]
fn compile_fail_capabilities_declared_without_subtrait_impl() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/capabilities_declared_without_subtrait_impl.rs");
}
