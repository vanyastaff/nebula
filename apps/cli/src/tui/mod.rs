//! Terminal UI for live workflow execution monitoring.
//!
//! Activated via `nebula run --tui`. Shows node graph with live status,
//! current node detail, error panel, and execution log.

#[cfg(feature = "tui")]
pub mod app;
#[cfg(feature = "tui")]
pub mod event;
#[cfg(feature = "tui")]
pub mod render;

#[cfg(feature = "tui")]
pub use app::run_tui;
