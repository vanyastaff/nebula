//! Resource v2 trait shape — Phase 4 spike validation.
//!
//! This crate is a **playground**, not a future production crate. It exists
//! to prove that the frozen Strategy §3.6 trait shape (`type Credential:
//! Credential` on `Resource`, with `on_credential_refresh` /
//! `on_credential_revoke` hooks) compiles cleanly with the real
//! `nebula-credential` crate, on representative `Resource` impls covering
//! every retained topology.
//!
//! See the cascade docs (`docs/superpowers/specs/2026-04-24-nebula-
//! resource-redesign-strategy.md` §4.1, ADR-0036) for the binding shape
//! contract this code must mirror.
#![forbid(unsafe_code)]

pub mod manager;
pub mod no_credential;
pub mod resource;
pub mod topology;

pub use manager::{DispatchOutcome, Manager, ResourceDispatcher, RotationOutcome};
pub use no_credential::{NoCredential, NoScheme};
pub use resource::{Resource, ResourceContext, ResourceKey};
pub use topology::{Exclusive, Pooled, Resident, Service, Transport};
