//! Seam (ADR-0052 P3) — `Credential::properties_schema()` is removed.
//!
//! Schema is reachable only via the `Properties: HasSchema` associated-type
//! bound / `nebula_schema::schema_of`. This locks the convergence: the
//! redundant per-trait schema method does not resolve.

#[test]
fn compile_fail_credential_properties_schema_removed() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/credential_properties_schema_removed.rs");
}
