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
//!
//! On a strict manager (a credential availability observer configured) the
//! fan-out is a cache and cleanup role: it installs advanced material, taints
//! and revokes, and suspends or reopens rows it observes, but the safety of a
//! new call is decided by that call's own per-acquire availability read. Its
//! 30 s reconciliation interval is therefore not a safety parameter there;
//! on an interim manager it still bounds how late a denial is observed.

pub mod driver;
pub mod index;
mod orchestrator;

pub use driver::{ResourceFanoutDriver, ResourceFanoutSpawnError};
pub use index::{Bind, ResourceFanoutIndex, RotationOutcome};

#[cfg(test)]
mod suspension_tests;
