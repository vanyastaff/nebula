//! Bulkhead pattern for resource isolation and parallelism limits

use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{debug, warn};

use crate::error::{ResilienceError, ResilienceResult};

/// Bulkhead configuration
#[derive(Debug, Clone)]
pub struct BulkheadConfig {
    /// Maximum number of concurrent operations
    pub max_concurrency: usize,
    /// Maximum number of operations waiting in queue
    pub max_queue_size: usize,
    /// Whether to reject operations when queue is full
    pub reject_when_full: bool,
}

impl Default for BulkheadConfig {
    fn default() -> Self {
        Self {
            max_concurrency: 10,
            max_queue_size: 100,
            reject_when_full: true,
        }
    }
}

/// Bulkhead implementation for resource isolation
#[derive(Clone)]
pub struct Bulkhead {
    config: BulkheadConfig,
    semaphore: Arc<Semaphore>,
    active_operations: Arc<tokio::sync::RwLock<usize>>,
}

impl Bulkhead {
    /// Create a new bulkhead with default configuration
    pub fn new(max_concurrency: usize) -> Self {
        Self::with_config(BulkheadConfig {
            max_concurrency,
            ..Default::default()
        })
    }

    /// Create a new bulkhead with custom configuration
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
    pub fn max_concurrency(&self) -> usize {
        self.config.max_concurrency
    }

    /// Check if the bulkhead is at capacity
    pub async fn is_at_capacity(&self) -> bool {
        self.active_operations().await >= self.config.max_concurrency
    }

    /// Try to acquire a permit without waiting
    pub fn try_acquire(&self) -> Option<BulkheadPermit> {
        let permit = self.semaphore.try_acquire().ok()?;
        Some(BulkheadPermit {
            permit,
            active_operations: Arc::clone(&self.active_operations),
        })
    }

    /// Acquire a permit, waiting if necessary
    pub async fn acquire(&self) -> Result<BulkheadPermit, ResilienceError> {
        let permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| ResilienceError::bulkhead_full(self.config.max_concurrency))?;

        Ok(BulkheadPermit {
            permit,
            active_operations: Arc::clone(&self.active_operations),
        })
    }

    /// Execute an operation with bulkhead protection
    pub async fn execute<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = ResilienceResult<T>>,
    {
        let _permit = self.acquire().await?;
        
        // Increment active operations counter
        {
            let mut active = self.active_operations.write().await;
            *active += 1;
            debug!(
                "Bulkhead operation started (active: {}/{})",
                *active, self.config.max_concurrency
            );
        }

        // Execute the operation
        let result = operation().await;

        // Decrement active operations counter
        {
            let mut active = self.active_operations.write().await;
            *active -= 1;
            debug!(
                "Bulkhead operation completed (active: {}/{})",
                *active, self.config.max_concurrency
            );
        }

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
        Fut: std::future::Future<Output = ResilienceResult<T>>,
    {
        use tokio::time::timeout as tokio_timeout;

        let _permit = self.acquire().await?;
        
        // Increment active operations counter
        {
            let mut active = self.active_operations.write().await;
            *active += 1;
            debug!(
                "Bulkhead operation started with timeout (active: {}/{})",
                *active, self.config.max_concurrency
            );
        }

        // Execute the operation with timeout
        let result = tokio_timeout(timeout, operation()).await;

        // Decrement active operations counter
        {
            let mut active = self.active_operations.write().await;
            *active -= 1;
            debug!(
                "Bulkhead operation completed (active: {}/{})",
                *active, self.config.max_concurrency
            );
        }

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
pub struct BulkheadPermit<'a> {
    permit: tokio::sync::SemaphorePermit<'a>,
    active_operations: Arc<tokio::sync::RwLock<usize>>,
}

impl<'a> BulkheadPermit<'a> {
    /// Get the number of active operations
    pub async fn active_operations(&self) -> usize {
        *self.active_operations.read().await
    }
}

impl<'a> Drop for BulkheadPermit<'a> {
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
    pub fn new(max_concurrency: usize) -> Self {
        Self {
            config: BulkheadConfig {
                max_concurrency,
                ..Default::default()
            },
        }
    }

    /// Set the maximum queue size
    pub fn with_max_queue_size(mut self, max_queue_size: usize) -> Self {
        self.config.max_queue_size = max_queue_size;
        self
    }

    /// Set whether to reject operations when queue is full
    pub fn with_reject_when_full(mut self, reject_when_full: bool) -> Self {
        self.config.reject_when_full = reject_when_full;
        self
    }

    /// Build the bulkhead
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

        // Start a long-running operation
        let bulkhead_clone = bulkhead.clone();
        let handle = tokio::spawn(async move {
            bulkhead_clone
                .execute(|| async {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    Ok::<&str, ResilienceError>("long operation")
                })
                .await
        });

        // Try to execute with timeout - should timeout
        let result = bulkhead
            .execute_with_timeout(
                Duration::from_millis(100),
                || async { Ok::<&str, ResilienceError>("should timeout") },
            )
            .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ResilienceError::Timeout { .. } => {}
            _ => panic!("Expected timeout error"),
        }

        // Wait for the long operation to complete
        let _ = handle.await;
    }

    #[tokio::test]
    async fn test_bulkhead_builder() {
        let bulkhead = BulkheadBuilder::new(5)
            .with_max_queue_size(50)
            .with_reject_when_full(false)
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
