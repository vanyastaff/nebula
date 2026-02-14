//! Adapter implementations bridging typed action traits to [`InternalHandler`](crate::handler::InternalHandler).

pub mod process;

pub use process::ProcessActionAdapter;
