//! Probe 3 — §15.4 capability sub-traits require method bodies (no
//! silent default).
//!
//! Each sub-fixture writes `impl Refreshable | Revocable | Testable |
//! Dynamic for Dummy {}` without the corresponding method body. The
//! supertrait split removes the defaulted bodies that previously made
//! `const REFRESHABLE = true` + missing `refresh()` silently no-op
//! at runtime — fixtures fail with `E0046` at the impl block.

#[test]
fn compile_fail_capability_subtrait_refreshable_no_method() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/capability_subtrait_refreshable_no_method.rs");
}

#[test]
fn compile_fail_capability_subtrait_revocable_no_method() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/capability_subtrait_revocable_no_method.rs");
}

#[test]
fn compile_fail_capability_subtrait_testable_no_method() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/capability_subtrait_testable_no_method.rs");
}

#[test]
fn compile_fail_capability_subtrait_dynamic_no_method() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/capability_subtrait_dynamic_no_method.rs");
}
