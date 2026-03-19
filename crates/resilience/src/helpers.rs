//! Helper utilities and macros for resilience patterns
//!
//! This module provides convenience macros and utilities to reduce boilerplate
//! when working with resilience operations.

/// Log the result of a resilience operation with structured logging
///
/// # Examples
///
/// ```rust,no_run
/// use nebula_resilience::{log_result, ResilienceError};
/// use tracing::{info, warn};
///
/// async fn example() -> Result<i32, ResilienceError> {
///     let result: Result<i32, ResilienceError> = Ok(42);
///     log_result!(result, "api_call", "Calling external API");
///     result
/// }
/// ```
#[macro_export]
macro_rules! log_result {
    ($result:expr, $operation:expr, $description:expr) => {
        match &$result {
            Ok(_value) => {
                tracing::info!(
                    operation = $operation,
                    description = $description,
                    "Operation succeeded"
                );
            }
            Err(err) => {
                tracing::warn!(
                    operation = $operation,
                    description = $description,
                    error = %err,
                    "Operation failed"
                );
            }
        }
    };

    ($result:expr, $operation:expr, $description:expr, value = $value_expr:expr) => {
        match &$result {
            Ok(value) => {
                tracing::info!(
                    operation = $operation,
                    description = $description,
                    result = %$value_expr(value),
                    "Operation succeeded"
                );
            }
            Err(err) => {
                tracing::warn!(
                    operation = $operation,
                    description = $description,
                    error = %err,
                    "Operation failed"
                );
            }
        }
    };
}

/// Execute an operation with automatic logging
///
/// This combines execution and logging into a single expression.
///
/// # Examples
///
/// ```rust,no_run
/// use nebula_resilience::{execute_logged, ResilienceError};
/// use tracing::{info, warn};
///
/// async fn example() -> Result<String, ResilienceError> {
///     execute_logged!(
///         "database_query",
///         "Fetching user data",
///         async {
///             // Your operation here
///             Ok::<_, ResilienceError>("user_data".to_string())
///         }
///     )
/// }
/// ```
#[macro_export]
macro_rules! execute_logged {
    ($operation:expr, $description:expr, $future:expr) => {{
        tracing::info!(operation = $operation, description = $description, "Starting operation");
        let start = std::time::Instant::now();
        let result = $future.await;
        let elapsed = start.elapsed();

        match &result {
            Ok(_) => {
                tracing::info!(
                    operation = $operation,
                    description = $description,
                    elapsed = ?elapsed,
                    "Operation succeeded"
                );
            }
            Err(err) => {
                tracing::warn!(
                    operation = $operation,
                    description = $description,
                    elapsed = ?elapsed,
                    error = %err,
                    "Operation failed"
                );
            }
        }

        result
    }};
}

/// Create a resilience policy with a fluent builder syntax
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_resilience::policy;
/// use std::time::Duration;
///
/// let my_policy = policy! {
///     name: "api-calls",
///     timeout: Duration::from_secs(5),
///     retry: exponential(3, Duration::from_millis(100)),
///     circuit_breaker: default,
/// };
/// ```
#[macro_export]
macro_rules! policy {
    (
        name: $name:expr,
        timeout: $timeout:expr,
        retry: exponential($max_attempts:expr, $base_delay:expr),
        circuit_breaker: default
        $(,)?
    ) => {
        $crate::PolicyBuilder::new()
            .with_timeout($timeout)
            .with_retry_exponential($max_attempts, $base_delay)
            .with_circuit_breaker($crate::CircuitBreakerConfig::default())
            .build()
            .with_name($name)
    };

    (
        name: $name:expr,
        timeout: $timeout:expr,
        retry: exponential($max_attempts:expr, $base_delay:expr),
        circuit_breaker: $circuit_breaker:expr
        $(,)?
    ) => {
        $crate::PolicyBuilder::new()
            .with_timeout($timeout)
            .with_retry_exponential($max_attempts, $base_delay)
            .with_circuit_breaker($circuit_breaker)
            .build()
            .with_name($name)
    };

    (
        name: $name:expr,
        timeout: $timeout:expr,
        retry: fixed($max_attempts:expr, $delay:expr)
        $(,)?
    ) => {
        $crate::PolicyBuilder::new()
            .with_timeout($timeout)
            .with_retry_fixed($max_attempts, $delay)
            .build()
            .with_name($name)
    };

    (
        name: $name:expr,
        timeout: $timeout:expr,
        retry: fixed($max_attempts:expr, $delay:expr),
        circuit_breaker: $circuit_breaker:expr
        $(,)?
    ) => {
        $crate::PolicyBuilder::new()
            .with_timeout($timeout)
            .with_retry_fixed($max_attempts, $delay)
            .with_circuit_breaker($circuit_breaker)
            .build()
            .with_name($name)
    };
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    #[test]
    fn test_macros_compile() {
        let fixed_policy = policy! {
            name: "fixed",
            timeout: Duration::from_secs(1),
            retry: fixed(2, Duration::from_millis(50)),
        };
        assert_eq!(fixed_policy.metadata.name, "fixed");
        assert!(fixed_policy.retry.is_some());

        let exp_policy = policy! {
            name: "exp-cb",
            timeout: Duration::from_secs(2),
            retry: exponential(3, Duration::from_millis(100)),
            circuit_breaker: default,
        };
        assert_eq!(exp_policy.metadata.name, "exp-cb");
        assert!(exp_policy.circuit_breaker.is_some());
    }
}
