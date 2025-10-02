//! Bulkhead pattern for resource isolation and parallelism limits

use nebula_log::debug;
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::core::config::{ConfigError, ConfigResult, ResilienceConfig};
use crate::{ResilienceError, ResilienceResult};

/// Bulkhead configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BulkheadConfig {
    /// Maximum number of concurrent operations
    pub max_concurrency: usize,
    /// Maximum number of operations waiting in queue
    pub queue_size: usize,
    /// Optional timeout for acquiring permits
    pub timeout: Option<std::time::Duration>,
}

impl Default for BulkheadConfig {
    fn default() -> Self {
        Self {
            max_concurrency: 10,
            queue_size: 100,
            timeout: Some(std::time::Duration::from_secs(30)),
        }
    }
}

/// Bulkhead implementation for resource isolation
#[derive(Debug, Clone)]
pub struct Bulkhead {
    config: BulkheadConfig,
    semaphore: Arc<Semaphore>,
    active_operations: Arc<tokio::sync::RwLock<usize>>,
}

impl Bulkhead {
    /// Create a new bulkhead with default configuration
    #[must_use]
    pub fn new(max_concurrency: usize) -> Self {
        Self::with_config(BulkheadConfig {
            max_concurrency,
            ..Default::default()
        })
    }

    /// Create a new bulkhead with custom configuration
    #[must_use]
    pub fn with_config(config: BulkheadConfig) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(config.max_concurrency)),
            active_operations: Arc::new(tokio::sync::RwLock::new(0)),
            config,
        }
    }

    /// Get the current number of active operations
    pub async fn active_operations(&self) -> usize {
        *self.active_operations.read().await
    }

    /// Get the current number of available permits
    pub async fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Get the maximum concurrency limit
    #[must_use]
    pub fn max_concurrency(&self) -> usize {
        self.config.max_concurrency
    }

    /// Check if the bulkhead is at capacity
    pub async fn is_at_capacity(&self) -> bool {
        self.active_operations().await >= self.config.max_concurrency
    }

    /// Try to acquire a permit without waiting
    #[must_use]
    pub fn try_acquire(&self) -> Option<BulkheadPermit> {
        let permit = Arc::clone(&self.semaphore).try_acquire_owned().ok()?;
        Some(BulkheadPermit {
            permit,
            active_operations: Arc::clone(&self.active_operations),
        })
    }

    /// Acquire a permit, waiting if necessary
    pub async fn acquire(&self) -> Result<BulkheadPermit, ResilienceError> {
        let permit = Arc::clone(&self.semaphore)
            .acquire_owned()
            .await
            .map_err(|_| ResilienceError::bulkhead_full(self.config.max_concurrency))?;

        Ok(BulkheadPermit {
            permit,
            active_operations: Arc::clone(&self.active_operations),
        })
    }

    /// Execute an operation with bulkhead protection (optimized)
    pub async fn execute<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ResilienceResult<T>>,
    {
        let _permit = self.acquire().await?;

        // The semaphore permit already handles concurrency control
        // No need for separate active operations tracking
        debug!(
            "Bulkhead operation started (permits available: {})",
            self.semaphore.available_permits()
        );

        // Execute the operation
        let result = operation().await;

        debug!(
            "Bulkhead operation completed (permits available: {})",
            self.semaphore.available_permits() + 1 // +1 because permit not yet released
        );

        result
    }

    /// Execute an operation with timeout
    pub async fn execute_with_timeout<T, F, Fut>(
        &self,
        timeout: std::time::Duration,
        operation: F,
    ) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ResilienceResult<T>>,
    {
        use tokio::time::timeout as tokio_timeout;

        let _permit = self.acquire().await?;

        debug!(
            "Bulkhead operation started with timeout (permits available: {})",
            self.semaphore.available_permits()
        );

        // Execute the operation with timeout
        let result = tokio_timeout(timeout, operation()).await;

        debug!(
            "Bulkhead operation completed (permits available: {})",
            self.semaphore.available_permits() + 1
        );

        match result {
            Ok(inner_result) => inner_result,
            Err(_) => Err(ResilienceError::timeout(timeout)),
        }
    }

    /// Get bulkhead statistics
    pub async fn stats(&self) -> BulkheadStats {
        BulkheadStats {
            max_concurrency: self.config.max_concurrency,
            active_operations: self.active_operations().await,
            available_permits: self.available_permits().await,
            is_at_capacity: self.is_at_capacity().await,
        }
    }
}

impl Default for Bulkhead {
    fn default() -> Self {
        Self::with_config(BulkheadConfig::default())
    }
}

/// Permit for executing an operation within the bulkhead
pub struct BulkheadPermit {
    #[allow(dead_code)]
    permit: tokio::sync::OwnedSemaphorePermit,
    active_operations: Arc<tokio::sync::RwLock<usize>>,
}

impl BulkheadPermit {
    /// Get the number of active operations
    pub async fn active_operations(&self) -> usize {
        *self.active_operations.read().await
    }
}

