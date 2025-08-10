use async_trait::async_trait;
use std::time::Duration;
use thiserror::Error;

/// Error type for lock operations
#[derive(Error, Debug, Clone)]
pub enum LockError {
    #[error("lock is contended")]
    Contended,

    #[error("lock was lost")]
    Lost,

    #[error("backend error: {0}")]
    Backend(String),
}

/// Lock guard that releases the lock when dropped
#[async_trait]
pub trait LockGuard: Send {
    /// Release the lock explicitly
    async fn release(self) -> Result<(), LockError>;
}

/// Trait for distributed locking
#[async_trait]
pub trait DistributedLock: Send + Sync {
    /// Type of guard returned when lock is acquired
    type Guard: LockGuard;

    /// Try to acquire lock with timeout
    async fn acquire(&self, key: &str, ttl: Duration) -> Result<Self::Guard, LockError>;

    /// Try to acquire lock without blocking
    async fn try_acquire(&self, key: &str, ttl: Duration) -> Result<Option<Self::Guard>, LockError>;
}