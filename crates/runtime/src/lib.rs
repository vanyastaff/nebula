#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # Nebula Runtime
//!
//! Action execution orchestration for the Nebula workflow engine.
//!
//! This crate provides:
//! - [`ActionRuntime`] -- executes actions through the sandbox with data limits
//! - [`ActionRegistry`] -- registers and looks up action handlers by key
//! - [`DataPassingPolicy`] -- controls output size enforcement
//!
//! The runtime sits between the engine (which schedules work) and the
//! sandbox (which provides isolation). It resolves actions from the
//! registry, enforces data passing policies, and emits telemetry events.

pub mod data_policy;
pub mod error;
pub mod registry;
pub mod runtime;

pub use data_policy::{DataPassingPolicy, LargeDataStrategy};
pub use error::RuntimeError;
pub use registry::ActionRegistry;
pub use runtime::ActionRuntime;
