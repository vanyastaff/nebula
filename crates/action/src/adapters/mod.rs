//! Adapter implementations bridging typed action traits to [`InternalHandler`](crate::handler::InternalHandler).

pub mod process;
pub mod stateful;
pub mod trigger;

pub use process::ProcessActionAdapter;
pub use stateful::StatefulActionAdapter;
pub use trigger::TriggerActionAdapter;
