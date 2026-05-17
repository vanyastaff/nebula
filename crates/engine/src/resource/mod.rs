//! Engine-side resource registration wiring.
//!
//! This module owns the seam that turns a *stored* resource row (a
//! `kind` string plus an opaque JSON config) into a typed registration
//! against [`nebula_resource::Manager`]. The engine never reflects on
//! the `kind` string nor constructs resource types dynamically: every
//! registrable kind must have been explicitly inserted into the
//! [`ResourceRegistrarRegistry`] ahead of time. An unrecognized kind is
//! a caller/wiring misconfiguration surfaced as a typed error at
//! activation time, never a silent no-op (INTEGRATION_MODEL §114-120;
//! ADR-0030 / ADR-0036 / ADR-0044).

pub mod registrar;

pub use registrar::{
    ErasedResourceRegistrar, RegisterRequest, RegistrarError, ResourceRegistrarRegistry,
    TypedResourceRegistrar,
};
