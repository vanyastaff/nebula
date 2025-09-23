//! Circuit breaker pattern for automatic failure detection and recovery

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use nebula_log::{debug, info, warn, error};

use crate::{ResilienceError, ResilienceResult, ResilienceConfig, ConfigResult, ConfigError};
use serde::{Deserialize, Serialize};

/// Circuit breaker states
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CircuitState {
    /// Circuit is closed - operations are allowed
    Closed,
    /// Circuit is open - operations are blocked
    Open,
    /// Circuit is half-open - limited operations allowed for testing
    HalfOpen,
}

impl std::fmt::Display for CircuitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => write!(f, "closed"),
            Self::Open => write!(f, "open"),
            Self::HalfOpen => write!(f, "half-open"),
        }
    }
}

/// Circuit breaker configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    /// Number of failures before opening the circuit
    pub failure_threshold: usize,
    /// Time to wait before attempting to close the circuit
    pub reset_timeout: Duration,
    /// Maximum number of operations allowed in half-open state
    pub half_open_max_operations: usize,
    /// Whether to count timeouts as failures
    pub count_timeouts: bool,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            reset_timeout: Duration::from_secs(60),
            half_open_max_operations: 3,
            count_timeouts: true,
        }
    }
}

impl ResilienceConfig for CircuitBreakerConfig {
    fn validate(&self) -> ConfigResult<()> {
        if self.failure_threshold == 0 {
            return Err(ConfigError::validation("failure_threshold must be greater than 0"));
        }

        if self.reset_timeout.as_millis() == 0 {
            return Err(ConfigError::validation("reset_timeout must be greater than 0"));
        }

        if self.half_open_max_operations == 0 {
            return Err(ConfigError::validation("half_open_max_operations must be greater than 0"));
        }

        Ok(())
    }

    fn default_config() -> Self {
        Self::default()
    }

    fn merge(&mut self, other: Self) {
        self.failure_threshold = other.failure_threshold;
        self.reset_timeout = other.reset_timeout;
        self.half_open_max_operations = other.half_open_max_operations;
        self.count_timeouts = other.count_timeouts;
    }
}

