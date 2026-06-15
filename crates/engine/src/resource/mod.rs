//! Engine-side resource registration wiring.
//!
//! `ResourceFactory`, `KindActivator`, `RegisterRequest`, `RegistrarError`,
//! `ResourceActivatorRegistry`, and `ResourceRegistrationOutcome` now live in
//! `nebula_resource::factory` (ADR-0095 D2 — moved down into nebula-resource).
//! The engine re-exports them here so engine-internal code reaches them through
//! the existing `crate::resource::*` path without any import changes.
//!
//! The old engine-owned `ResourceActivator` trait name is retired — callers
//! use [`ResourceFactory`].

pub use nebula_resource::{
    KindActivator, RegisterRequest, RegistrarError, ResourceActivatorRegistry, ResourceFactory,
    ResourceRegistrationOutcome,
};
