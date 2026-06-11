//! Credential-rotation resource fan-out — moved from `nebula-engine` per ADR-0092 step 5.
//!
//! The fan-out is the only part of the engine's credential subtree that
//! reaches `nebula_resource` types directly (`Manager`, `SlotIdentity`,
//! `RevokeTail`). Because this crate already owns those types, co-locating
//! the fan-out here removes the only cross-crate dependency that motivated
//! keeping it in the engine.
//!
//! The engine remains the **consumer**: it holds the index arc, spawns the
//! driver, and calls `bind` from `ResourceRegistrarRegistry::register_and_bind`.
//! No `nebula-resource → nebula-engine` edge is introduced; the rotation
//! signals flow through `nebula-eventbus`.
//!
//! Gated behind the `rotation` cargo feature so it does not widen the
//! default dependency footprint of `nebula-resource`.

pub mod driver;
pub mod index;

pub use driver::ResourceFanoutDriver;
pub use index::{Bind, ResourceFanoutIndex, RotationOutcome};
