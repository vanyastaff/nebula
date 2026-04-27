//! Probe: `impl TriggerAction` without `type Source` must fail to compile.
//! Per Tech Spec §2.2.3 line 393 — "without it, `impl TriggerAction for X`
//! produces `error[E0046]: not all trait items implemented, missing: Source`".

#[test]
fn missing_source_fails() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/missing_trigger_source.rs");
}
