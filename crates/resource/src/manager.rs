//! Resource manager placeholder.
//!
//! The full `Manager` implementation arrives in Phase 5 (Tasks 28-33).
//! This module provides a minimal struct so dependent crates compile.

/// Central registry and lifecycle manager for all resources.
///
/// **Note:** This is a placeholder. The full implementation with registration,
/// acquire, hot-reload, and shutdown will be added in a later phase.
#[derive(Debug, Default)]
pub struct Manager {
    _priv: (),
}

impl Manager {
    /// Creates a new (empty) manager.
    pub fn new() -> Self {
        Self::default()
    }
}
