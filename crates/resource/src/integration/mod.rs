//! Integration layer — configuration structs for cross-crate features.
//!
//! These types define *what* resilience policies to apply during resource
//! acquisition. The actual pipeline wiring lives in the execution layer
//! and depends on `nebula-resilience`.

pub mod resilience;

pub use resilience::{AcquireCircuitBreakerPreset, AcquireResilience, AcquireRetryConfig};
