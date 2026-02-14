//! Dynamic configuration support
//!
//! This module provides runtime configuration through string-based paths
//! as well as compile-time type-safe builder APIs for resilience patterns.

pub mod builder;
pub mod config;

pub use builder::{
    BulkheadConfigBuilder, CircuitBreakerConfigBuilder, DynamicConfigBuilder, RetryConfigBuilder,
};

pub use config::{DynamicConfig, DynamicConfigurable, ResiliencePresets};
