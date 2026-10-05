//! Unsupported test-only conformance observation for the required NS15 producer.
//!
//! Explicit activation retains only operation identities, statuses and fixed
//! finding codes. Request authorities, URI queries and payloads are never saved.

mod observer;
pub mod validation;
mod wire;

pub(crate) use observer::observe_if_enabled;

/// Bounded, payload-free failures of the conformance driver.
#[derive(Debug, thiserror::Error)]
pub enum ConformanceError {
    #[error("served OpenAPI inventory is invalid")]
    InvalidSpec,
    #[error("conformance exclusions are invalid")]
    InvalidExclusions,
    #[error("observation is outside the served inventory")]
    InvalidObservation,
    #[error("conformance observation cannot be retained")]
    ObservationWrite,
}
