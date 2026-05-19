//! Recovery layer — CAS-based gate.
//!
//! The recovery layer prevents thundering herd on dead backends by
//! serializing recovery attempts through [`RecoveryGate`] instances.

pub mod gate;

pub use gate::{GateState, RecoveryGate, RecoveryGateConfig, RecoveryTicket, RecoveryWaiter};
