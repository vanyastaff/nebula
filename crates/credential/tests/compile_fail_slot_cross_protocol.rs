//! The F3 moat is a compile-time guarantee: binding a credential of one
//! protocol where another protocol's scheme is expected is a nominal type
//! error, not a runtime check. Locks `CredentialGuard<S>`'s nominal typing
//! against a future regression (e.g. type-erasing the slot value).

#[test]
fn compile_fail_slot_cross_protocol() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/slot_cross_protocol.rs");
}
