//! Metrics collection — moved to `nebula-telemetry` / `nebula-metrics`.
//!
//! This module previously re-exported the ecosystem `metrics` crate and provided
//! timing helpers. Those capabilities now live in the dedicated metrics pipeline
//! crates. The module is retained as an empty placeholder for backward
//! compatibility.

mod helpers;
