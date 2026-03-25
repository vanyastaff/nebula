//! Recovery layer — CAS-based gate and group registry.
//!
//! The recovery layer prevents thundering herd on dead backends by
//! serializing recovery attempts through [`RecoveryGate`] instances,
//! managed per-key by [`RecoveryGroupRegistry`].

pub mod gate;
pub mod group;
pub mod watchdog;

pub use gate::{GateState, RecoveryGate, RecoveryGateConfig, RecoveryTicket, RecoveryWaiter};
pub use group::{RecoveryGroupKey, RecoveryGroupRegistry};
pub use watchdog::{WatchdogConfig, WatchdogHandle};
