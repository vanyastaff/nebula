//! Core traits for resilience patterns

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use super::error::ResilienceError;
use super::result::ResilienceResult;

/// Base trait for all resilience patterns
pub trait ResiliencePattern: Send + Sync {
    /// Pattern name for identification
    fn name(&self) -> &str;

    /// Get pattern metrics
    fn metrics(&self) -> PatternMetrics;

    /// Reset pattern state
    fn reset(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;

    /// Check if pattern is healthy
    fn is_healthy(&self) -> bool {
        self.metrics().error_rate() < 0.5
    }
}

/// Trait for executable operations
pub trait Executable: Send + Sync {
    /// Execute an operation with this pattern
    fn execute<'a, T, F, Fut>(
        &'a self,
        operation: F,
    ) -> Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send + 'a>>
    where
        F: FnOnce() -> Fut + Send + 'a,
        Fut: Future<Output = ResilienceResult<T>> + Send + 'a,
        T: Send + 'a;
}

/// Trait for retryable operations
pub trait Retryable {
    /// Check if the error should trigger a retry
    fn is_retryable(&self) -> bool;

    /// Check if the error is terminal
    fn is_terminal(&self) -> bool;

    /// Get suggested retry delay
    fn retry_delay(&self) -> Option<Duration> {
        None
    }
}

impl Retryable for ResilienceError {
    fn is_retryable(&self) -> bool {
        self.is_retryable()
    }

    fn is_terminal(&self) -> bool {
        self.is_terminal()
    }

    fn retry_delay(&self) -> Option<Duration> {
        self.retry_after()
    }
}

/// Pattern metrics
#[derive(Debug, Clone, Default)]
pub struct PatternMetrics {
    /// Total number of calls
    pub total_calls: u64,
    /// Number of successful calls
    pub successful_calls: u64,
    /// Number of failed calls
    pub failed_calls: u64,
    /// Total latency in milliseconds
    pub total_latency_ms: u64,
    /// Minimum latency
    pub min_latency_ms: u64,
    /// Maximum latency
    pub max_latency_ms: u64,
    /// Pattern-specific metrics
    pub custom: std::collections::HashMap<String, f64>,
}

impl PatternMetrics {
    /// Calculate error rate
    #[must_use] 
    pub fn error_rate(&self) -> f64 {
        if self.total_calls == 0 {
            0.0
        } else {
            self.failed_calls as f64 / self.total_calls as f64
        }
    }

    /// Calculate average latency
    #[must_use] 
    pub fn avg_latency_ms(&self) -> f64 {
        if self.total_calls == 0 {
            0.0
        } else {
            self.total_latency_ms as f64 / self.total_calls as f64
        }
    }

    /// Success rate
    #[must_use] 
    pub fn success_rate(&self) -> f64 {
        1.0 - self.error_rate()
    }
}

/// Health check trait
pub trait HealthCheck: Send + Sync {
    /// Check health status
    fn check_health(&self) -> Pin<Box<dyn Future<Output = HealthStatus> + Send + '_>>;
}

/// Health status
#[derive(Debug, Clone, PartialEq)]
pub enum HealthStatus {
    /// Fully operational
    Healthy,
    /// Degraded performance
    Degraded { reason: String },
    /// Not operational
    Unhealthy { reason: String },
}

/// Circuit breaker states for type safety
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Circuit is closed - operations allowed
    Closed,
    /// Circuit is open - operations blocked
    Open,
    /// Circuit is half-open - limited operations
    HalfOpen,
}

impl fmt::Display for CircuitState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => write!(f, "closed"),
            Self::Open => write!(f, "open"),
            Self::HalfOpen => write!(f, "half-open"),
        }
    }
}
