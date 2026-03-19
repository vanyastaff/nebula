//! Helper utilities and macros for resilience patterns.

/// Log the result of a resilience operation with structured logging.
///
/// # Examples
///
/// ```rust,no_run
/// use nebula_resilience::log_result;
///
/// async fn example() -> Result<i32, Box<dyn std::error::Error>> {
///     let result: Result<i32, Box<dyn std::error::Error>> = Ok(42);
///     log_result!(result, "api_call", "Calling external API");
///     Ok(42)
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

/// Execute an operation with automatic logging.
///
/// Combines execution and logging into a single expression.
///
/// # Examples
///
/// ```rust,no_run
/// use nebula_resilience::execute_logged;
///
/// async fn example() -> Result<String, Box<dyn std::error::Error>> {
///     execute_logged!(
///         "database_query",
///         "Fetching user data",
///         async {
///             Ok::<_, Box<dyn std::error::Error>>("user_data".to_string())
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
