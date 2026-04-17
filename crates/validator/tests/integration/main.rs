//! End-to-end integration scenarios for `nebula-validator`.
//!
//! Each sub-module exercises a realistic composition of the crate's
//! features — the programmatic `Validate` trait, the declarative `Rule`
//! enum, the `#[derive(Validator)]` proc-macro, and their interactions —
//! against concrete problem domains (forms, API payloads, serialized
//! schemas, error semantics). These are **scenario** tests, not unit
//! tests: they should fail loudly when a cross-cutting behavior regresses.
//!
//! The folder is organised as a single Cargo test binary
//! (`tests/integration/main.rs` + peer modules) so scenarios can share
//! helpers via `mod common;` without re-duplicating fixtures.

mod common;

mod api_payload;
mod combinator_interop;
mod deep_nesting;
mod derive_form;
mod error_semantics;
mod proof_tokens;
mod rule_roundtrip;
mod unicode_lengths;