/// Circuit breaker implementation
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: Arc<RwLock<CircuitState>>,
    failure_count: Arc<RwLock<usize>>,
    last_failure_time: Arc<RwLock<Option<Instant>>>,
    half_open_operations: Arc<RwLock<usize>>,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with default configuration
    #[must_use] pub fn new() -> Self {
        Self::with_config(CircuitBreakerConfig::default())
    }

    /// Create a new circuit breaker with custom configuration
    #[must_use] pub fn with_config(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: Arc::new(RwLock::new(CircuitState::Closed)),
            failure_count: Arc::new(RwLock::new(0)),
            last_failure_time: Arc::new(RwLock::new(None)),
            half_open_operations: Arc::new(RwLock::new(0)),
        }
    }

    /// Get the current circuit state
    pub async fn state(&self) -> CircuitState {
        self.state.read().await.clone()
    }

    /// Check if the circuit is closed (operations allowed)
    pub async fn is_closed(&self) -> bool {
        matches!(self.state().await, CircuitState::Closed)
    }

    /// Check if the circuit is open (operations blocked)
    pub async fn is_open(&self) -> bool {
        matches!(self.state().await, CircuitState::Open)
    }

    /// Check if the circuit is half-open (limited operations)
    pub async fn is_half_open(&self) -> bool {
        matches!(self.state().await, CircuitState::HalfOpen)
    }

    /// Record a successful operation
    pub async fn record_success(&self) {
        let mut state = self.state.write().await;
        let mut failure_count = self.failure_count.write().await;
        let mut half_open_operations = self.half_open_operations.write().await;

        match *state {
            CircuitState::Closed => {
                // Reset failure count on success
                *failure_count = 0;
                debug!(
                    state = %CircuitState::Closed,
                    action = "success_recorded",
                    failure_count = 0,
                    "Circuit breaker success in closed state"
                );
            }
            CircuitState::HalfOpen => {
                // Success in half-open state, close the circuit
                *state = CircuitState::Closed;
                *failure_count = 0;
                *half_open_operations = 0;
                info!(
                    state_transition = %format!("{} -> {}", CircuitState::HalfOpen, CircuitState::Closed),
                    action = "circuit_closed",
                    failure_count = 0,
                    half_open_operations = 0,
                    "Circuit breaker closed after successful half-open operation"
                );
            }
            CircuitState::Open => {
                // Circuit is open, no state change
                debug!(
                    state = %CircuitState::Open,
                    action = "success_ignored",
                    "Circuit breaker success ignored in open state"
                );
            }
        }
    }

    /// Record a failed operation
    pub async fn record_failure(&self) {
        let mut state = self.state.write().await;
        let mut failure_count = self.failure_count.write().await;
        let mut last_failure_time = self.last_failure_time.write().await;

        match *state {
            CircuitState::Closed => {
                *failure_count += 1;
                *last_failure_time = Some(Instant::now());

                if *failure_count >= self.config.failure_threshold {
                    *state = CircuitState::Open;
                    warn!(
                        state_transition = %format!("{} -> {}", CircuitState::Closed, CircuitState::Open),
                        action = "circuit_opened",
                        failure_count = %failure_count,
                        threshold = self.config.failure_threshold,
                        "Circuit breaker opened due to failure threshold"
                    );
                } else {
                    debug!(
                        state = %CircuitState::Closed,
                        action = "failure_recorded",
                        failure_count = %failure_count,
                        threshold = self.config.failure_threshold,
                        "Circuit breaker failure recorded in closed state"
                    );
                }
            }
            CircuitState::HalfOpen => {
                // Failure in half-open state, open the circuit
                *state = CircuitState::Open;
                *failure_count += 1;
                *last_failure_time = Some(Instant::now());
                warn!("Circuit opened after failure in half-open state");
            }
            CircuitState::Open => {
                // Circuit is already open, just update failure count
                *failure_count += 1;
                *last_failure_time = Some(Instant::now());
                debug!("Circuit open - failure recorded");
            }
        }
    }

    /// Check if an operation should be allowed
    pub async fn can_execute(&self) -> ResilienceResult<()> {
        let mut state = self.state.write().await;
        let mut half_open_operations = self.half_open_operations.write().await;

        match *state {
            CircuitState::Closed => {
                // Operations are always allowed in closed state
                Ok(())
            }
            CircuitState::Open => {
                // Check if reset timeout has elapsed
                let last_failure_time = *self.last_failure_time.read().await;
                if let Some(last_failure) = last_failure_time {
                    if last_failure.elapsed() >= self.config.reset_timeout {
                        // Time to try half-open
                        *state = CircuitState::HalfOpen;
                        *half_open_operations = 0;
                        info!("Circuit transitioning to half-open state");
                        Ok(())
                    } else {
                        Err(ResilienceError::circuit_breaker_open("open"))
                    }
                } else {
                    Err(ResilienceError::circuit_breaker_open("open"))
                }
            }
            CircuitState::HalfOpen => {
                // Check if we can allow more operations
                if *half_open_operations < self.config.half_open_max_operations {
                    *half_open_operations += 1;
                    debug!(
                        "Half-open operation allowed ({}/{})",
                        *half_open_operations, self.config.half_open_max_operations
                    );
                    Ok(())
                } else {
                    Err(ResilienceError::circuit_breaker_open(
                        "half-open limit reached",
                    ))
                }
            }
        }
    }

    /// Execute an operation with circuit breaker protection
    pub async fn execute<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ResilienceResult<T>>,
    {
        // Check if we can execute
        self.can_execute().await?;

        // Execute the operation
        let result = operation().await;

        // Record the result
        match &result {
            Ok(_) => {
                self.record_success().await;
            }
            Err(error) => {
                if self.config.count_timeouts || !matches!(error, ResilienceError::Timeout { .. }) {
                    self.record_failure().await;
                }
            }
        }

        result
    }

    /// Reset the circuit breaker to closed state
    pub async fn reset(&self) {
        let mut state = self.state.write().await;
        let mut failure_count = self.failure_count.write().await;
        let mut half_open_operations = self.half_open_operations.write().await;

        *state = CircuitState::Closed;
        *failure_count = 0;
        *half_open_operations = 0;
        info!("Circuit breaker manually reset to closed state");
    }

    /// Get circuit breaker statistics
    pub async fn stats(&self) -> CircuitBreakerStats {
        CircuitBreakerStats {
            state: self.state().await,
            failure_count: *self.failure_count.read().await,
            last_failure_time: *self.last_failure_time.read().await,
            half_open_operations: *self.half_open_operations.read().await,
        }
    }
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

