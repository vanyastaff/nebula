//! Probe - `dyn Credential` rejected at compile time. See parent
//! driver `compile_fail_dyn_credential_const_key.rs`.
//!
//! `Credential` has `Component` as a supertrait, and `Component` requires
//! `Sized` and carries `const KEY`. Either alone makes the trait
//! dyn-incompatible, so `E0038` fires at every `dyn Credential` site, with or
//! without the associated types specified.
//!
//! This is the structural reason the phantom-shim pattern (capability
//! macro 1) exists: any `dyn`-position use must route through a phantom trait
//! that has neither `const KEY` nor a `Sized` bound.

use nebula_credential::Credential;

fn _take_bare(_c: &dyn Credential) {}

fn main() {}
