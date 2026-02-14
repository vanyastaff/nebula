//! Retry Logic with Exponential Backoff
//!
//! Provides retry functionality for rotation operations.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;

use super::error::RotationError;

/// Retry policy configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationRetryPolicy {
    /// Maximum number of retry attempts
    pub max_attempts: u32,

    /// Initial backoff duration
    pub initial_backoff: Duration,

    /// Backoff multiplier (typically 2.0 for exponential)
    pub backoff_multiplier: f32,

    /// Maximum backoff duration
    pub max_backoff: Duration,
}

impl Default for RotationRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            initial_backoff: Duration::from_millis(100),
            backoff_multiplier: 2.0,
            max_backoff: Duration::from_secs(32),
        }
    }
}

impl RotationRetryPolicy {
    /// Create a new retry policy with custom parameters
    pub fn new(
        max_attempts: u32,
        initial_backoff: Duration,
        backoff_multiplier: f32,
        max_backoff: Duration,
    ) -> Self {
        Self {
            max_attempts,
            initial_backoff,
            backoff_multiplier,
            max_backoff,
        }
    }

    /// Calculate backoff duration for given attempt number
    ///
    /// Applies exponential backoff with ±10% jitter to prevent thundering herd.
    pub fn backoff_duration(&self, attempt: u32) -> Duration {
        use rand::Rng;

        let base_ms = self.initial_backoff.as_millis() as f32;
        let multiplier = self.backoff_multiplier.powi(attempt as i32);
        let backoff_ms = base_ms * multiplier;

        // Apply ±10% jitter to prevent thundering herd
        let jitter = rand::rng().random_range(0.9..=1.1);
        let jittered_ms = (backoff_ms * jitter) as u64;

        Duration::from_millis(jittered_ms).min(self.max_backoff)
    }
}

/// Retry an async operation with exponential backoff
///
/// # Example
///
/// ```rust,ignore
/// let policy = RotationRetryPolicy::default();
/// let result = retry_with_backoff(&policy, "database_connection", || async {
///     database.connect().await
/// }).await?;
/// ```
pub async fn retry_with_backoff<F, Fut, T, E>(
    policy: &RotationRetryPolicy,
    operation_name: &str,
    mut f: F,
) -> Result<T, RotationError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display + std::fmt::Debug,
{
    let mut last_error: Option<String> = None;

    for attempt in 0..policy.max_attempts {
        match f().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                let error_str = e.to_string();
                last_error = Some(error_str.clone());

                tracing::warn!(
                    operation = operation_name,
                    attempt = attempt + 1,
                    max_attempts = policy.max_attempts,
                    error = %error_str,
                    "Retry attempt failed"
                );

                // Don't sleep after the last attempt
                if attempt < policy.max_attempts - 1 {
                    let backoff = policy.backoff_duration(attempt);
                    tracing::debug!(
                        operation = operation_name,
                        backoff_ms = backoff.as_millis(),
                        "Backing off before next retry"
                    );
                    sleep(backoff).await;
                }
            }
        }
    }

    let error_context = last_error
        .map(|e| format!(" (last error: {})", e))
        .unwrap_or_default();

    Err(RotationError::MaxRetriesExceeded {
        operation: format!("{}{}", operation_name, error_context),
        max_attempts: policy.max_attempts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_calculation() {
        let policy = RotationRetryPolicy::default();

        // First attempt: 100ms * 2^0 = 100ms ± 10% jitter (90-110ms)
        let backoff_0 = policy.backoff_duration(0);
        assert!(
            backoff_0 >= Duration::from_millis(90) && backoff_0 <= Duration::from_millis(110),
            "Expected 90-110ms, got {:?}",
            backoff_0
        );

        // Second attempt: 100ms * 2^1 = 200ms ± 10% jitter (180-220ms)
        let backoff_1 = policy.backoff_duration(1);
        assert!(
            backoff_1 >= Duration::from_millis(180) && backoff_1 <= Duration::from_millis(220),
            "Expected 180-220ms, got {:?}",
            backoff_1
        );

        // Third attempt: 100ms * 2^2 = 400ms ± 10% jitter (360-440ms)
        let backoff_2 = policy.backoff_duration(2);
        assert!(
            backoff_2 >= Duration::from_millis(360) && backoff_2 <= Duration::from_millis(440),
            "Expected 360-440ms, got {:?}",
            backoff_2
        );

        // Fourth attempt: 100ms * 2^3 = 800ms ± 10% jitter (720-880ms)
        let backoff_3 = policy.backoff_duration(3);
        assert!(
            backoff_3 >= Duration::from_millis(720) && backoff_3 <= Duration::from_millis(880),
            "Expected 720-880ms, got {:?}",
            backoff_3
        );

        // Large attempt should cap at max_backoff (32s)
        let backoff_large = policy.backoff_duration(10);
        assert_eq!(
            backoff_large,
            Duration::from_secs(32),
            "Large attempt should cap at max_backoff"
        );
    }

    #[tokio::test]
    async fn test_retry_success_on_first_attempt() {
        let policy = RotationRetryPolicy {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(10),
            backoff_multiplier: 2.0,
            max_backoff: Duration::from_secs(1),
        };

        use std::sync::atomic::{AtomicU32, Ordering};
        let counter = AtomicU32::new(0);

        let result = retry_with_backoff(&policy, "test_op", || async {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok::<i32, String>(42)
        })
        .await
        .unwrap();

        assert_eq!(result, 42);
        assert_eq!(counter.load(Ordering::SeqCst), 1); // Only tried once
    }

    #[tokio::test]
    async fn test_retry_success_on_second_attempt() {
        let policy = RotationRetryPolicy {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(10),
            backoff_multiplier: 2.0,
            max_backoff: Duration::from_secs(1),
        };

        use std::sync::atomic::{AtomicU32, Ordering};
        let counter = AtomicU32::new(0);

        let result = retry_with_backoff(&policy, "test_op", || async {
            let count = counter.fetch_add(1, Ordering::SeqCst) + 1;
            if count == 1 {
                Err("first attempt fails".to_string())
            } else {
                Ok::<i32, String>(42)
            }
        })
        .await
        .unwrap();

        assert_eq!(result, 42);
        assert_eq!(counter.load(Ordering::SeqCst), 2); // Tried twice
    }

    #[tokio::test]
    async fn test_retry_max_attempts_exceeded() {
        let policy = RotationRetryPolicy {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(10),
            backoff_multiplier: 2.0,
            max_backoff: Duration::from_secs(1),
        };

        use std::sync::atomic::{AtomicU32, Ordering};
        let counter = AtomicU32::new(0);

        let result = retry_with_backoff(&policy, "test_op", || async {
            counter.fetch_add(1, Ordering::SeqCst);
            Err::<i32, String>("always fails".to_string())
        })
        .await;

        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::SeqCst), 3); // Tried max_attempts times
        match result.unwrap_err() {
            RotationError::MaxRetriesExceeded { max_attempts, .. } => {
                assert_eq!(max_attempts, 3);
            }
            _ => panic!("Expected MaxRetriesExceeded error"),
        }
    }
}
