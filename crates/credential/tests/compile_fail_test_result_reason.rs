//! Provider-controlled failure text must not cross the `Testable` contract.
//!
//! A free-form `reason: String` can contain echoed credentials and would flow
//! into logs and API responses. The payload-free, extensible
//! `TestFailureCode` vocabulary is the only supported failure payload.

#[test]
fn compile_fail_test_result_rejects_free_form_reason() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/test_result_free_form_reason.rs");
}
