//! Probe: third-party `IsRefreshable::VALUE = true` lie does not grant
//! access to the engine's `where C: Refreshable` dispatcher (E0277).
//!
//! A hand-rolled credential type can implement `plugin_capability_report::IsRefreshable`
//! with `VALUE = true` without implementing the `Refreshable` sub-trait.
//! This probe confirms that the dispatcher still rejects the type: the engine
//! binds on the *sub-trait* (`where C: Refreshable`), not on the const-bool
//! report. The lie is structurally inert at the dispatch site.
//!
//! Closes security-lead findings N1 + N3 + N5 (Tech Spec §15.4 capability
//! sub-trait split): the engine gate is a trait bound, not a runtime const read.

#[test]
fn compile_fail_sealed_capability_third_party_rejected() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/sealed_capability_third_party.rs");
}
