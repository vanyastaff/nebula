//! Integration tests for nebula-value
//!
//! These tests verify that different modules work together correctly

mod integration {
    mod workflow_scenario;
    mod cross_module;
}

// Re-export for test runner
pub use integration::*;