/// Circuit breaker statistics
#[derive(Debug, Clone)]
pub struct CircuitBreakerStats {
    /// Current circuit state
    pub state: CircuitState,
    /// Total failure count
    pub failure_count: usize,
    /// Time of last failure
    pub last_failure_time: Option<Instant>,
    /// Current operations in half-open state
    pub half_open_operations: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_circuit_breaker_default_state() {
        let cb = CircuitBreaker::new();
        assert!(cb.is_closed().await);
        assert!(!cb.is_open().await);
        assert!(!cb.is_half_open().await);
    }

    #[tokio::test]
    async fn test_circuit_breaker_failure_threshold() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            reset_timeout: Duration::from_millis(100),
            half_open_max_operations: 2,
            count_timeouts: true,
        };
        let cb = CircuitBreaker::with_config(config);

        // Should be closed initially
        assert!(cb.is_closed().await);

        // Record 2 failures - should still be closed
        cb.record_failure().await;
        cb.record_failure().await;
        assert!(cb.is_closed().await);

        // Record 3rd failure - should open
        cb.record_failure().await;
        assert!(cb.is_open().await);
    }

    #[tokio::test]
    async fn test_circuit_breaker_half_open_transition() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            reset_timeout: Duration::from_millis(10),
            half_open_max_operations: 2,
            count_timeouts: true,
        };
        let cb = CircuitBreaker::with_config(config);

        // Open the circuit
        cb.record_failure().await;
        assert!(cb.is_open().await);

        // Wait for reset timeout
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Should transition to half-open
        assert!(cb.can_execute().await.is_ok());
        assert!(cb.is_half_open().await);
    }

    #[tokio::test]
    async fn test_circuit_breaker_success_recovery() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            reset_timeout: Duration::from_millis(10),
            half_open_max_operations: 2,
            count_timeouts: true,
        };
        let cb = CircuitBreaker::with_config(config);

        // Open the circuit
        cb.record_failure().await;
        assert!(cb.is_open().await);

        // Wait for reset timeout
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Success in half-open should close the circuit
        cb.record_success().await;
        assert!(cb.is_closed().await);
    }

    #[tokio::test]
    async fn test_circuit_breaker_execute() {
        let cb = CircuitBreaker::new();

        // Should execute successfully in closed state
        let result = cb
            .execute(|| async { Ok::<&str, ResilienceError>("success") })
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");

        // Should execute and record failure
        let result = cb
            .execute(|| async {
                Err::<&str, ResilienceError>(ResilienceError::timeout(Duration::from_secs(1)))
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_circuit_breaker_reset() {
        let cb = CircuitBreaker::new();

        // Open the circuit
        cb.record_failure().await;
        cb.record_failure().await;
        cb.record_failure().await;
        cb.record_failure().await;
        cb.record_failure().await;
        assert!(cb.is_open().await);

        // Reset should close it
        cb.reset().await;
        assert!(cb.is_closed().await);
    }
}
