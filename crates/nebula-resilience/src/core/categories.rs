//! Unified categories for resilience patterns and services
//!
//! This module consolidates all category definitions to eliminate duplication
//! and provide a single source of truth for categorization.

use std::time::Duration;

// =============================================================================
// SEALED TRAITS
// =============================================================================

mod sealed {
    pub trait SealedCategory {}
    pub trait SealedServiceCategory {}
    pub trait SealedPatternCategory {}
}

// =============================================================================
// BASE CATEGORY TRAITS
// =============================================================================

/// Base category trait for all resilience components
pub trait Category: sealed::SealedCategory + Send + Sync + 'static {
    /// Category name
    fn name() -> &'static str;

    /// Category description
    fn description() -> &'static str;
}

/// Service category for grouping related services
pub trait ServiceCategory: Category + sealed::SealedServiceCategory {
    /// Default timeout for this service category
    fn default_timeout() -> Duration {
        Duration::from_secs(30)
    }

    /// Default retry attempts for this service category
    fn default_retry_attempts() -> usize {
        3
    }

    /// Default circuit breaker threshold for this service category
    fn default_failure_threshold() -> usize {
        5
    }

    /// Whether services in this category are critical
    fn is_critical() -> bool {
        false
    }
}

/// Pattern category for grouping resilience patterns
pub trait PatternCategory: Category + sealed::SealedPatternCategory {
    /// Pattern execution order priority (lower = earlier)
    fn execution_order() -> u8;
}

// =============================================================================
// SERVICE CATEGORIES
// =============================================================================

/// Database service category
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Database;

impl sealed::SealedCategory for Database {}
impl sealed::SealedServiceCategory for Database {}

impl Category for Database {
    fn name() -> &'static str {
        "database"
    }

    fn description() -> &'static str {
        "Database and persistent storage services"
    }
}

impl ServiceCategory for Database {
    fn default_timeout() -> Duration {
        Duration::from_secs(5)
    }

    fn default_retry_attempts() -> usize {
        2
    }

    fn default_failure_threshold() -> usize {
        3
    }

    fn is_critical() -> bool {
        true
    }
}

/// HTTP/API service category
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Http;

impl sealed::SealedCategory for Http {}
impl sealed::SealedServiceCategory for Http {}

impl Category for Http {
    fn name() -> &'static str {
        "http"
    }

    fn description() -> &'static str {
        "HTTP APIs and web services"
    }
}

impl ServiceCategory for Http {
    fn default_timeout() -> Duration {
        Duration::from_secs(10)
    }

    fn default_retry_attempts() -> usize {
        3
    }

    fn default_failure_threshold() -> usize {
        5
    }

    fn is_critical() -> bool {
        false
    }
}

/// Message queue service category
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageQueue;

impl sealed::SealedCategory for MessageQueue {}
impl sealed::SealedServiceCategory for MessageQueue {}

impl Category for MessageQueue {
    fn name() -> &'static str {
        "message_queue"
    }

    fn description() -> &'static str {
        "Message queues and event streaming"
    }
}

impl ServiceCategory for MessageQueue {
    fn default_timeout() -> Duration {
        Duration::from_secs(15)
    }

    fn default_retry_attempts() -> usize {
        5
    }

    fn default_failure_threshold() -> usize {
        10
    }

    fn is_critical() -> bool {
        true
    }
}

/// Cache service category
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cache;

impl sealed::SealedCategory for Cache {}
impl sealed::SealedServiceCategory for Cache {}

impl Category for Cache {
    fn name() -> &'static str {
        "cache"
    }

    fn description() -> &'static str {
        "Caching and temporary storage"
    }
}

impl ServiceCategory for Cache {
    fn default_timeout() -> Duration {
        Duration::from_millis(500)
    }

    fn default_retry_attempts() -> usize {
        1
    }

    fn default_failure_threshold() -> usize {
        2
    }

    fn is_critical() -> bool {
        false
    }
}

/// Generic uncategorized service category
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Generic;

