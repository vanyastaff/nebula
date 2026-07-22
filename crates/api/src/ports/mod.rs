//! API-owned port traits — the seam between `nebula-api` and lower layers.
//!
//! Ports carry only api-safe types (primitives, `serde_json::Value`,
//! api-owned structs). Concrete impls live in the composition root, which
//! legally depends on the lower-layer crates. This keeps `nebula-api` free
//! of lower-layer types in its DTOs (stub-endpoint policy Cross-Layer Schema Strategy).

#[cfg(any(test, feature = "test-util"))]
pub(crate) mod credential_builder;
pub mod credential_command;
pub mod credential_schema;
#[cfg(any(test, feature = "test-util"))]
pub mod credential_schema_registry;
#[cfg(any(test, feature = "test-util"))]
pub mod credential_service_factory;
pub mod email;
#[cfg(any(test, feature = "test-util"))]
pub mod reqwest_transport;
