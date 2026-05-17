//! API-owned port traits — the seam between `nebula-api` and lower layers.
//!
//! Ports carry only api-safe types (primitives, `serde_json::Value`,
//! api-owned structs). Concrete impls live in the composition root, which
//! legally depends on the lower-layer crates. This keeps `nebula-api` free
//! of lower-layer types in its DTOs (ADR-0047 Cross-Layer Schema Strategy).

pub mod credential_schema;