impl sealed::SealedCategory for Generic {}
impl sealed::SealedServiceCategory for Generic {}

impl Category for Generic {
    fn name() -> &'static str {
        "generic"
    }

    fn description() -> &'static str {
        "Generic uncategorized service"
    }
}

impl ServiceCategory for Generic {}

// =============================================================================
// PATTERN CATEGORIES
// =============================================================================

/// Protection pattern category (circuit breakers, bulkheads)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Protection;

impl sealed::SealedCategory for Protection {}
impl sealed::SealedPatternCategory for Protection {}

impl Category for Protection {
    fn name() -> &'static str {
        "protection"
    }

    fn description() -> &'static str {
        "Protection patterns (circuit breakers, bulkheads)"
    }
}

impl PatternCategory for Protection {
    fn execution_order() -> u8 {
        10
    }
}

/// Flow control pattern category (rate limiting, throttling)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlowControl;

impl sealed::SealedCategory for FlowControl {}
impl sealed::SealedPatternCategory for FlowControl {}

impl Category for FlowControl {
    fn name() -> &'static str {
        "flow_control"
    }

    fn description() -> &'static str {
        "Flow control patterns (rate limiting, throttling)"
    }
}

impl PatternCategory for FlowControl {
    fn execution_order() -> u8 {
        5
    }
}

/// Fallback pattern category
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fallback;

impl sealed::SealedCategory for Fallback {}
impl sealed::SealedPatternCategory for Fallback {}

impl Category for Fallback {
    fn name() -> &'static str {
        "fallback"
    }

    fn description() -> &'static str {
        "Fallback and graceful degradation patterns"
    }
}

impl PatternCategory for Fallback {
    fn execution_order() -> u8 {
        20
    }
}

/// Retry pattern category
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retry;

impl sealed::SealedCategory for Retry {}
impl sealed::SealedPatternCategory for Retry {}

impl Category for Retry {
    fn name() -> &'static str {
        "retry"
    }

    fn description() -> &'static str {
        "Retry and backoff patterns"
    }
}

impl PatternCategory for Retry {
    fn execution_order() -> u8 {
        15
    }
}

/// Timeout pattern category
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeout;

impl sealed::SealedCategory for Timeout {}
impl sealed::SealedPatternCategory for Timeout {}

impl Category for Timeout {
    fn name() -> &'static str {
        "timeout"
    }

    fn description() -> &'static str {
        "Timeout and deadline patterns"
    }
}

impl PatternCategory for Timeout {
    fn execution_order() -> u8 {
        1 // Timeouts should be outermost
    }
}

// =============================================================================
// CONVENIENCE TYPE ALIASES
// =============================================================================

/// Common service categories
pub mod service {
    pub use super::{Cache, Database, Generic, Http, MessageQueue};
}

/// Common pattern categories
pub mod pattern {
    pub use super::{Fallback, FlowControl, Protection, Retry, Timeout};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_categories() {
        assert_eq!(Database::name(), "database");
        assert_eq!(Http::name(), "http");
        assert_eq!(Cache::name(), "cache");

        assert!(Database::is_critical());
        assert!(!Http::is_critical());
        assert!(!Cache::is_critical());

        assert_eq!(Database::default_timeout(), Duration::from_secs(5));
        assert_eq!(Http::default_timeout(), Duration::from_secs(10));
        assert_eq!(Cache::default_timeout(), Duration::from_millis(500));
    }

    #[test]
    fn test_pattern_categories() {
        assert_eq!(Timeout::execution_order(), 1);
        assert_eq!(FlowControl::execution_order(), 5);
        assert_eq!(Protection::execution_order(), 10);
        assert_eq!(Retry::execution_order(), 15);
        assert_eq!(Fallback::execution_order(), 20);
    }

    #[test]
    fn test_category_descriptions() {
        assert!(Database::description().contains("Database"));
        assert!(Http::description().contains("HTTP"));
        assert!(Protection::description().contains("Protection"));
        assert!(FlowControl::description().contains("Flow control"));
    }
}
