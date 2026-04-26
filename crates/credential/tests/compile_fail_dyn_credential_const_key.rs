//! Bonus probe (Stage 4) - `dyn Credential` not dyn-compatible.
//!
//! Per Tech Spec 15.4 line 3237 + ADR-0035 context section, the
//! `Credential` trait carries `const KEY: &'static str` plus several
//! associated types (`Input`, `State`, `Scheme`, `Pending`). Any one of
//! those alone is enough to block dyn-compatibility:
//!
//! - `const KEY` triggers `E0038`: "must be possible to call this function on `dyn Credential`" -
//!   dyn-incompatible.
//! - The associated types separately trigger `E0191` / require specification when the trait is used
//!   in a `dyn` position.
//!
//! This probe documents the structural reason the phantom-shim
//! pattern exists (ADR-0035 1): a `dyn Credential` trait object
//! is not constructible at the type level, regardless of any runtime
//! vtable considerations. The phantom pattern resolves the gap by
//! introducing a separate dyn-safe phantom trait (no `const KEY`, no
//! associated types).

#[test]
fn compile_fail_dyn_credential_const_key() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/dyn_credential_const_key.rs");
}
