//! Compile-fail probes — the *structural* halves of two spec §6 abuse
//! invariants that cannot be proven by a runtime assertion:
//!
//! - `raw_store_without_layers.rs` (§6 #7): `CredentialService` is
//!   unconstructable without the builder's secure layered composition —
//!   `__from_parts` is `pub(crate)` and every field is private, so an
//!   external caller cannot smuggle a raw, unencrypted store in.
//! - `snapshot_not_serialize.rs` (§6 #3): `CredentialSnapshot` does not
//!   implement `Serialize`, so a secret-bearing projection can never be
//!   put on the wire by serde.
//!
//! Each probe's `.stderr` was generated once with `TRYBUILD=overwrite`
//! and inspected to confirm the failure is the *intended* privacy /
//! missing-trait error (E0603/E0616/E0624 / unsatisfied `Serialize`),
//! not an incidental unrelated error.

#[test]
fn compile_fail() {
    trybuild::TestCases::new().compile_fail("tests/compile_fail/*.rs");
}