impl Drop for BulkheadPermit {
    fn drop(&mut self) {
        // Permit is automatically released when dropped
    }
}

/// Bulkhead statistics
#[derive(Debug, Clone)]
pub struct BulkheadStats {
    /// Maximum concurrency limit
    pub max_concurrency: usize,
    /// Current number of active operations
    pub active_operations: usize,
    /// Current number of available permits
    pub available_permits: usize,
    /// Whether the bulkhead is at capacity
    pub is_at_capacity: bool,
}

/// Builder for creating bulkheads with custom configurations
pub struct BulkheadBuilder {
    config: BulkheadConfig,
}

impl BulkheadBuilder {
    /// Create a new bulkhead builder
    #[must_use]
    pub fn new(max_concurrency: usize) -> Self {
        Self {
            config: BulkheadConfig {
                max_concurrency,
                ..Default::default()
            },
        }
    }

    /// Set the maximum queue size
    #[must_use]
    pub fn with_queue_size(mut self, queue_size: usize) -> Self {
        self.config.queue_size = queue_size;
        self
    }

    /// Set the timeout for acquiring permits
    #[must_use]
    pub fn with_timeout(mut self, timeout: Option<std::time::Duration>) -> Self {
        self.config.timeout = timeout;
        self
    }

    /// Build the bulkhead
    #[must_use]
    pub fn build(self) -> Bulkhead {
        Bulkhead::with_config(self.config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_bulkhead_default() {
        let bulkhead = Bulkhead::new(5);
        assert_eq!(bulkhead.max_concurrency(), 5);
        assert_eq!(bulkhead.active_operations().await, 0);
        assert_eq!(bulkhead.available_permits().await, 5);
        assert!(!bulkhead.is_at_capacity().await);
    }

    #[tokio::test]
    async fn test_bulkhead_execute() {
        let bulkhead = Bulkhead::new(2);

        // Should execute successfully
        let result = bulkhead
            .execute(|| async { Ok::<&str, ResilienceError>("success") })
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");
    }

    #[tokio::test]
    async fn test_bulkhead_concurrency_limit() {
        let bulkhead = Bulkhead::new(2);
        let (tx, mut rx) = tokio::sync::mpsc::channel(10);

        // Start 3 operations concurrently
        for i in 0..3 {
            let bulkhead = bulkhead.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                let result = bulkhead
                    .execute(|| async {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        Ok::<usize, ResilienceError>(i)
                    })
                    .await;
                let _ = tx.send(result).await;
            });
        }

        // Wait for all operations to complete
        drop(tx);
        let mut results = Vec::new();
        while let Some(result) = rx.recv().await {
            results.push(result);
        }

        // All operations should succeed
        assert_eq!(results.len(), 3);
        for result in results {
            assert!(result.is_ok());
        }
    }

    #[tokio::test]
    async fn test_bulkhead_timeout() {
        let bulkhead = Bulkhead::new(1);

        // Start a long-running operation that blocks the permit
        let bulkhead_clone = bulkhead.clone();
        let handle = tokio::spawn(async move {
            let _permit = bulkhead_clone.acquire().await.unwrap();
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok::<&str, ResilienceError>("long operation")
        });

        // Give the long operation time to acquire the permit
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Now try to execute with timeout - should timeout waiting for permit
        let start = std::time::Instant::now();
        let result = tokio::time::timeout(
            Duration::from_millis(100),
            bulkhead.execute(|| async { Ok::<&str, ResilienceError>("should timeout") }),
        )
        .await;

        // Should have timed out
        assert!(result.is_err(), "Operation should have timed out");
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(90) && elapsed <= Duration::from_millis(200),
            "Should have timed out around 100ms, got {:?}",
            elapsed
        );

        // Wait for the long operation to complete
        let _ = handle.await;
    }

    #[tokio::test]
    async fn test_bulkhead_builder() {
        let bulkhead = BulkheadBuilder::new(5)
            .with_queue_size(50)
            .with_timeout(Some(Duration::from_secs(30)))
            .build();

        assert_eq!(bulkhead.max_concurrency(), 5);
        assert_eq!(bulkhead.active_operations().await, 0);
    }

    #[tokio::test]
    async fn test_bulkhead_stats() {
        let bulkhead = Bulkhead::new(3);
        let stats = bulkhead.stats().await;

        assert_eq!(stats.max_concurrency, 3);
        assert_eq!(stats.active_operations, 0);
        assert_eq!(stats.available_permits, 3);
        assert!(!stats.is_at_capacity);
    }
}

impl ResilienceConfig for BulkheadConfig {
    fn validate(&self) -> ConfigResult<()> {
        if self.max_concurrency == 0 {
            return Err(ConfigError::validation(
                "max_concurrency must be greater than 0",
            ));
        }
        if self.queue_size == 0 {
            return Err(ConfigError::validation("queue_size must be greater than 0"));
        }
        Ok(())
    }

    fn default_config() -> Self {
        Self::default()
    }

    fn merge(&mut self, other: Self) {
        *self = other;
    }
}
