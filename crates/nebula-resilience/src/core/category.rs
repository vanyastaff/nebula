//! Sealed category traits for resilience patterns
//!
//! This module provides type-safe categorization of resilience patterns
//! using the sealed trait pattern to prevent external implementations.
//!
//! # Pattern Categories
//!
//! - **Flow Control**: Patterns that control operation flow (retry, timeout)
//! - **Protection**: Patterns that protect resources (circuit breaker, bulkhead)
//! - **Rate Limiting**: Patterns that limit operation rate (rate limiter)
//! - **Fallback**: Patterns that provide alternative behavior (fallback, hedge)
//!
//! # Example
//!
//! ```ignore
//! use nebula_resilience::core::category::{FlowControlPattern, ProtectionPattern};
//!
//! fn handle_flow_control<P: FlowControlPattern>(pattern: &P) {
//!     println!("Flow control pattern: {} - {}", P::category(), pattern.description());
//! }
//! ```

use crate::patterns::{bulkhead::Bulkhead, circuit_breaker::CircuitBreaker};

/// Private module to seal the traits
mod sealed {
    pub trait Sealed {}
}

// =============================================================================
// FLOW CONTROL PATTERNS
// =============================================================================

/// Marker trait for patterns that control operation flow.
///
/// Flow control patterns manage how and when operations are executed,
/// including retry logic and timeouts.
///
/// This trait is sealed and cannot be implemented outside this crate.
pub trait FlowControlPattern: sealed::Sealed {
    /// Returns the category name for this pattern type.
    #[must_use]
    fn category() -> &'static str {
        "flow_control"
    }

    /// Returns a human-readable description of this pattern instance.
    fn description(&self) -> &str;

    /// Returns whether this pattern can delay operation execution.
    fn can_delay(&self) -> bool {
        true
    }
}

// =============================================================================
// PROTECTION PATTERNS
// =============================================================================

/// Marker trait for patterns that protect resources.
///
/// Protection patterns prevent resource exhaustion and cascade failures
/// by limiting access to protected resources.
///
/// This trait is sealed and cannot be implemented outside this crate.
pub trait ProtectionPattern: sealed::Sealed {
    /// Returns the category name for this pattern type.
    #[must_use]
    fn category() -> &'static str {
        "protection"
    }

    /// Returns a human-readable description of this pattern instance.
    fn description(&self) -> &str;

    /// Returns whether this pattern can reject operations.
    fn can_reject(&self) -> bool {
        true
    }

    /// Returns the current protection level (0.0 = fully open, 1.0 = fully closed).
    fn protection_level(&self) -> f64;
}

// =============================================================================
// RATE LIMITING PATTERNS
// =============================================================================

/// Marker trait for patterns that limit operation rate.
///
/// Rate limiting patterns control the frequency of operations to prevent
/// overload and ensure fair resource usage.
///
/// This trait is sealed and cannot be implemented outside this crate.
pub trait RateLimitingPattern: sealed::Sealed {
    /// Returns the category name for this pattern type.
    #[must_use]
    fn category() -> &'static str {
        "rate_limiting"
    }

    /// Returns a human-readable description of this pattern instance.
    fn description(&self) -> &str;

    /// Returns the configured rate limit (operations per second).
    fn rate_limit(&self) -> f64;
}

// =============================================================================
// FALLBACK PATTERNS
// =============================================================================

/// Marker trait for patterns that provide fallback behavior.
///
/// Fallback patterns provide alternative behavior when primary operations fail,
/// ensuring graceful degradation.
///
/// This trait is sealed and cannot be implemented outside this crate.
pub trait FallbackPattern: sealed::Sealed {
    /// Returns the category name for this pattern type.
    #[must_use]
    fn category() -> &'static str {
        "fallback"
    }

    /// Returns a human-readable description of this pattern instance.
    fn description(&self) -> &str;
}

// =============================================================================
// IMPLEMENTATIONS
// =============================================================================

// Circuit Breaker - Protection Pattern
impl sealed::Sealed for CircuitBreaker {}

impl ProtectionPattern for CircuitBreaker {
    fn description(&self) -> &'static str {
        "Circuit breaker prevents cascade failures by stopping operations when failure threshold is exceeded"
    }

    fn protection_level(&self) -> f64 {
        // Would need async access to state, return placeholder
        // In practice, check the circuit state
        0.0
    }
}

// Bulkhead - Protection Pattern
impl sealed::Sealed for Bulkhead {}

impl ProtectionPattern for Bulkhead {
    fn description(&self) -> &'static str {
        "Bulkhead limits concurrent operations to prevent resource exhaustion"
    }

    fn protection_level(&self) -> f64 {
        // Approximate based on max concurrency
        // Would need async access for exact calculation
        0.0
    }
}

// Note: RateLimiter implementations would need individual impl blocks
// due to the trait object nature. We implement for concrete types instead.

// ValueFallback - Fallback Pattern
impl<T: Clone + Send + Sync> sealed::Sealed for crate::patterns::fallback::ValueFallback<T> {}

impl<T: Clone + Send + Sync> FallbackPattern for crate::patterns::fallback::ValueFallback<T> {
    fn description(&self) -> &'static str {
        "Value fallback provides a default value when operation fails"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protection_pattern_category() {
        assert_eq!(
            <CircuitBreaker as ProtectionPattern>::category(),
            "protection"
        );
        assert_eq!(<Bulkhead as ProtectionPattern>::category(), "protection");
    }

    #[test]
    fn test_circuit_breaker_description() {
        let cb = CircuitBreaker::with_defaults().expect("Failed to create circuit breaker");
        assert!(cb.description().contains("Circuit breaker"));
        assert!(cb.can_reject());
    }

    #[test]
    fn test_bulkhead_description() {
        let bh = Bulkhead::new(10);
        assert!(bh.description().contains("Bulkhead"));
        assert!(bh.can_reject());
    }
}
