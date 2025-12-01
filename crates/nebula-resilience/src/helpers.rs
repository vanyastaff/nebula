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
            Ok(value) => {
                info!(
                    operation = $operation,
                    description = $description,
                    "✅ Operation succeeded"
                );
            }
            Err(err) => {
                warn!(
                    operation = $operation,
                    description = $description,
                    error = %err,
                    "❌ Operation failed"
                );
            }
        }
    };

    ($result:expr, $operation:expr, $description:expr, value = $value_expr:expr) => {
        match &$result {
            Ok(value) => {
                info!(
                    operation = $operation,
                    description = $description,
                    result = %$value_expr(value),
                    "✅ Operation succeeded"
                );
            }
            Err(err) => {
                warn!(
                    operation = $operation,
                    description = $description,
                    error = %err,
                    "❌ Operation failed"
                );
            }
        }
    };
}

/// Print the result of an operation with emoji indicators
///
/// # Examples
///
/// ```rust,no_run
/// use nebula_resilience::{print_result, ResilienceError};
///
/// async fn example() {
///     let result: Result<i32, ResilienceError> = Ok(42);
///     print_result!(result, "Operation completed");
/// }
/// ```
#[macro_export]
macro_rules! print_result {
    ($result:expr, $fmt:expr $(, $arg:expr)*) => {
        match &$result {
            Ok(value) => {
                println!(concat!("✅ ", $fmt) $(, $arg)*);
            }
            Err(err) => {
                println!(concat!("❌ ", $fmt, " - Error: {}") $(, $arg)*, err);
            }
        }
    };

    (short $result:expr) => {
        match &$result {
            Ok(_) => print!("✅"),
            Err(_) => print!("❌"),
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
        info!(operation = $operation, description = $description, "▶️  Starting operation");
        let start = std::time::Instant::now();
        let result = $future.await;
        let elapsed = start.elapsed();

        match &result {
            Ok(_) => {
                info!(
                    operation = $operation,
                    description = $description,
                    elapsed = ?elapsed,
                    "✅ Operation succeeded"
                );
            }
            Err(err) => {
                warn!(
                    operation = $operation,
                    description = $description,
                    elapsed = ?elapsed,
                    error = %err,
                    "❌ Operation failed"
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
/// use nebula_resilience::{policy, ResiliencePolicy};
/// use std::time::Duration;
///
/// let my_policy = policy! {
///     name: "api-calls",
///     timeout: Duration::from_secs(5),
///     retry: exponential(3, Duration::from_millis(100)),
///     circuit_breaker: {
///         failure_threshold: 5,
///         reset_timeout: Duration::from_secs(30),
///     }
/// };
/// ```
#[macro_export]
macro_rules! policy {
    (
        name: $name:expr,
        timeout: $timeout:expr,
        retry: exponential($max_attempts:expr, $base_delay:expr),
        circuit_breaker: {
            failure_threshold: $threshold:expr,
            reset_timeout: $reset:expr $(,)?
        }
        $(,)?
    ) => {
        $crate::ResiliencePolicy::named($name)
            .with_timeout($timeout)
            .with_retry($crate::RetryStrategy::exponential(
                $max_attempts,
                $base_delay,
            ))
            .with_circuit_breaker($crate::CircuitBreakerConfig {
                failure_threshold: $threshold,
                reset_timeout: $reset,
                half_open_max_operations: 2,
                count_timeouts: true,
            })
    };

    (
        name: $name:expr,
        timeout: $timeout:expr,
        retry: fixed($max_attempts:expr, $delay:expr)
        $(,)?
    ) => {
        $crate::ResiliencePolicy::named($name)
            .with_timeout($timeout)
            .with_retry($crate::RetryStrategy::fixed($max_attempts, $delay))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_macros_compile() {
        // Just verify macros compile correctly
        let result: Result<i32, String> = Ok(42);
        print_result!(short result);
    }
}
